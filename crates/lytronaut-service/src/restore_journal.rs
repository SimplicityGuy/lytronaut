//! Durable ownership for SQL/filesystem restore publication and cleanup.
//! Credentials and archive-selected identifiers never enter these records.
use lytronaut_storage::{Catalog, StorageError as E, restore_namespace::RestoreNamespace};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const MAX_JOURNAL: u64 = 16 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Prepared,
    SchemaOwned,
    RowsCommitted,
    FilesReady,
    FilesPublished,
    JobSucceeded,
    CleanupPending,
    CleanupComplete,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    version: u8,
    job: String,
    nonce: String,
    namespace: Option<RestoreNamespace>,
    reserved: u64,
    phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    removing: Option<Removal>,
}
/// A verified directory's identity survives removal of its inner marker. This
/// intent is durable before renaming; filenames alone never grant ownership.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    published: bool,
    device: u64,
    inode: u64,
}
impl std::fmt::Debug for Journal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreJournal")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}
impl Journal {
    pub fn prepare(
        root: &Path,
        job: &str,
        namespace: Option<RestoreNamespace>,
        reserved: u64,
    ) -> Result<Self, E> {
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| E::Unavailable)?;
        let nonce = random.iter().map(|b| format!("{b:02x}")).collect();
        let value = Self {
            version: 1,
            job: job.into(),
            nonce,
            namespace,
            reserved,
            phase: Phase::Prepared,
            removing: None,
        };
        value.validate()?;
        let result = value.result(root);
        // Preserve an existing final result before writing any intent record.
        match fs::symlink_metadata(&result) {
            Ok(_) => return Err(E::Conflict),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(E::Unavailable),
        }
        // This durable intent precedes BOTH filesystem and SQL creation. If an
        // initial mkdir/marker write is interrupted, recovery retains this record
        // and quota until exact ownership is proved; it never guesses by job name.
        value.persist_new(root)?;
        fault("prepared")?;
        let pending = value.pending(root);
        fs::DirBuilder::new().mode(0o700).create(&pending)?;
        fault("mkdir")?;
        let mut marker = new_file(&pending.join("restore-owner"))?;
        marker.write_all(value.nonce.as_bytes())?;
        fault("marker")?;
        marker.sync_all()?;
        File::open(&pending)?.sync_all()?;
        File::open(root)?.sync_all()?;
        Ok(value)
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn job(&self) -> &str {
        &self.job
    }
    pub fn reserved(&self) -> u64 {
        self.reserved
    }
    pub fn namespace(&self) -> Option<&RestoreNamespace> {
        self.namespace.as_ref()
    }
    pub fn verify_published(&self, root: &Path) -> Result<(), E> {
        let path = self.result(root);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(E::Conflict);
        }
        let mut marker = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path.join("restore-owner"))?;
        let metadata = marker.metadata()?;
        if !metadata.is_file() || metadata.len() != 64 || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(E::Conflict);
        }
        let mut nonce = [0; 64];
        marker.read_exact(&mut nonce)?;
        if nonce.as_slice() != self.nonce.as_bytes() {
            return Err(E::Conflict);
        }
        Ok(())
    }
    pub fn result(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.restored", self.job))
    }
    pub fn pending(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.pending-{}", self.job, self.nonce))
    }
    fn path(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.restore-journal", self.job))
    }
    fn validate(&self) -> Result<(), E> {
        if self.version != 1
            || !job_identity(&self.job)
            || !identity(&self.nonce)
            || self.reserved > i64::MAX as u64
        {
            return Err(E::Invalid);
        }
        if self.removing.is_some() && self.phase != Phase::CleanupPending {
            return Err(E::Invalid);
        }
        if let Some(namespace) = &self.namespace {
            namespace.validate()?;
            if namespace.job() != self.job {
                return Err(E::Conflict);
            }
        }
        Ok(())
    }
    fn encoded(&self) -> Result<Vec<u8>, E> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| E::Invalid)?;
        if bytes.len() as u64 > MAX_JOURNAL {
            return Err(E::Quota);
        }
        Ok(bytes)
    }
    fn persist_new(&self, root: &Path) -> Result<(), E> {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| E::Unavailable)?;
        let temporary = root.join(format!(
            "{}.journal-{:x}",
            self.job,
            u128::from_le_bytes(random)
        ));
        let result = (|| {
            let mut output = new_file(&temporary)?;
            output.write_all(&self.encoded()?)?;
            output.sync_all()?;
            // Link publishes a complete record without replacing an existing job
            // journal. A torn initial write is never mistaken for durable intent.
            fs::hard_link(&temporary, self.path(root))?;
            File::open(root)?.sync_all()?;
            Ok::<_, E>(())
        })();
        let _ = fs::remove_file(&temporary);
        result
    }
    pub fn load(root: &Path, job: &str) -> Result<Option<Self>, E> {
        if !job_identity(job) {
            return Err(E::Invalid);
        }
        let path = root.join(format!("{job}.restore-journal"));
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(E::Unavailable),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.len() > MAX_JOURNAL
        {
            return Err(E::Invalid);
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_JOURNAL + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_JOURNAL {
            return Err(E::Quota);
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| E::Invalid)?;
        value.validate()?;
        if value.job != job {
            return Err(E::Conflict);
        }
        Ok(Some(value))
    }
    pub fn transition(&mut self, root: &Path, phase: Phase) -> Result<(), E> {
        let allowed = matches!(
            (self.phase, phase),
            (Phase::Prepared, Phase::SchemaOwned | Phase::RowsCommitted)
                | (Phase::SchemaOwned, Phase::RowsCommitted)
                | (Phase::RowsCommitted, Phase::FilesReady)
                | (Phase::FilesReady, Phase::FilesPublished)
                | (Phase::FilesPublished, Phase::JobSucceeded)
                | (Phase::CleanupPending, Phase::CleanupComplete)
        ) || phase == Phase::CleanupPending
            || self.phase == phase;
        if !allowed {
            return Err(E::Conflict);
        }
        self.phase = phase;
        let result = self.persist(root);
        if result.is_err() {
            // rename may already have succeeded before directory fsync failed.
            // Reload exact ownership rather than asserting the old phase survived.
            if let Some(observed) = Self::load(root, &self.job)? {
                if observed.nonce != self.nonce
                    || observed.namespace != self.namespace
                    || observed.reserved != self.reserved
                {
                    return Err(E::Conflict);
                }
                *self = observed;
            } else {
                return Err(E::Unavailable);
            }
        }
        result
    }
    fn persist(&self, root: &Path) -> Result<(), E> {
        // Private root plus unpredictable create_new name; rename never follows an
        // existing destination symlink. After rename, fsync failure is an ambiguous
        // durability outcome; transition reloads the actual record and fails closed.
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| E::Unavailable)?;
        let temporary = root.join(format!(
            "{}.journal-{:x}",
            self.job,
            u128::from_le_bytes(random)
        ));
        let result = (|| {
            let mut output = new_file(&temporary)?;
            output.write_all(&self.encoded()?)?;
            output.sync_all()?;
            fs::rename(&temporary, self.path(root))?;
            fault("rename")?;
            File::open(root)?.sync_all()?;
            Ok::<_, E>(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    /// Shared by restart, failed worker/final UPDATE, and late DELETE. Caller must
    /// serialize per-job reconciliation against publication and other cleanup calls.
    /// This leaves the journal present until the caller durably releases SQL quota.
    pub async fn cleanup(&mut self, catalog: &Catalog, root: &Path) -> Result<(), E> {
        self.transition(root, Phase::CleanupPending)?;
        if let Some(namespace) = &self.namespace {
            namespace.cleanup(catalog).await?;
        }
        if self.removing.is_some() {
            self.finish_removal(root)?;
        }
        for (published, result) in [(false, self.pending(root)), (true, self.result(root))] {
            match fs::symlink_metadata(&result) {
                Ok(metadata) => {
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err(E::Conflict);
                    }
                    let mut marker = OpenOptions::new()
                        .read(true)
                        .custom_flags(libc::O_NOFOLLOW)
                        .open(result.join("restore-owner"))?;
                    if !marker.metadata()?.is_file() || marker.metadata()?.len() != 64 {
                        return Err(E::Conflict);
                    }
                    let mut nonce = [0; 64];
                    marker.read_exact(&mut nonce)?;
                    if nonce.as_slice() != self.nonce.as_bytes() {
                        return Err(E::Conflict);
                    }
                    let removal = Removal {
                        published,
                        device: metadata.dev(),
                        inode: metadata.ino(),
                    };
                    // Never overwrite a pre-existing tombstone, even an empty
                    // directory: it has no durable ownership proof from us.
                    match fs::symlink_metadata(self.tombstone(root, removal)) {
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        _ => return Err(E::Conflict),
                    }
                    self.removing = Some(removal);
                    self.persist(root)?;
                    fault("cleanup-intent")?;
                    self.finish_removal(root)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(E::Unavailable),
            }
        }
        self.transition(root, Phase::CleanupComplete)
    }
    fn tombstone(&self, root: &Path, removal: Removal) -> PathBuf {
        let kind = if removal.published {
            "result"
        } else {
            "pending"
        };
        root.join(format!("{}.deleting-{}-{kind}", self.job, self.nonce))
    }
    fn finish_removal(&mut self, root: &Path) -> Result<(), E> {
        let removal = self.removing.ok_or(E::Conflict)?;
        let source = if removal.published {
            self.result(root)
        } else {
            self.pending(root)
        };
        let tombstone = self.tombstone(root, removal);
        let metadata = |path: &Path| -> Result<Option<fs::Metadata>, E> {
            match fs::symlink_metadata(path) {
                Ok(value) => Ok(Some(value)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(error.into()),
            }
        };
        let verify = |value: &fs::Metadata| -> Result<(), E> {
            if !value.is_dir()
                || value.file_type().is_symlink()
                || value.dev() != removal.device
                || value.ino() != removal.inode
            {
                return Err(E::Conflict);
            }
            Ok(())
        };
        if let Some(value) = metadata(&source)? {
            verify(&value)?;
            if metadata(&tombstone)?.is_some() {
                return Err(E::Conflict);
            }
            fs::rename(&source, &tombstone)?;
            File::open(root)?.sync_all()?;
            fault("cleanup-renamed")?;
        }
        if let Some(value) = metadata(&tombstone)? {
            verify(&value)?;
            // The durable directory identity, checked above, is independent of
            // the inner marker that a previous remove_dir_all may have removed.
            fs::remove_dir_all(&tombstone)?;
            fault("cleanup-removed")?;
        }
        File::open(root)?.sync_all()?;
        self.removing = None;
        self.persist(root)
    }
    /// Only call AFTER retained/reserved SQL accounting is durably cleared.
    pub fn forget_after_quota_release(&self, root: &Path) -> Result<(), E> {
        if self.phase != Phase::CleanupComplete {
            return Err(E::Conflict);
        }
        fs::remove_file(self.path(root))?;
        File::open(root)?.sync_all()?;
        Ok(())
    }
}
fn job_identity(value: &str) -> bool {
    lytronaut_contracts::OpaqueId::try_from(value.to_owned()).is_ok()
}
fn identity(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn new_file(path: &Path) -> Result<File, E> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}
#[cfg(test)]
thread_local! {
    static JOURNAL_FAULT: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}
/// Test-only crash cut for the next matching durability point on this thread.
#[cfg(test)]
pub(crate) fn inject_fault(point: &'static str) {
    JOURNAL_FAULT.with(|fault| fault.set(Some(point)));
}
/// Test-only crash cut; compiled to a no-op outside `cfg(test)`.
pub(crate) fn fault(point: &str) -> Result<(), E> {
    #[cfg(test)]
    if JOURNAL_FAULT.with(|fault| {
        if fault.get() == Some(point) {
            fault.set(None);
            true
        } else {
            false
        }
    }) {
        return Err(E::Interrupted);
    }
    #[cfg(not(test))]
    let _ = point;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-journal-{:x}",
            u128::from_le_bytes(random)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        root
    }
    #[test]
    fn prepared_intent_survives_initial_crash_cuts_and_preserves_preexisting_result() {
        for point in ["prepared", "mkdir", "marker"] {
            let root = root();
            let job = format!("app-{}", "a".repeat(64));
            JOURNAL_FAULT.with(|fault| fault.set(Some(point)));
            assert!(matches!(
                Journal::prepare(&root, &job, None, 4096),
                Err(E::Interrupted)
            ));
            let journal = Journal::load(&root, &job).unwrap().unwrap();
            assert_eq!(journal.phase(), Phase::Prepared);
            assert_eq!(journal.reserved(), 4096);
            assert!(!journal.result(&root).exists());
            if point == "prepared" {
                assert!(!journal.pending(&root).exists());
            }
            if point == "mkdir" {
                assert!(journal.pending(&root).exists());
                assert!(!journal.pending(&root).join("restore-owner").exists());
            }
            fs::remove_dir_all(root).unwrap();
        }
        let root = root();
        let job = format!("app-{}", "b".repeat(64));
        let result = root.join(format!("{job}.restored"));
        fs::create_dir(&result).unwrap();
        fs::write(result.join("unrelated"), b"sentinel").unwrap();
        assert!(Journal::prepare(&root, &job, None, 4096).is_err());
        assert!(Journal::load(&root, &job).unwrap().is_none());
        assert_eq!(fs::read(result.join("unrelated")).unwrap(), b"sentinel");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rename_before_directory_sync_failure_reloads_new_phase_without_false_rollback() {
        let root = root();
        let job = format!("app-{}", "c".repeat(64));
        let mut journal = Journal::prepare(&root, &job, None, 4096).unwrap();
        JOURNAL_FAULT.with(|fault| fault.set(Some("rename")));
        assert!(matches!(
            journal.transition(&root, Phase::RowsCommitted),
            Err(E::Interrupted)
        ));
        assert_eq!(journal.phase(), Phase::RowsCommitted);
        assert_eq!(
            Journal::load(&root, &job).unwrap().unwrap().phase(),
            Phase::RowsCommitted
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn cleanup_preserves_quota_record_until_explicit_release_and_unrelated_files() {
        let root = root();
        let job = "1".repeat(64);
        fs::write(root.join("unrelated"), b"sentinel").unwrap();
        let mut journal = Journal::prepare(&root, &job, None, 1024).unwrap();
        assert!(journal.transition(&root, Phase::FilesPublished).is_err());
        journal.transition(&root, Phase::RowsCommitted).unwrap();
        journal.transition(&root, Phase::FilesReady).unwrap();
        journal.transition(&root, Phase::FilesPublished).unwrap();
        fs::write(journal.pending(&root).join("original"), b"owned").unwrap();
        let catalog = Catalog::open_application_database(Path::new(":memory:"))
            .await
            .unwrap();
        journal.cleanup(&catalog, &root).await.unwrap();
        assert!(!journal.result(&root).exists());
        let persisted = Journal::load(&root, &job).unwrap().unwrap();
        assert_eq!(persisted.phase(), Phase::CleanupComplete);
        assert_eq!(persisted.reserved(), 1024);
        assert_eq!(fs::read(root.join("unrelated")).unwrap(), b"sentinel");
        // SQL accounting release is the caller's required boundary, not inferred
        // from the best-effort absence of a filesystem directory.
        journal.forget_after_quota_release(&root).unwrap();
        assert!(Journal::load(&root, &job).unwrap().is_none());
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn forged_or_replaced_result_never_gets_removed_or_releases_ownership() {
        let root = root();
        let job = "2".repeat(64);
        let mut journal = Journal::prepare(&root, &job, None, 4096).unwrap();
        assert!(Journal::prepare(&root, &job, None, 4096).is_err());
        let result = journal.pending(&root);
        fs::rename(&result, root.join("retained-owned-copy")).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&result).unwrap();
        fs::write(result.join("unrelated"), b"sentinel").unwrap();
        let catalog = Catalog::open_application_database(Path::new(":memory:"))
            .await
            .unwrap();
        assert!(journal.cleanup(&catalog, &root).await.is_err());
        assert_eq!(fs::read(result.join("unrelated")).unwrap(), b"sentinel");
        assert_eq!(
            Journal::load(&root, &job).unwrap().unwrap().phase(),
            Phase::CleanupPending
        );
        assert!(journal.forget_after_quota_release(&root).is_err());
        let path = journal.path(&root);
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(root.join("retained-owned-copy/restore-owner"), &path).unwrap();
        assert!(Journal::load(&root, &job).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
