//! Restore ownership journal on a real private filesystem root: tampered or
//! foreign records, illegal phase changes and foreign ownership markers all fail
//! closed, and nothing the journal does not own is ever removed.
use lytronaut_service::restore_journal::{Journal, Phase};
use lytronaut_storage::{Catalog, StorageError};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-journal-test-{:x}",
            u128::from_le_bytes(b)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        Self(root)
    }
    fn journal(&self, job: &str) -> PathBuf {
        self.0.join(format!("{job}.restore-journal"))
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn prepared_journals_round_trip_and_reject_tampering() {
    let root = Root::new();
    let journal = Journal::prepare(&root.0, "job-a", None, 4096).unwrap();
    assert_eq!(
        (journal.phase(), journal.job(), journal.reserved()),
        (Phase::Prepared, "job-a", 4096)
    );
    assert!(journal.namespace().is_none());
    // The ownership marker exists only in the pending directory.
    assert!(journal.pending(&root.0).join("restore-owner").is_file());
    let debug = format!("{journal:?}");
    assert!(debug.contains("Prepared") && !debug.contains("job-a"));
    let loaded = Journal::load(&root.0, "job-a").unwrap().unwrap();
    assert_eq!(loaded.phase(), Phase::Prepared);
    // A second intent for the same job never replaces the first.
    assert!(Journal::prepare(&root.0, "job-a", None, 1).is_err());

    assert!(Journal::load(&root.0, "absent-job").unwrap().is_none());
    assert_eq!(
        Journal::load(&root.0, "../escape").unwrap_err(),
        StorageError::Invalid
    );
    // A record copied under another job name is foreign.
    fs::copy(root.journal("job-a"), root.journal("job-b")).unwrap();
    fs::set_permissions(root.journal("job-b"), fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        Journal::load(&root.0, "job-b").unwrap_err(),
        StorageError::Conflict
    );
    // Group/world-readable, malformed, unknown-field and oversized records.
    fs::set_permissions(root.journal("job-b"), fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        Journal::load(&root.0, "job-b").unwrap_err(),
        StorageError::Invalid
    );
    for bytes in [
        b"not json".to_vec(),
        br#"{"version":1,"job":"job-b","nonce":"00","namespace":null,"reserved":0,"phase":"Prepared","extra":1}"#.to_vec(),
        vec![b' '; 16 * 1024 + 1],
    ] {
        fs::write(root.journal("job-b"), bytes).unwrap();
        fs::set_permissions(root.journal("job-b"), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            Journal::load(&root.0, "job-b").unwrap_err(),
            StorageError::Invalid
        );
    }
    // An existing final result is preserved: no new intent is written over it.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(root.0.join("job-c.restored"))
        .unwrap();
    assert_eq!(
        Journal::prepare(&root.0, "job-c", None, 0).unwrap_err(),
        StorageError::Conflict
    );
    assert!(!root.journal("job-c").exists());
    assert_eq!(
        Journal::prepare(&root.0, "bad/job", None, 0).unwrap_err(),
        StorageError::Invalid
    );
}

#[test]
fn phases_only_move_forward_and_publication_needs_the_exact_marker() {
    let root = Root::new();
    let mut journal = Journal::prepare(&root.0, "job-p", None, 0).unwrap();
    assert_eq!(
        journal
            .transition(&root.0, Phase::FilesPublished)
            .unwrap_err(),
        StorageError::Conflict
    );
    journal.transition(&root.0, Phase::RowsCommitted).unwrap();
    journal.transition(&root.0, Phase::RowsCommitted).unwrap();
    journal.transition(&root.0, Phase::FilesReady).unwrap();
    assert_eq!(
        journal.transition(&root.0, Phase::SchemaOwned).unwrap_err(),
        StorageError::Conflict
    );
    assert_eq!(
        Journal::load(&root.0, "job-p").unwrap().unwrap().phase(),
        Phase::FilesReady
    );
    // Unpublished: no result directory.
    assert!(journal.verify_published(&root.0).is_err());
    fs::rename(journal.pending(&root.0), journal.result(&root.0)).unwrap();
    journal.transition(&root.0, Phase::FilesPublished).unwrap();
    journal.verify_published(&root.0).unwrap();
    let marker = journal.result(&root.0).join("restore-owner");
    let genuine = fs::read(&marker).unwrap();
    // A foreign nonce, a truncated marker or a shared result directory is not ours.
    let mut foreign = genuine.clone();
    foreign[0] = if foreign[0] == b'0' { b'1' } else { b'0' };
    for (bytes, mode) in [
        (foreign, 0o700),
        (genuine[..63].to_vec(), 0o700),
        (genuine.clone(), 0o755),
    ] {
        fs::write(&marker, &bytes).unwrap();
        fs::set_permissions(journal.result(&root.0), fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            journal.verify_published(&root.0).unwrap_err(),
            StorageError::Conflict
        );
    }
    fs::write(&marker, &genuine).unwrap();
    fs::set_permissions(journal.result(&root.0), fs::Permissions::from_mode(0o700)).unwrap();
    journal.verify_published(&root.0).unwrap();
    // The journal may only be forgotten after cleanup completed.
    assert_eq!(
        journal.forget_after_quota_release(&root.0).unwrap_err(),
        StorageError::Conflict
    );
}

#[tokio::test]
async fn cleanup_removes_only_owned_directories_then_forgets() {
    let root = Root::new();
    let catalog = Catalog::open_application_database(&root.0.join("catalog.db"))
        .await
        .unwrap();
    // A result directory whose marker belongs to someone else is never deleted.
    let mut journal = Journal::prepare(&root.0, "job-x", None, 0).unwrap();
    fs::rename(journal.pending(&root.0), journal.result(&root.0)).unwrap();
    let marker = journal.result(&root.0).join("restore-owner");
    let genuine = fs::read(&marker).unwrap();
    fs::write(&marker, [b'f'; 64]).unwrap();
    assert_eq!(
        journal.cleanup(&catalog, &root.0).await.unwrap_err(),
        StorageError::Conflict
    );
    assert!(journal.result(&root.0).is_dir(), "foreign result retained");
    // A regular file in place of the owned directory is a conflict too.
    fs::remove_dir_all(journal.result(&root.0)).unwrap();
    fs::write(journal.result(&root.0), b"not ours").unwrap();
    assert_eq!(
        journal.cleanup(&catalog, &root.0).await.unwrap_err(),
        StorageError::Conflict
    );
    fs::remove_file(journal.result(&root.0)).unwrap();
    // The genuine owned directory is removed and the journal can be forgotten.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(journal.result(&root.0))
        .unwrap();
    fs::write(&marker, &genuine).unwrap();
    journal.cleanup(&catalog, &root.0).await.unwrap();
    assert_eq!(journal.phase(), Phase::CleanupComplete);
    assert!(!journal.result(&root.0).exists());
    journal.forget_after_quota_release(&root.0).unwrap();
    assert!(Journal::load(&root.0, "job-x").unwrap().is_none());
    catalog.pool().close().await;
}
