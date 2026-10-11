//! Durable preservation intake. Logical names never select physical filesystem paths.
use crate::Catalog;
use fs2::FileExt;
use lytronaut_contracts::{
    ApiError, ErrorCode, ImportRequest, JobReport, JobState, OpaqueId, WireVersion,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    Invalid,
    Quota,
    Busy,
    Cancelled,
    Conflict,
    Unavailable,
    Interrupted,
}
impl From<std::io::Error> for StorageError {
    fn from(_: std::io::Error) -> Self {
        Self::Unavailable
    }
}
impl From<sqlx::Error> for StorageError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "storage {:?}", self)
    }
}
impl std::error::Error for StorageError {}
/// Explicit administrator policy; declarations are upper bounds, never trusted actual counts.
#[derive(Clone, Debug)]
pub struct Limits {
    pub upload_bytes: u64,
    pub files: u32,
    pub file_bytes: u64,
    pub path_bytes: usize,
    pub depth: usize,
    pub metadata_bytes: u64,
    pub encoded_bytes: u64,
    pub active_jobs: u32,
    pub decoders: usize,
    pub reserved_bytes: u64,
    pub free_floor: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            upload_bytes: 20 * 1024 * 1024 * 1024,
            files: 100_000,
            file_bytes: 512 * 1024 * 1024,
            path_bytes: 4096,
            depth: 32,
            metadata_bytes: 32 * 1024 * 1024,
            encoded_bytes: 21 * 1024 * 1024 * 1024,
            active_jobs: 2,
            decoders: 1,
            reserved_bytes: 84 * 1024 * 1024 * 1024,
            free_floor: 64 * 1024 * 1024,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), StorageError> {
        if self.upload_bytes == 0
            || self.upload_bytes > i64::MAX as u64 / 4
            || self.files == 0
            || self.file_bytes == 0
            || self.file_bytes > self.upload_bytes
            || self.path_bytes == 0
            || self.path_bytes > 65535
            || self.depth == 0
            || self.metadata_bytes == 0
            || self.encoded_bytes == 0
            || self.encoded_bytes > i64::MAX as u64 / 4
            || self.active_jobs == 0
            || self.decoders == 0
            || self.decoders > Semaphore::MAX_PERMITS
            || self.reserved_bytes > i64::MAX as u64
            || self.free_floor > i64::MAX as u64
        {
            return Err(StorageError::Invalid);
        }
        Ok(())
    }
    /// Numeric environment overrides are explicit and fail closed on malformed values.
    pub fn from_environment() -> Result<Self, StorageError> {
        let mut l = Self::default();
        macro_rules! env {
            ($field:ident,$name:literal) => {
                if let Some(v) = std::env::var_os($name) {
                    l.$field = v
                        .to_str()
                        .ok_or(StorageError::Invalid)?
                        .parse()
                        .map_err(|_| StorageError::Invalid)?;
                }
            };
        }
        env!(upload_bytes, "LYTRONAUT_UPLOAD_BYTES");
        env!(files, "LYTRONAUT_UPLOAD_FILES");
        env!(file_bytes, "LYTRONAUT_FILE_BYTES");
        env!(path_bytes, "LYTRONAUT_PATH_BYTES");
        env!(depth, "LYTRONAUT_PATH_DEPTH");
        env!(metadata_bytes, "LYTRONAUT_ZIP_METADATA_BYTES");
        env!(encoded_bytes, "LYTRONAUT_ENCODED_BYTES");
        env!(active_jobs, "LYTRONAUT_IMPORT_JOBS");
        env!(decoders, "LYTRONAUT_IMPORT_DECODERS");
        env!(reserved_bytes, "LYTRONAUT_RESERVED_BYTES");
        env!(free_floor, "LYTRONAUT_FREE_FLOOR");
        l.validate()?;
        Ok(l)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    pub logical_path: String,
    pub digest: String,
    pub length: u64,
}
impl std::fmt::Debug for SourceEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourceEntry { redacted }")
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    BeforeWrite,
    AfterWrite,
    BeforeSync,
    AfterSync,
    BeforePublish,
    AfterPublish,
    BeforeTransaction,
    AfterTransaction,
}
#[derive(Clone)]
pub struct Store(Arc<Inner>);
struct Inner {
    catalog: Catalog,
    originals: PathBuf,
    staging: PathBuf,
    limits: Limits,
    gate: AsyncMutex<()>,
    cancel: Mutex<HashMap<String, Arc<AtomicBool>>>,
    decode: Arc<Semaphore>,
    verify: Arc<Semaphore>,
    uploads: Mutex<HashSet<String>>,
    fault: Mutex<Option<FaultPoint>>,
    _lock: File,
    _stage_lock: File,
}
fn unique() -> Result<String, StorageError> {
    let mut b = [0; 16];
    getrandom::fill(&mut b).map_err(|_| StorageError::Unavailable)?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
fn id_valid(id: &str) -> Result<(), StorageError> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StorageError::Invalid);
    }
    Ok(())
}
fn sync_dir(path: &Path) -> Result<(), StorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}
fn private_dir(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !path.exists() {
        let mut b = fs::DirBuilder::new();
        b.mode(0o700);
        b.create(path)?;
    }
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.file_type().is_symlink() || m.permissions().mode() & 0o077 != 0 {
        return Err(StorageError::Invalid);
    }
    Ok(())
}
fn new_file(path: &Path) -> Result<File, StorageError> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?)
}
fn regular(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_type().is_symlink() || m.nlink() != 1 {
        return Err(StorageError::Invalid);
    }
    Ok(())
}
/// Deliberately rejects aliases instead of normalizing traversal or platform-dependent names.
pub fn validate_path(path: &str, l: &Limits) -> Result<(), StorageError> {
    if path.is_empty()
        || path.len() > l.path_bytes
        || path.contains(['\0', '\\', ':'])
        || path.starts_with('/')
        || path.ends_with('/')
        || path.split('/').count() > l.depth
        || path.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || p.ends_with(['.', ' '])
                || p.chars().any(char::is_control)
        })
    {
        return Err(StorageError::Invalid);
    }
    Ok(())
}
fn archive_magic(prefix: &[u8]) -> bool {
    prefix.starts_with(b"PK\x03\x04")
        || prefix.starts_with(b"PK\x05\x06")
        || prefix.starts_with(b"PK\x07\x08")
        || prefix.starts_with(b"7z\xbc\xaf\x27\x1c")
        || prefix.starts_with(b"Rar!\x1a\x07")
        || prefix.starts_with(b"\x1f\x8b")
        || prefix.starts_with(b"BZh")
        || prefix.starts_with(b"\xfd7zXZ\0")
        || prefix.get(257..262) == Some(b"ustar")
}
impl Store {
    pub async fn open(
        catalog: Catalog,
        originals: &Path,
        staging: &Path,
        limits: Limits,
    ) -> Result<Self, StorageError> {
        limits.validate()?;
        if originals == staging {
            return Err(StorageError::Invalid);
        }
        private_dir(originals)?;
        private_dir(staging)?;
        // The application owns both roots exclusively. No other process can mutate its blobs/spools.
        fn root_lock(root: &Path) -> Result<File, StorageError> {
            let path = root.join("store.lock");
            let f = if path.exists() {
                regular(&path)?;
                OpenOptions::new().read(true).write(true).open(&path)?
            } else {
                new_file(&path)?
            };
            f.try_lock_exclusive().map_err(|_| StorageError::Busy)?;
            Ok(f)
        }
        let lock = root_lock(originals)?;
        let stage_lock = root_lock(staging)?;
        for sql in [
            "CREATE TABLE IF NOT EXISTS intake_jobs (id TEXT PRIMARY KEY, kind TEXT NOT NULL, state TEXT NOT NULL, total INTEGER NOT NULL, budget INTEGER NOT NULL, max_files INTEGER NOT NULL, encoded_limit INTEGER NOT NULL, zip_complete INTEGER NOT NULL DEFAULT 0, completed INTEGER NOT NULL DEFAULT 0, files INTEGER NOT NULL DEFAULT 0, path_bytes INTEGER NOT NULL DEFAULT 0, reserved INTEGER NOT NULL, error TEXT)",
            "CREATE TABLE IF NOT EXISTS intake_sources (job TEXT NOT NULL REFERENCES intake_jobs(id), path TEXT NOT NULL, canonical TEXT NOT NULL, digest TEXT NOT NULL, length INTEGER NOT NULL, PRIMARY KEY(job,path), UNIQUE(job,canonical))",
        ] {
            sqlx::query(sql).execute(catalog.pool()).await?;
        }
        let s = Self(Arc::new(Inner {
            catalog,
            originals: originals.into(),
            staging: staging.into(),
            decode: Arc::new(Semaphore::new(limits.decoders)),
            verify: Arc::new(Semaphore::new(limits.decoders)),
            limits,
            gate: AsyncMutex::new(()),
            cancel: Mutex::new(HashMap::new()),
            uploads: Mutex::new(HashSet::new()),
            fault: Mutex::new(None),
            _lock: lock,
            _stage_lock: stage_lock,
        }));
        s.recover().await?;
        Ok(s)
    }
    pub fn limits(&self) -> &Limits {
        &self.0.limits
    }
    /// Test/operator fault injection: one interruption at the selected durable boundary.
    pub fn interrupt_once(&self, point: FaultPoint) {
        *self.0.fault.lock().unwrap() = Some(point);
    }
    fn fault(&self, p: FaultPoint) -> Result<(), StorageError> {
        let mut f = self.0.fault.lock().unwrap();
        if *f == Some(p) {
            *f = None;
            return Err(StorageError::Interrupted);
        }
        Ok(())
    }
    pub async fn begin(&self, request: &ImportRequest) -> Result<String, StorageError> {
        request.validate().map_err(|_| StorageError::Invalid)?;
        let l = &self.0.limits;
        if request.declared_bytes > l.upload_bytes || request.declared_files > l.files {
            return Err(StorageError::Quota);
        }
        let _g = self.0.gate.lock().await;
        let per_file = l.file_bytes.min(request.declared_bytes);
        let encoded = match request.kind {
            lytronaut_contracts::ImportKind::Directory => per_file,
            _ => l.encoded_bytes.min(
                request
                    .declared_bytes
                    .saturating_add(request.declared_bytes / 100)
                    .saturating_add(l.metadata_bytes.saturating_mul(2))
                    .saturating_add(1024),
            ),
        };
        let reserve = request
            .declared_bytes
            .checked_add(encoded)
            .and_then(|x| x.checked_add(per_file))
            .ok_or(StorageError::Quota)?;
        let mut tx = self.0.catalog.pool().begin().await?;
        // Acquire SQLite's write reservation before calculating aggregate admission.
        sqlx::query("UPDATE intake_jobs SET reserved=reserved WHERE 0")
            .execute(&mut *tx)
            .await?;
        let row=sqlx::query("SELECT COUNT(*) n, COALESCE(SUM(reserved),0) bytes FROM intake_jobs WHERE state='running'").fetch_one(&mut *tx).await?;
        let n: i64 = row.get("n");
        let bytes: i64 = row.get("bytes");
        if n >= l.active_jobs as i64
            || (bytes as u64)
                .checked_add(reserve)
                .is_none_or(|x| x > l.reserved_bytes)
        {
            return Err(StorageError::Busy);
        }
        // Conservatively reserve every active job on both volumes; same-volume roots are safe too.
        for root in [&self.0.originals, &self.0.staging] {
            if fs2::available_space(root)?
                < (bytes as u64)
                    .saturating_add(reserve)
                    .saturating_add(l.free_floor)
            {
                return Err(StorageError::Quota);
            }
        }
        let id = unique()?;
        sqlx::query("INSERT INTO intake_jobs(id,kind,state,total,budget,max_files,encoded_limit,reserved) VALUES(?,?,'running',?,?,?,?,?)").bind(&id).bind(match request.kind {lytronaut_contracts::ImportKind::ZipLibrary=>"zip",_=>"directory"}).bind(request.declared_bytes as i64).bind(request.declared_bytes as i64).bind(request.declared_files as i64).bind(encoded as i64).bind(reserve as i64).execute(&mut *tx).await?;
        tx.commit().await?;
        self.0
            .cancel
            .lock()
            .unwrap()
            .insert(id.clone(), Arc::new(AtomicBool::new(false)));
        Ok(id)
    }
    fn flag(&self, id: &str) -> Result<Arc<AtomicBool>, StorageError> {
        self.0
            .cancel
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or(StorageError::Conflict)
    }
    async fn running(&self, id: &str) -> Result<sqlx::sqlite::SqliteRow, StorageError> {
        id_valid(id)?;
        let r = sqlx::query("SELECT * FROM intake_jobs WHERE id=?")
            .bind(id)
            .fetch_optional(self.0.catalog.pool())
            .await?
            .ok_or(StorageError::Invalid)?;
        if r.get::<String, _>("state") != "running" {
            return Err(StorageError::Conflict);
        }
        Ok(r)
    }
    /// A server-created encoded spool. Never accept a client-supplied filesystem path.
    pub async fn spool(&self, id: &str) -> Result<Spool, StorageError> {
        let _g = self.0.gate.lock().await;
        let r = self.running(id).await?;
        let flag = self.flag(id)?;
        let mut uploads = self.0.uploads.lock().unwrap();
        if !uploads.insert(id.to_string()) {
            return Err(StorageError::Busy);
        }
        let path = self.0.staging.join(format!("{id}-{}", unique()?));
        let file = match new_file(&path) {
            Ok(f) => f,
            Err(e) => {
                uploads.remove(id);
                return Err(e);
            }
        };
        Ok(Spool {
            owner: self.clone(),
            job: id.to_string(),
            path,
            file: Some(tokio::fs::File::from_std(file)),
            bytes: 0,
            limit: r.get::<i64, _>("encoded_limit") as u64,
            flag,
        })
    }
    pub async fn append_directory_file<R: Read + Send + 'static>(
        &self,
        id: &str,
        path: &str,
        reader: R,
    ) -> Result<(), StorageError> {
        if self.running(id).await?.get::<String, _>("kind") != "directory" {
            return Err(StorageError::Conflict);
        }
        self.append(id, path, reader, None).await
    }
    async fn append<R: Read + Send + 'static>(
        &self,
        id: &str,
        path: &str,
        reader: R,
        expected: Option<(u64, u32)>,
    ) -> Result<(), StorageError> {
        validate_path(path, &self.0.limits)?;
        let _g = self.0.gate.lock().await;
        let r = self.running(id).await?;
        if self.flag(id)?.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        let canonical = path.to_lowercase();
        let child = format!("{canonical}/");
        let upper = format!("{canonical}0");
        let collision:i64=sqlx::query_scalar("SELECT COUNT(*) FROM intake_sources WHERE job=? AND (canonical=? OR (canonical>=? AND canonical<?))").bind(id).bind(&canonical).bind(&child).bind(&upper).fetch_one(self.0.catalog.pool()).await?;
        if collision != 0 {
            return Err(StorageError::Conflict);
        }
        for (i, _) in canonical.match_indices('/') {
            let ancestor: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM intake_sources WHERE job=? AND canonical=?",
            )
            .bind(id)
            .bind(&canonical[..i])
            .fetch_one(self.0.catalog.pool())
            .await?;
            if ancestor != 0 {
                return Err(StorageError::Conflict);
            }
        }
        if (r.get::<i64, _>("path_bytes") as u64).saturating_add(path.len() as u64)
            > self.0.limits.metadata_bytes
        {
            return Err(StorageError::Quota);
        }
        let files: i64 = r.get("files");
        let max_files: i64 = r.get("max_files");
        if files >= max_files {
            return Err(StorageError::Quota);
        }
        let completed: i64 = r.get("completed");
        let total: i64 = r.get("total");
        let bound = (total - completed) as u64;
        let s = self.clone();
        let flag = self.flag(id)?;
        let entry = tokio::task::spawn_blocking(move || s.publish(reader, bound, expected, flag))
            .await
            .map_err(|_| StorageError::Unavailable)??;
        self.fault(FaultPoint::BeforeTransaction)?;
        let mut tx = self.0.catalog.pool().begin().await?;
        sqlx::query(
            "INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES(?,?,?,?,?)",
        )
        .bind(id)
        .bind(path)
        .bind(&canonical)
        .bind(&entry.0)
        .bind(entry.1 as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE intake_jobs SET completed=completed+?,files=files+1,path_bytes=path_bytes+? WHERE id=? AND state='running'").bind(entry.1 as i64).bind(path.len() as i64).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.fault(FaultPoint::AfterTransaction)?;
        Ok(())
    }
    fn publish<R: Read>(
        &self,
        mut reader: R,
        bound: u64,
        expected: Option<(u64, u32)>,
        cancel: Arc<AtomicBool>,
    ) -> Result<(String, u64), StorageError> {
        let temp = self.0.originals.join(format!("pending-{}", unique()?));
        let mut f = new_file(&temp)?;
        let result = (|| {
            self.fault(FaultPoint::BeforeWrite)?;
            let mut hash = Sha256::new();
            let mut crc = crc32fast::Hasher::new();
            let mut n = 0u64;
            let mut buf = [0; 64 * 1024];
            let mut prefix = Vec::with_capacity(512);
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(StorageError::Cancelled);
                }
                let got = reader.read(&mut buf)?;
                if got == 0 {
                    break;
                }
                n = n.checked_add(got as u64).ok_or(StorageError::Quota)?;
                if n > bound
                    || n > self.0.limits.file_bytes
                    || expected.is_some_and(|(size, _)| n > size)
                {
                    return Err(StorageError::Quota);
                }
                let prefix_count = (512 - prefix.len()).min(got);
                prefix.extend_from_slice(&buf[..prefix_count]);
                if archive_magic(&prefix) {
                    return Err(StorageError::Invalid);
                }
                hash.update(&buf[..got]);
                crc.update(&buf[..got]);
                f.write_all(&buf[..got])?;
                self.fault(FaultPoint::AfterWrite)?;
            }
            if expected.is_some_and(|(size, check)| n != size || crc.clone().finalize() != check) {
                return Err(StorageError::Invalid);
            }
            self.fault(FaultPoint::BeforeSync)?;
            f.sync_all()?;
            self.fault(FaultPoint::AfterSync)?;
            let digest = format!("{:x}", hash.finalize());
            let dest = self.0.originals.join(format!("{digest}-{n}"));
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o400))?;
            f.sync_all()?;
            self.fault(FaultPoint::BeforePublish)?;
            // Atomic no-replace publication on the target filesystem. Temp lives on this volume,
            // so separate staging volumes never cause EXDEV or fallback to copying partial blobs.
            match fs::hard_link(&temp, &dest) {
                Ok(()) => sync_dir(&self.0.originals)?,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    verify_equal(&temp, &dest, &digest, n)?
                }
                Err(e) => return Err(e.into()),
            }
            self.fault(FaultPoint::AfterPublish)?;
            Ok((digest, n))
        })();
        drop(f);
        let _ = fs::remove_file(&temp);
        let _ = sync_dir(&self.0.originals);
        result
    }
    pub async fn finish(&self, id: &str) -> Result<(), StorageError> {
        let _g = self.0.gate.lock().await;
        let r = self.running(id).await?;
        if self.0.uploads.lock().unwrap().contains(id) {
            return Err(StorageError::Busy);
        }
        if self.flag(id)?.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        if r.get::<String, _>("kind") == "zip" && r.get::<i64, _>("zip_complete") == 0 {
            return Err(StorageError::Conflict);
        }
        if r.get::<i64, _>("files") == 0 {
            return Err(StorageError::Invalid);
        }
        let _verify = self
            .0
            .verify
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let entries = self.entries(id).await?;
        let s = self.clone();
        let flag = self.flag(id)?;
        let verify_flag = flag.clone();
        tokio::task::spawn_blocking(move || {
            for e in entries {
                s.verified_blob(&e, Some(&verify_flag))?;
            }
            Ok::<_, StorageError>(())
        })
        .await
        .map_err(|_| StorageError::Unavailable)??;
        if flag.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        self.fault(FaultPoint::BeforeTransaction)?;
        sqlx::query("UPDATE intake_jobs SET state='succeeded',total=completed,reserved=0,error=NULL WHERE id=? AND state='running'").bind(id).execute(self.0.catalog.pool()).await?;
        self.0.cancel.lock().unwrap().remove(id);
        self.fault(FaultPoint::AfterTransaction)?;
        Ok(())
    }
    pub async fn job(&self, id: &str) -> Result<JobReport, StorageError> {
        id_valid(id)?;
        let r = sqlx::query("SELECT * FROM intake_jobs WHERE id=?")
            .bind(id)
            .fetch_optional(self.0.catalog.pool())
            .await?
            .ok_or(StorageError::Invalid)?;
        let state = match r.get::<String, _>("state").as_str() {
            "running" => JobState::Running,
            "succeeded" => JobState::Succeeded,
            "cancelled" => JobState::Cancelled,
            _ => JobState::Failed,
        };
        Ok(JobReport {
            version: WireVersion::V1,
            id: OpaqueId::try_from(id.to_string()).map_err(|_| StorageError::Invalid)?,
            state,
            completed_bytes: r.get::<i64, _>("completed") as u64,
            total_bytes: r.get::<i64, _>("total") as u64,
            error: if state == JobState::Failed {
                Some(ApiError {
                    version: WireVersion::V1,
                    code: if matches!(
                        r.get::<Option<String>, _>("error").as_deref(),
                        Some("invalid" | "quota" | "conflict")
                    ) {
                        ErrorCode::InvalidInput
                    } else {
                        ErrorCode::StorageUnavailable
                    },
                    retryable: !matches!(
                        r.get::<Option<String>, _>("error").as_deref(),
                        Some("invalid" | "quota" | "conflict")
                    ),
                })
            } else {
                None
            },
        })
    }
    async fn entries(&self, id: &str) -> Result<Vec<SourceEntry>, StorageError> {
        let rows =
            sqlx::query("SELECT path,digest,length FROM intake_sources WHERE job=? ORDER BY path")
                .bind(id)
                .fetch_all(self.0.catalog.pool())
                .await?;
        Ok(rows
            .into_iter()
            .map(|r| SourceEntry {
                logical_path: r.get("path"),
                digest: r.get("digest"),
                length: r.get::<i64, _>("length") as u64,
            })
            .collect())
    }
    /// Local/catalog API only; logical source paths must not be included in routine HTTP reports.
    pub async fn manifest(&self, id: &str) -> Result<Vec<SourceEntry>, StorageError> {
        if self.job(id).await?.state != JobState::Succeeded {
            return Err(StorageError::Conflict);
        }
        let _verify = self
            .0
            .verify
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageError::Unavailable)?;
        let entries = self.entries(id).await?;
        let s = self.clone();
        tokio::task::spawn_blocking(move || {
            for e in &entries {
                s.open_blob(e)?;
            }
            Ok(entries)
        })
        .await
        .map_err(|_| StorageError::Unavailable)?
    }
    /// Opens original content read-only, checking complete length and digest. Never a writable API.
    pub fn open_blob(&self, e: &SourceEntry) -> Result<File, StorageError> {
        self.verified_blob(e, None)
    }
    fn verified_blob(
        &self,
        e: &SourceEntry,
        cancel: Option<&AtomicBool>,
    ) -> Result<File, StorageError> {
        if e.digest.len() != 64
            || !e
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || e.length > self.0.limits.file_bytes
        {
            return Err(StorageError::Invalid);
        }
        let p = self.0.originals.join(format!("{}-{}", e.digest, e.length));
        verify_digest(&p, &e.digest, e.length, cancel)?;
        Ok(File::open(p)?)
    }
    pub async fn fail(&self, id: &str) -> Result<(), StorageError> {
        self.fail_with(id, StorageError::Unavailable).await
    }
    pub async fn fail_with(&self, id: &str, error: StorageError) -> Result<(), StorageError> {
        self.stop(
            id,
            "failed",
            Some(match error {
                StorageError::Invalid => "invalid",
                StorageError::Quota => "quota",
                StorageError::Conflict => "conflict",
                _ => "retryable",
            }),
        )
        .await
    }
    pub async fn cancel(&self, id: &str) -> Result<(), StorageError> {
        self.stop(id, "cancelled", None).await
    }
    async fn stop(&self, id: &str, state: &str, error: Option<&str>) -> Result<(), StorageError> {
        id_valid(id)?;
        if let Ok(f) = self.flag(id) {
            f.store(true, Ordering::Relaxed)
        }
        let _g = self.0.gate.lock().await;
        sqlx::query(
            "UPDATE intake_jobs SET state=?,reserved=0,error=? WHERE id=? AND state='running'",
        )
        .bind(state)
        .bind(error)
        .bind(id)
        .execute(self.0.catalog.pool())
        .await?;
        self.0.cancel.lock().unwrap().remove(id);
        self.cleanup(id)?;
        Ok(())
    }
    /// Retry starts a new intake with existing verified content dedup; old progress remains durable.
    pub async fn retry(&self, id: &str) -> Result<String, StorageError> {
        if !matches!(
            self.job(id).await?.state,
            JobState::Failed | JobState::Cancelled
        ) {
            return Err(StorageError::Conflict);
        }
        let r = sqlx::query("SELECT kind,budget,max_files FROM intake_jobs WHERE id=?")
            .bind(id)
            .fetch_one(self.0.catalog.pool())
            .await?;
        self.begin(&ImportRequest {
            version: WireVersion::V1,
            kind: if r.get::<String, _>("kind") == "zip" {
                lytronaut_contracts::ImportKind::ZipLibrary
            } else {
                lytronaut_contracts::ImportKind::Directory
            },
            declared_bytes: r.get::<i64, _>("budget") as u64,
            declared_files: r.get::<i64, _>("max_files") as u32,
        })
        .await
    }
    fn cleanup(&self, id: &str) -> Result<(), StorageError> {
        let prefix = format!("{id}-");
        for entry in fs::read_dir(&self.0.staging)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&prefix))
            {
                regular(&entry.path())?;
                fs::remove_file(entry.path())?;
            }
        }
        sync_dir(&self.0.staging)
    }
    async fn recover(&self) -> Result<(), StorageError> {
        // No worker survives the exclusive-root reopen. Interrupted jobs are retryable, never ready.
        sqlx::query("UPDATE intake_jobs SET state='failed',reserved=0,error='interrupted' WHERE state='running'").execute(self.0.catalog.pool()).await?;
        for root in [&self.0.originals, &self.0.staging] {
            for e in fs::read_dir(root)? {
                let e = e?;
                let n = e.file_name();
                let n = n.to_str().ok_or(StorageError::Invalid)?;
                if n == "store.lock" {
                    continue;
                }
                if root == &self.0.staging || n.starts_with("pending-") {
                    let m = fs::symlink_metadata(e.path())?;
                    if !m.is_file() || m.file_type().is_symlink() {
                        return Err(StorageError::Invalid);
                    }
                    fs::remove_file(e.path())?;
                }
            }
            sync_dir(root)?;
        }
        let mut last = String::new();
        while let Some(id) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM intake_jobs WHERE state='succeeded' AND id>? ORDER BY id LIMIT 1",
        )
        .bind(&last)
        .fetch_optional(self.0.catalog.pool())
        .await?
        {
            let entries = self.entries(&id).await?;
            let s = self.clone();
            let valid =
                tokio::task::spawn_blocking(move || entries.iter().all(|e| s.open_blob(e).is_ok()))
                    .await
                    .map_err(|_| StorageError::Unavailable)?;
            if !valid {
                sqlx::query("UPDATE intake_jobs SET state='failed',reserved=0,error='verification' WHERE id=?").bind(&id).execute(self.0.catalog.pool()).await?;
            }
            last = id;
        }
        Ok(())
    }
    /// The spool is owned/created by Store; ZIP metadata is bounded before allocating entries.
    pub async fn import_zip(&self, id: &str, spool: Spool) -> Result<(), StorageError> {
        if self.running(id).await?.get::<String, _>("kind") != "zip" {
            return Err(StorageError::Conflict);
        }
        let _permit = self
            .0
            .decode
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageError::Busy)?;
        let path = spool.path.clone();
        let l = self.0.limits.clone();
        let entries = tokio::task::spawn_blocking(move || crate::zip::preflight(&path, &l))
            .await
            .map_err(|_| StorageError::Unavailable)??;
        for entry in entries {
            let path = spool.path.clone();
            let name = entry.name.clone();
            let reader = entry.reader(&path)?;
            self.append(id, &name, reader, Some((entry.size, entry.crc)))
                .await?;
        }
        sqlx::query("UPDATE intake_jobs SET zip_complete=1 WHERE id=? AND state='running'")
            .bind(id)
            .execute(self.0.catalog.pool())
            .await?;
        drop(spool);
        self.finish(id).await
    }
}
/// Streaming upload spool bounded before every write. Drop removes abandoned encoded input.
pub struct Spool {
    owner: Store,
    job: String,
    path: PathBuf,
    file: Option<tokio::fs::File>,
    bytes: u64,
    limit: u64,
    flag: Arc<AtomicBool>,
}
impl Spool {
    pub fn restrict(&mut self, limit: u64) {
        self.limit = self.limit.min(limit)
    }
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
        use tokio::io::AsyncWriteExt;
        if self.flag.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(StorageError::Quota)?;
        if self.bytes > self.limit {
            return Err(StorageError::Quota);
        }
        self.file
            .as_mut()
            .ok_or(StorageError::Conflict)?
            .write_all(bytes)
            .await?;
        Ok(())
    }
    pub async fn sync(&mut self) -> Result<(), StorageError> {
        self.file
            .as_mut()
            .ok_or(StorageError::Conflict)?
            .sync_all()
            .await?;
        self.file.take();
        Ok(())
    }
    pub fn reader(&self) -> Result<File, StorageError> {
        if self.file.is_some() {
            return Err(StorageError::Conflict);
        }
        regular(&self.path)?;
        Ok(File::open(&self.path)?)
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        self.owner.0.uploads.lock().unwrap().remove(&self.job);
    }
}
fn verify_digest(
    path: &Path,
    digest: &str,
    len: u64,
    cancel: Option<&AtomicBool>,
) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;
    let m = fs::symlink_metadata(path)?;
    if !m.is_file()
        || m.file_type().is_symlink()
        || m.len() != len
        || m.permissions().mode() & 0o222 != 0
    {
        return Err(StorageError::Conflict);
    }
    let mut file = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        if cancel.is_some_and(|f| f.load(Ordering::Relaxed)) {
            return Err(StorageError::Cancelled);
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    if format!("{:x}", h.finalize()) != digest {
        return Err(StorageError::Conflict);
    }
    Ok(())
}
fn verify_equal(temp: &Path, dest: &Path, digest: &str, len: u64) -> Result<(), StorageError> {
    verify_digest(dest, digest, len, None)?;
    let mut a = File::open(temp)?;
    let mut b = File::open(dest)?;
    let mut x = [0; 65536];
    let mut y = [0; 65536];
    loop {
        let n = a.read(&mut x)?;
        b.read_exact(&mut y[..n])?;
        if x[..n] != y[..n] {
            return Err(StorageError::Conflict);
        }
        if n == 0 {
            break;
        }
    }
    Ok(())
}
#[cfg(test)]
mod publication_tests {
    use super::*;
    #[tokio::test]
    async fn simultaneous_no_replace_publication_verifies_dedup_bytes() {
        let root = std::env::temp_dir().join(format!("lytronaut-publish-{}", unique().unwrap()));
        fs::create_dir(&root).unwrap();
        let c = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let s = Store::open(
            c,
            &root.join("originals"),
            &root.join("staging"),
            Limits::default(),
        )
        .await
        .unwrap();
        let a = s.clone();
        let b = s.clone();
        let flag = Arc::new(AtomicBool::new(false));
        let fa = flag.clone();
        let (f, g) = tokio::join!(
            tokio::task::spawn_blocking(move || a.publish(&b"synthetic race"[..], 100, None, fa)),
            tokio::task::spawn_blocking(move || b.publish(&b"synthetic race"[..], 100, None, flag))
        );
        assert_eq!(f.unwrap().unwrap(), g.unwrap().unwrap());
        assert_eq!(fs::read_dir(root.join("originals")).unwrap().count(), 2);
        drop(s);
        fs::remove_dir_all(root).unwrap();
    }
}
