//! Durable ownership of job-local upload/artifact/spool files.
//! A filename alone never authorizes deletion after restart.
use lytronaut_contracts::OpaqueId;
use lytronaut_storage::StorageError as E;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Owner {
    version: u8,
    job: String,
    nonce: String,
}
impl Owner {
    fn record(root: &Path, job: &str) -> PathBuf {
        root.join(format!("{job}.transfer-owner"))
    }
    pub fn directory(root: &Path, job: &str) -> PathBuf {
        root.join(format!("{job}.transfer"))
    }
    fn validate(&self, job: &str) -> Result<(), E> {
        if self.version != 1
            || self.job != job
            || OpaqueId::try_from(job.to_owned()).is_err()
            || self.nonce.len() != 64
            || !self
                .nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(E::Invalid);
        }
        Ok(())
    }
    pub fn prepare(root: &Path, job: &str) -> Result<Self, E> {
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| E::Unavailable)?;
        let owner = Self {
            version: 1,
            job: job.into(),
            nonce: random.iter().map(|b| format!("{b:02x}")).collect(),
        };
        owner.validate(job)?;
        // A pre-existing container is never adopted or modified.
        match fs::symlink_metadata(Self::directory(root, job)) {
            Ok(_) => return Err(E::Conflict),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        // Fully written, fsynced intent is linked without replacing an existing
        // record BEFORE mkdir/marker. Initial unmarked crash cuts fail closed.
        let temporary = root.join(format!("{job}.transfer-intent-{}", owner.nonce));
        let result = (|| {
            let mut file = new_file(&temporary)?;
            file.write_all(&serde_json::to_vec(&owner).map_err(|_| E::Invalid)?)?;
            file.sync_all()?;
            fs::hard_link(&temporary, Self::record(root, job))?;
            File::open(root)?.sync_all()?;
            Ok::<_, E>(())
        })();
        let _ = fs::remove_file(&temporary);
        result?;
        let directory = Self::directory(root, job);
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let mut marker = new_file(&directory.join("owner"))?;
        marker.write_all(owner.nonce.as_bytes())?;
        marker.sync_all()?;
        File::open(&directory)?.sync_all()?;
        File::open(root)?.sync_all()?;
        Ok(owner)
    }
    pub fn load(root: &Path, job: &str) -> Result<Option<Self>, E> {
        if OpaqueId::try_from(job.to_owned()).is_err() {
            return Err(E::Invalid);
        }
        let mut file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(Self::record(root, job))
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.len() > 1024
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(E::Invalid);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let owner: Self = serde_json::from_slice(&bytes).map_err(|_| E::Invalid)?;
        owner.validate(job)?;
        Ok(Some(owner))
    }
    pub fn verify(&self, root: &Path) -> Result<(), E> {
        let directory = Self::directory(root, &self.job);
        let metadata = fs::symlink_metadata(&directory)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(E::Conflict);
        }
        let mut marker = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join("owner"))?;
        let metadata = marker.metadata()?;
        if !metadata.is_file() || metadata.len() != 64 || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(E::Conflict);
        }
        let mut bytes = [0; 64];
        marker.read_exact(&mut bytes)?;
        if bytes.as_slice() != self.nonce.as_bytes() {
            return Err(E::Conflict);
        }
        Ok(())
    }
    pub fn cleanup(&self, root: &Path) -> Result<(), E> {
        let directory = Self::directory(root, &self.job);
        match fs::symlink_metadata(&directory) {
            Ok(_) => {
                self.verify(root)?;
                fs::remove_dir_all(directory)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        File::open(root)?.sync_all()?;
        Ok(())
    }
    pub fn forget(self, root: &Path) -> Result<(), E> {
        fs::remove_file(Self::record(root, &self.job))?;
        File::open(root)?.sync_all()?;
        Ok(())
    }
}
fn new_file(path: &Path) -> Result<File, E> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}
