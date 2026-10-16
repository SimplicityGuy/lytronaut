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
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedRwLockReadGuard, RwLock, Semaphore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    Invalid,
    /// A declared or framed size exceeds a configured or format bound; the
    /// same request can never succeed (not retryable).
    Quota,
    /// The volume lacks the free space this work needs right now, including
    /// other jobs' reservations and the free-space floor (retryable).
    Space {
        required_bytes: u64,
        available_bytes: u64,
    },
    /// The volume lacks free inodes for the files this work creates (retryable).
    Inodes,
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
    /// No accepted byte/file progress for this duration cancels a running intake.
    pub import_idle_timeout: Duration,
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
            import_idle_timeout: Duration::from_secs(30 * 60),
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
            || self.import_idle_timeout.is_zero()
            || self.import_idle_timeout > Duration::from_secs(24 * 60 * 60)
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
        if let Some(value) = std::env::var_os("LYTRONAUT_IMPORT_IDLE_SECONDS") {
            let seconds = value
                .to_str()
                .ok_or(StorageError::Invalid)?
                .parse()
                .map_err(|_| StorageError::Invalid)?;
            l.import_idle_timeout = Duration::from_secs(seconds);
        }
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
    activity: Mutex<HashMap<String, Activity>>,
    #[cfg(test)]
    reaper_enabled: AtomicBool,
    #[cfg(test)]
    write_latch: Mutex<Option<Arc<TestLatch>>>,
    #[cfg(test)]
    preflight_latch: Mutex<Option<Arc<TestLatch>>>,
    fault: Mutex<Option<FaultPoint>>,
    _lock: File,
    _stage_lock: File,
}
struct Activity {
    last: tokio::time::Instant,
    processing: usize,
    drain: Arc<RwLock<()>>,
}
/// Owned by the async operation AND every blocking worker it starts. Dropping
/// the caller cannot release cancellation's drain before filesystem IO ends.
struct Processing {
    owner: Store,
    job: String,
    flag: Arc<AtomicBool>,
    renew: AtomicBool,
    _drain: OwnedRwLockReadGuard<()>,
}
impl Drop for Processing {
    fn drop(&mut self) {
        if let Some(activity) = self.owner.0.activity.lock().unwrap().get_mut(&self.job) {
            activity.processing -= 1;
            if self.renew.load(Ordering::Relaxed) && !self.flag.load(Ordering::Relaxed) {
                activity.last = tokio::time::Instant::now();
            }
        }
    }
}
#[cfg(test)]
struct TestLatch {
    entered: tokio::sync::Notify,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
#[cfg(test)]
impl TestLatch {
    fn wait(&self) {
        self.entered.notify_one();
        self.release.lock().unwrap().recv().unwrap();
    }
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
    /// Stop active intake after the listener stops accepting work; no original is deleted.
    pub async fn shutdown(&self) -> Result<(), StorageError> {
        let jobs: Vec<String> =
            sqlx::query_scalar("SELECT id FROM intake_jobs WHERE state='running'")
                .fetch_all(self.0.catalog.pool())
                .await?;
        for id in jobs {
            self.cancel(&id).await?;
        }
        Ok(())
    }
    pub fn cancellation_requested(&self, id: &str) -> bool {
        self.flag(id)
            .map_or(true, |flag| flag.load(Ordering::Relaxed))
    }

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
        let mut migration = catalog.begin_write().await?;
        for &sql in catalog.schema_statements(crate::SchemaComponent::Intake) {
            sqlx::query(sql).execute(&mut *migration).await?;
        }
        migration.commit().await?;
        catalog
            .backfill_job_order(crate::SchemaComponent::Intake)
            .await?;
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
            activity: Mutex::new(HashMap::new()),
            #[cfg(test)]
            reaper_enabled: AtomicBool::new(true),
            #[cfg(test)]
            write_latch: Mutex::new(None),
            #[cfg(test)]
            preflight_latch: Mutex::new(None),
            fault: Mutex::new(None),
            _lock: lock,
            _stage_lock: stage_lock,
        }));
        s.recover().await?;
        // The worker cannot retain the store or its exclusive filesystem locks
        // across its timer. Restart recovery already ends all prior running jobs.
        let owner = Arc::downgrade(&s.0);
        let interval = s.0.limits.import_idle_timeout.min(Duration::from_secs(1));
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let Some(inner) = owner.upgrade() else { break };
                #[cfg(test)]
                if !inner.reaper_enabled.load(Ordering::Relaxed) {
                    continue;
                }
                Store(inner).expire_idle().await;
            }
        });
        Ok(s)
    }
    fn progress(&self, id: &str) {
        let flag = self.flag(id).ok();
        if let Some(activity) = self.0.activity.lock().unwrap().get_mut(id)
            && flag.is_some_and(|flag| !flag.load(Ordering::Relaxed))
        {
            activity.last = tokio::time::Instant::now();
        }
    }
    fn drain(&self, id: &str) -> Result<Arc<RwLock<()>>, StorageError> {
        self.0
            .activity
            .lock()
            .unwrap()
            .get(id)
            .map(|a| a.drain.clone())
            .ok_or(StorageError::Conflict)
    }
    async fn processing(&self, id: &str) -> Result<Arc<Processing>, StorageError> {
        let drain = self
            .drain(id)?
            .try_read_owned()
            .map_err(|_| StorageError::Cancelled)?;
        let flag = self.flag(id)?;
        {
            let mut activities = self.0.activity.lock().unwrap();
            let activity = activities.get_mut(id).ok_or(StorageError::Conflict)?;
            if flag.load(Ordering::Relaxed) {
                return Err(StorageError::Cancelled);
            }
            activity.processing += 1;
        }
        Ok(Arc::new(Processing {
            owner: self.clone(),
            job: id.to_owned(),
            flag,
            renew: AtomicBool::new(false),
            _drain: drain,
        }))
    }
    async fn expire_idle(&self) {
        let timeout = self.0.limits.import_idle_timeout;
        let expired: Vec<String> = {
            let activity = self.0.activity.lock().unwrap();
            activity
                .iter()
                .filter_map(|(id, activity)| {
                    let flag = self.flag(id).ok()?;
                    if activity.processing != 0
                        || (activity.last.elapsed() < timeout && !flag.load(Ordering::Relaxed))
                    {
                        return None;
                    }
                    // Progress/processing admission uses this same mutex. Once this
                    // cancellation wins, late completion cannot renew the job.
                    flag.store(true, Ordering::Relaxed);
                    Some(id.clone())
                })
                .collect()
        };
        for id in expired {
            // stop drains real IO before acquiring the global gate, so an active
            // decoder can finish/unwind without deadlocking on that gate.
            let _ = self.stop(&id, "cancelled", None).await;
        }
    }
    /// Application-owned scratch root, never a source library.
    pub fn available_staging_bytes(&self) -> Result<u64, StorageError> {
        Ok(fs2::available_space(&self.0.staging)?)
    }
    pub fn staging_root(&self) -> &Path {
        &self.0.staging
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
        let mut tx = self.0.catalog.begin_write().await?;
        // Writer admission precedes aggregate quota reads on both engines.
        let row=sqlx::query("SELECT COUNT(*) n, CAST(COALESCE(SUM(reserved),0) AS BIGINT) bytes FROM intake_jobs WHERE state='running'").fetch_one(&mut *tx).await?;
        let n: i64 = row.get("n");
        let bytes: i64 = row.get("bytes");
        if n >= l.active_jobs as i64
            || (bytes as u64)
                .checked_add(reserve)
                .is_none_or(|x| x > l.reserved_bytes)
        {
            return Err(StorageError::Busy);
        }
        // Export and restore jobs share the work volume. Their in-flight
        // reservations count here, under the same writer admission, so an
        // import and a whole-library export cannot both pass the free-space
        // check and then together exhaust the volume (export admission counts
        // running import reservations in turn).
        let exports_present: i64 = sqlx::query_scalar(self.0.catalog.dialect(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='export_jobs'",
            "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=current_schema() AND c.relkind='r' AND c.relname='export_jobs'",
        ))
        .fetch_one(&mut *tx)
        .await?;
        let exports: i64 = if exports_present == 1 {
            sqlx::query_scalar("SELECT CAST(COALESCE(sum(reserved),0) AS BIGINT) FROM export_jobs")
                .fetch_one(&mut *tx)
                .await?
        } else {
            0
        };
        // Conservatively reserve every active job on both volumes; same-volume roots are safe too.
        // A shortage is retryable and names its numbers; only a declaration
        // beyond the configured budget (checked above) is a permanent Quota.
        for (root, others) in [(&self.0.originals, 0), (&self.0.staging, exports as u64)] {
            let required = (bytes as u64)
                .saturating_add(reserve)
                .saturating_add(others)
                .saturating_add(l.free_floor);
            let available = fs2::available_space(root)?;
            if available < required {
                return Err(StorageError::Space {
                    required_bytes: required,
                    available_bytes: available,
                });
            }
        }
        let id = unique()?;
        sqlx::query("INSERT INTO intake_jobs(id,kind,state,total,budget,max_files,encoded_limit,reserved) VALUES($1,$2,'running',$3,$4,$5,$6,$7)").bind(&id).bind(match request.kind {lytronaut_contracts::ImportKind::ZipLibrary=>"zip",_=>"directory"}).bind(request.declared_bytes as i64).bind(request.declared_bytes as i64).bind(request.declared_files as i64).bind(encoded as i64).bind(reserve as i64).execute(&mut *tx).await?;
        self.0
            .catalog
            .record_job_order(&mut tx, crate::SchemaComponent::Intake, &id)
            .await?;
        tx.commit().await?;
        self.0
            .cancel
            .lock()
            .unwrap()
            .insert(id.clone(), Arc::new(AtomicBool::new(false)));
        self.0.activity.lock().unwrap().insert(
            id.clone(),
            Activity {
                last: tokio::time::Instant::now(),
                processing: 0,
                drain: Arc::new(RwLock::new(())),
            },
        );
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
    async fn running(&self, id: &str) -> Result<sqlx::any::AnyRow, StorageError> {
        id_valid(id)?;
        let r = sqlx::query("SELECT * FROM intake_jobs WHERE id=$1")
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
        drop(file);
        Ok(Spool {
            owner: self.clone(),
            job: id.to_string(),
            path,
            sealed: false,
            released: false,
            io: Arc::new(AsyncMutex::new(())),
            drain: self.drain(id)?,
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
        let processing = self.processing(id).await?;
        self.append_processing(id, path, reader, expected, processing)
            .await
    }
    async fn append_processing<R: Read + Send + 'static>(
        &self,
        id: &str,
        path: &str,
        reader: R,
        expected: Option<(u64, u32)>,
        processing: Arc<Processing>,
    ) -> Result<(), StorageError> {
        let _operation = processing.clone();
        validate_path(path, &self.0.limits)?;
        let _g = self.0.gate.lock().await;
        let r = self.running(id).await?;
        if self.flag(id)?.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        let canonical = path.to_lowercase();
        let child = format!("{canonical}/");
        let upper = format!("{canonical}0");
        let collision:i64=sqlx::query_scalar("SELECT COUNT(*) FROM intake_sources WHERE job=$1 AND (canonical=$2 OR (canonical>=$3 AND canonical<$4))").bind(id).bind(&canonical).bind(&child).bind(&upper).fetch_one(self.0.catalog.pool()).await?;
        if collision != 0 {
            return Err(StorageError::Conflict);
        }
        for (i, _) in canonical.match_indices('/') {
            let ancestor: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM intake_sources WHERE job=$1 AND canonical=$2",
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
        processing.renew.store(true, Ordering::Relaxed);
        let entry = tokio::task::spawn_blocking(move || {
            let _processing = processing;
            s.publish(reader, bound, expected, flag)
        })
        .await
        .map_err(|_| StorageError::Unavailable)??;
        self.fault(FaultPoint::BeforeTransaction)?;
        let mut tx = self.0.catalog.begin_write().await?;
        sqlx::query(
            "INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES($1,$2,$3,$4,$5)",
        )
        .bind(id)
        .bind(path)
        .bind(&canonical)
        .bind(&entry.0)
        .bind(entry.1 as i64)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE intake_jobs SET completed=completed+$1,files=files+1,path_bytes=path_bytes+$2 WHERE id=$3 AND state='running'").bind(entry.1 as i64).bind(path.len() as i64).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.fault(FaultPoint::AfterTransaction)?;
        self.progress(id);
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
        let processing = self.processing(id).await?;
        self.finish_processing(id, processing).await
    }
    async fn finish_processing(
        &self,
        id: &str,
        processing: Arc<Processing>,
    ) -> Result<(), StorageError> {
        let _operation = processing.clone();
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
        processing.renew.store(true, Ordering::Relaxed);
        tokio::task::spawn_blocking(move || {
            let _processing = processing;
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
        sqlx::query("UPDATE intake_jobs SET state='succeeded',total=completed,reserved=0,error=NULL WHERE id=$1 AND state='running'").bind(id).execute(self.0.catalog.pool()).await?;
        self.0.cancel.lock().unwrap().remove(id);
        self.0.activity.lock().unwrap().remove(id);
        self.fault(FaultPoint::AfterTransaction)?;
        Ok(())
    }
    pub async fn job(&self, id: &str) -> Result<JobReport, StorageError> {
        id_valid(id)?;
        let r = sqlx::query("SELECT * FROM intake_jobs WHERE id=$1")
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
                    shortfall: None,
                })
            } else {
                None
            },
        })
    }
    async fn entries(&self, id: &str) -> Result<Vec<SourceEntry>, StorageError> {
        let rows =
            sqlx::query("SELECT path,digest,length FROM intake_sources WHERE job=$1 ORDER BY path")
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
        // No global gate while waiting: processing readers may need it in
        // order to observe cancellation and unwind. The write lease excludes
        // new writes and covers already spawned blocking IO, even orphaned IO.
        let _drain = if let Ok(drain) = self.drain(id) {
            Some(drain.write_owned().await)
        } else {
            None
        };
        let _g = self.0.gate.lock().await;
        self.stop_locked(id, state, error).await
    }
    async fn stop_locked(
        &self,
        id: &str,
        state: &str,
        error: Option<&str>,
    ) -> Result<(), StorageError> {
        // Release capacity only after the job-owned encoded files are gone.
        // Sealed originals remain immutable and available for dedup/recovery.
        self.cleanup(id)?;
        sqlx::query(
            "UPDATE intake_jobs SET state=$1,reserved=0,error=$2 WHERE id=$3 AND state='running'",
        )
        .bind(state)
        .bind(error)
        .bind(id)
        .execute(self.0.catalog.pool())
        .await?;
        self.0.cancel.lock().unwrap().remove(id);
        self.0.activity.lock().unwrap().remove(id);
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
        let r = sqlx::query("SELECT kind,budget,max_files FROM intake_jobs WHERE id=$1")
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
        use std::os::unix::fs::MetadataExt;
        let prefix = format!("{id}-");
        for entry in fs::read_dir(&self.0.staging)? {
            let entry = entry?;
            if !entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&prefix))
            {
                continue;
            }
            // Spool::drop may already have removed exactly this owned path after
            // draining its IO. Missing is idempotent; aliases remain forbidden.
            let metadata = match fs::symlink_metadata(entry.path()) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                value => value?,
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.nlink() != 1 {
                return Err(StorageError::Invalid);
            }
            match fs::remove_file(entry.path()) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                value => value?,
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
            "SELECT id FROM intake_jobs WHERE state='succeeded' AND id>$1 ORDER BY id LIMIT 1",
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
                sqlx::query("UPDATE intake_jobs SET state='failed',reserved=0,error='verification' WHERE id=$1").bind(&id).execute(self.0.catalog.pool()).await?;
            }
            last = id;
        }
        Ok(())
    }
    /// The spool is owned/created by Store; ZIP metadata is bounded before allocating entries.
    pub async fn import_zip(&self, id: &str, mut spool: Spool) -> Result<(), StorageError> {
        if self.running(id).await?.get::<String, _>("kind") != "zip" {
            return Err(StorageError::Conflict);
        }
        let processing = self.processing(id).await?;
        let _permit = loop {
            tokio::select! {
                permit = self.0.decode.clone().acquire_owned() => break permit.map_err(|_|StorageError::Busy)?,
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if processing.flag.load(Ordering::Relaxed) { return Err(StorageError::Cancelled); }
                }
            }
        };
        let path = spool.path.clone();
        let l = self.0.limits.clone();
        processing.renew.store(true, Ordering::Relaxed);
        let preflight_processing = processing.clone();
        let entries = tokio::task::spawn_blocking(move || {
            let _processing = preflight_processing;
            #[cfg(test)]
            if let Some(latch) = _processing.owner.0.preflight_latch.lock().unwrap().take() {
                let _open = File::open(&path)?;
                latch.wait();
            }
            if _processing.flag.load(Ordering::Relaxed) {
                return Err(StorageError::Cancelled);
            }
            crate::zip::preflight(&path, &l)
        })
        .await
        .map_err(|_| StorageError::Unavailable)??;
        for entry in entries {
            let path = spool.path.clone();
            let name = entry.name.clone();
            let reader = entry.reader(&path)?;
            self.append_processing(
                id,
                &name,
                reader,
                Some((entry.size, entry.crc)),
                processing.clone(),
            )
            .await?;
        }
        sqlx::query("UPDATE intake_jobs SET zip_complete=1 WHERE id=$1 AND state='running'")
            .bind(id)
            .execute(self.0.catalog.pool())
            .await?;
        // All preflight/publication workers have been joined. Remove this
        // input under our existing processing lease before finish releases quota.
        spool.discard_ready()?;
        drop(spool);
        self.finish_processing(id, processing).await
    }
}
/// Streaming upload spool bounded before every write. Blocking tasks own their
/// file and drain leases until synchronous filesystem IO is truly complete.
pub struct Spool {
    owner: Store,
    job: String,
    path: PathBuf,
    sealed: bool,
    released: bool,
    io: Arc<AsyncMutex<()>>,
    drain: Arc<RwLock<()>>,
    bytes: u64,
    limit: u64,
    flag: Arc<AtomicBool>,
}
impl Spool {
    pub fn restrict(&mut self, limit: u64) {
        self.limit = self.limit.min(limit)
    }
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), StorageError> {
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
        if self.sealed {
            return Err(StorageError::Conflict);
        }
        // One additional owned 64 KiB chunk, never a copy of the full request.
        for chunk in bytes.chunks(65536) {
            let drain = self.drain.clone().read_owned().await;
            let io = self.io.clone().lock_owned().await;
            if self.flag.load(Ordering::Relaxed) {
                return Err(StorageError::Cancelled);
            }
            let chunk = chunk.to_vec();
            let path = self.path.clone();
            let flag = self.flag.clone();
            let owner = self.owner.clone();
            let job = self.job.clone();
            tokio::task::spawn_blocking(move || {
                let (_drain, _io) = (drain, io);
                if flag.load(Ordering::Relaxed) {
                    return Err(StorageError::Cancelled);
                }
                let mut file = OpenOptions::new().append(true).open(path)?;
                let result = file.write_all(&chunk).map_err(StorageError::from);
                #[cfg(test)]
                if let Some(latch) = owner.0.write_latch.lock().unwrap().take() {
                    latch.wait();
                }
                // No FD or buffered Tokio IO survives release of the drain.
                drop(file);
                if flag.load(Ordering::Relaxed) {
                    return Err(StorageError::Cancelled);
                }
                if result.is_ok() {
                    owner.progress(&job);
                }
                result
            })
            .await
            .map_err(|_| StorageError::Unavailable)??;
        }
        Ok(())
    }
    pub async fn sync(&mut self) -> Result<(), StorageError> {
        let drain = self.drain.clone().read_owned().await;
        let io = self.io.clone().lock_owned().await;
        if self.flag.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        if self.sealed {
            return Err(StorageError::Conflict);
        }
        let path = self.path.clone();
        let flag = self.flag.clone();
        tokio::task::spawn_blocking(move || {
            let (_drain, _io) = (drain, io);
            if flag.load(Ordering::Relaxed) {
                return Err(StorageError::Cancelled);
            }
            let file = OpenOptions::new().write(true).open(path)?;
            let result = file.sync_all().map_err(StorageError::from);
            drop(file);
            if flag.load(Ordering::Relaxed) {
                return Err(StorageError::Cancelled);
            }
            result
        })
        .await
        .map_err(|_| StorageError::Unavailable)??;
        self.sealed = true;
        Ok(())
    }
    fn discard_ready(&mut self) -> Result<(), StorageError> {
        match fs::remove_file(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            value => value?,
        }
        self.owner.0.uploads.lock().unwrap().remove(&self.job);
        self.released = true;
        Ok(())
    }
    pub fn reader(&self) -> Result<SpoolReader, StorageError> {
        if !self.sealed {
            return Err(StorageError::Conflict);
        }
        let drain = self
            .drain
            .clone()
            .try_read_owned()
            .map_err(|_| StorageError::Cancelled)?;
        if self.flag.load(Ordering::Relaxed) {
            return Err(StorageError::Cancelled);
        }
        regular(&self.path)?;
        Ok(SpoolReader {
            file: File::open(&self.path)?,
            _drain: drain,
        })
    }
}
/// The file closes before its cancellation drain is released, including when a
/// caller drops a pending append future before publication starts.
pub struct SpoolReader {
    file: File,
    _drain: OwnedRwLockReadGuard<()>,
}
impl Read for SpoolReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(bytes)
    }
}
impl Drop for Spool {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        if let Ok(_drain) = self.drain.clone().try_write_owned() {
            let _ = fs::remove_file(&self.path);
            self.owner.0.uploads.lock().unwrap().remove(&self.job);
        } else {
            // The caller may have been dropped while its blocking worker still
            // reads/writes. Keep the input until every task-owned reader ends.
            let drain = self.drain.clone();
            let path = self.path.clone();
            let owner = self.owner.clone();
            let job = self.job.clone();
            tokio::spawn(async move {
                let _drain = drain.write_owned().await;
                let _ = fs::remove_file(path);
                owner.0.uploads.lock().unwrap().remove(&job);
            });
        }
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
    async fn idle_progress_finish_and_failed_cleanup_keep_their_ownership_invariants() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("lytronaut-idle-{}", unique().unwrap()));
        fs::create_dir(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let idle = Duration::from_secs(10);
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                free_floor: 0,
                import_idle_timeout: idle,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        let request = ImportRequest {
            version: WireVersion::V1,
            kind: lytronaut_contracts::ImportKind::Directory,
            declared_bytes: 128,
            declared_files: 4,
        };
        let id = store.begin(&request).await.unwrap();
        let mut spool = store.spool(&id).await.unwrap();
        let stale = tokio::time::Instant::now() - idle;
        store.0.activity.lock().unwrap().get_mut(&id).unwrap().last = stale;
        spool.write(b"").await.unwrap();
        assert_eq!(
            store.0.activity.lock().unwrap()[&id].last,
            stale,
            "empty frames are not progress"
        );
        spool.write(b"public bytes").await.unwrap();
        assert!(store.0.activity.lock().unwrap()[&id].last > stale);
        store.expire_idle().await;
        assert_eq!(store.job(&id).await.unwrap().state, JobState::Running);
        drop(spool);
        store
            .append_directory_file(&id, "synthetic.txt", &b"sealed synthetic original"[..])
            .await
            .unwrap();
        store.finish(&id).await.unwrap();
        assert!(!store.0.activity.lock().unwrap().contains_key(&id));
        store.expire_idle().await;
        assert_eq!(store.job(&id).await.unwrap().state, JobState::Succeeded);
        let entries = store.manifest(&id).await.unwrap();
        let mut bytes = Vec::new();
        store
            .open_blob(&entries[0])
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"sealed synthetic original");

        let blocked = store.begin(&request).await.unwrap();
        let untouched = root.join("untouched");
        fs::write(&untouched, b"outside the job").unwrap();
        let bad = root.join("staging").join(format!("{blocked}-unsafe"));
        symlink(&untouched, &bad).unwrap();
        store
            .0
            .activity
            .lock()
            .unwrap()
            .get_mut(&blocked)
            .unwrap()
            .last = tokio::time::Instant::now() - idle;
        store.expire_idle().await;
        let row = sqlx::query("SELECT reserved,state FROM intake_jobs WHERE id=$1")
            .bind(&blocked)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("state"), "running");
        assert!(
            row.get::<i64, _>("reserved") > 0,
            "failed cleanup must retain quota"
        );
        assert_eq!(fs::read(&untouched).unwrap(), b"outside the job");
        fs::remove_file(&bad).unwrap();
        store.expire_idle().await;
        assert_eq!(
            store.job(&blocked).await.unwrap().state,
            JobState::Cancelled
        );
        assert!(!store.0.activity.lock().unwrap().contains_key(&blocked));
        assert_eq!(fs::read(&untouched).unwrap(), b"outside the job");
        drop(store);
        catalog.pool().close().await;
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn idle_timeout_configuration_is_bounded_and_cannot_be_disabled() {
        for idle in [Duration::ZERO, Duration::from_secs(86401)] {
            assert_eq!(
                Limits {
                    import_idle_timeout: idle,
                    ..Limits::default()
                }
                .validate(),
                Err(StorageError::Invalid)
            );
        }
        assert_eq!(
            Limits::default().import_idle_timeout,
            Duration::from_secs(1800)
        );
    }

    #[tokio::test]
    async fn shutdown_cancels_intake_and_preserves_existing_originals() {
        let root = std::env::temp_dir().join(format!("lytronaut-shutdown-{}", unique().unwrap()));
        fs::create_dir(&root).unwrap();
        let c = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let s = Store::open(
            c.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                free_floor: 0,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        let job = s
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: lytronaut_contracts::ImportKind::Directory,
                declared_bytes: 100,
                declared_files: 2,
            })
            .await
            .unwrap();
        s.append_directory_file(&job, "kept.bin", &b"synthetic original"[..])
            .await
            .unwrap();
        let sources = s.entries(&job).await.unwrap();
        let flag = s.flag(&job).unwrap();
        assert!(!s.cancellation_requested(&job));
        s.shutdown().await.unwrap();
        assert!(flag.load(Ordering::Relaxed));
        assert!(s.cancellation_requested(&job));
        assert_eq!(s.job(&job).await.unwrap().state, JobState::Cancelled);
        assert_eq!(
            s.open_blob(&sources[0]).unwrap().metadata().unwrap().len(),
            18
        );
        s.shutdown().await.unwrap();
        drop(s);
        c.pool().close().await;
        fs::remove_dir_all(root).unwrap();
    }
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

#[cfg(test)]
#[path = "intake_idle_tests.rs"]
mod idle_tests;
