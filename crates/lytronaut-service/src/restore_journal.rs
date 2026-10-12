//! Durable ownership for SQL/filesystem restore publication and cleanup.
//! Credentials and archive-selected identifiers never enter these records.
use lytronaut_storage::{Catalog, StorageError as E, restore_namespace::RestoreNamespace};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
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
        };
        value.validate()?;
        let result = value.result(root);
        // Ownership is exclusive; never adopt an already-present result directory.
        fs::DirBuilder::new().mode(0o700).create(&result)?;
        let created = (|| {
            let mut marker = new_file(&result.join("restore-owner"))?;
            marker.write_all(value.nonce.as_bytes())?;
            marker.sync_all()?;
            File::open(&result)?.sync_all()?;
            value.persist_new(root)?;
            Ok::<_, E>(())
        })();
        if let Err(error) = created {
            // Only our just-created directory is removed; no SQL has been created.
            let _ = fs::remove_dir_all(&result);
            return Err(error);
        }
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
    pub fn result(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.restored", self.job))
    }
    fn path(&self, root: &Path) -> PathBuf {
        root.join(format!("{}.restore-journal", self.job))
    }
    fn validate(&self) -> Result<(), E> {
        if self.version != 1
            || !identity(&self.job)
            || !identity(&self.nonce)
            || self.reserved > i64::MAX as u64
        {
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
        if !identity(job) {
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
        let old = self.phase;
        self.phase = phase;
        let result = self.persist(root);
        if result.is_err() {
            self.phase = old;
        }
        result
    }
    fn persist(&self, root: &Path) -> Result<(), E> {
        // Private root plus unpredictable create_new name; rename never follows an
        // existing destination symlink. A failed rewrite leaves the previous record.
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
        let result = self.result(root);
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
                fs::remove_dir_all(result)?;
                File::open(root)?.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(E::Unavailable),
        }
        self.transition(root, Phase::CleanupComplete)
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
        fs::write(journal.result(&root).join("original"), b"owned").unwrap();
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
        let result = journal.result(&root);
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
