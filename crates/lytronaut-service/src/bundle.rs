//! Versioned private lossless bundle. Restores publish only an isolated application copy.
//! Original source databases and unknown files are opaque, byte-exact immutable blobs.
use lytronaut_storage::{Catalog, Limits, StorageError as E, Store};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
const MAGIC: &[u8; 16] = b"LYTRONAUT-BNDL01";
const PORTABLE_MAGIC: &[u8; 16] = b"LYTRONAUT-BNDL02";
const LEGACY_AUTH: [(&str, &str); 3] = [
    (
        "auth_account",
        "CREATE TABLE auth_account (singleton INTEGER PRIMARY KEY CHECK(singleton=1), username_hash TEXT, password_hash TEXT, bootstrap_hash TEXT)",
    ),
    (
        "auth_sessions",
        "CREATE TABLE auth_sessions (token_hash TEXT PRIMARY KEY, expires INTEGER NOT NULL, created INTEGER NOT NULL)",
    ),
    (
        "auth_attempts",
        "CREATE TABLE auth_attempts (kind TEXT PRIMARY KEY, start INTEGER NOT NULL, count INTEGER NOT NULL)",
    ),
];
const MAX_DB: u64 = 32 * 1024 * 1024;
/// The one working buffer every bundle transfer moves bytes through. Bundle
/// creation and restore never hold an original or the bundle in memory, so a
/// library of any size needs the same memory (docs/design/007, lytronaut-lbk).
pub(crate) const COPY_BUFFER: usize = 65536;
/// Conservative allowance for auxiliary job files/directories (ownership
/// markers, journals and destination database side files). Explicit payload
/// files and spools are charged separately.
const JOB_FILES: u64 = 16;
/// Per-file allowance for inode allocation and directory entries, in addition
/// to rounded data blocks. This is a sizing model, not an exact filesystem bill.
const FILE_METADATA: u64 = 1024;
/// Inodes kept free beyond the files a restore creates.
pub(crate) const INODE_FLOOR: u64 = 1024;
/// Inode admission is advisory against other filesystem users; the service's
/// one validation permit serializes restore checks through extraction.
pub(crate) fn check_restore_inodes(originals: u64, available: Option<u64>) -> Result<(), E> {
    let Some(available) = available else {
        return Ok(());
    };
    let required = originals
        .checked_add(JOB_FILES)
        .and_then(|n| n.checked_add(INODE_FLOOR))
        .ok_or(E::Inodes)?;
    if available < required {
        Err(E::Inodes)
    } else {
        Ok(())
    }
}
/// The filesystem's allocation unit and free inodes (`None` when it does not
/// report a finite inode count). Every file occupies whole units, so per-file
/// cost is real disk, not payload bytes.
pub(crate) fn block_unit(path: &Path) -> Result<(u64, Option<u64>), E> {
    let fs = rustix::fs::statvfs(path).map_err(|_| E::Unavailable)?;
    let unit = if fs.f_frsize > 0 {
        fs.f_frsize
    } else {
        fs.f_bsize
    };
    Ok((unit.max(512), (fs.f_files > 0).then_some(fs.f_favail)))
}
/// Modeled cost of one file: at least one data allocation unit, plus a
/// separate inode/directory metadata allowance.
pub(crate) fn file_cost(length: u64, unit: u64) -> Result<u64, E> {
    if unit == 0 {
        return Err(E::Quota);
    }
    length
        .div_ceil(unit)
        .max(1)
        .checked_mul(unit)
        .and_then(|bytes| bytes.checked_add(FILE_METADATA))
        .ok_or(E::Quota)
}
fn job_cost(unit: u64) -> Result<u64, E> {
    file_cost(0, unit)?.checked_mul(JOB_FILES).ok_or(E::Quota)
}
fn restore_base_need(upload: u64, unit: u64) -> Result<u64, E> {
    file_cost(upload, unit)?
        .checked_add(lytronaut_storage::portable::MAX_BYTES as u64)
        .and_then(|bytes| bytes.checked_add(job_cost(unit).ok()?))
        .ok_or(E::Quota)
}
/// Running work-volume accounting of one job against its reservation: every
/// extracted file and artifact growth is charged before writing. Backup
/// capture spools are checked after capture; see `backup` for that limitation.
pub(crate) struct DiskBudget {
    limit: u64,
    used: u64,
    unit: u64,
}
impl DiskBudget {
    pub(crate) fn new(limit: u64, used: u64, unit: u64) -> Self {
        Self { limit, used, unit }
    }
    /// No accounting (standalone tooling restores without a reservation).
    fn unlimited() -> Self {
        Self::new(u64::MAX, 0, 512)
    }
    /// Charge one new file at its modeled allocation and metadata cost.
    fn charge(&mut self, length: u64) -> Result<(), E> {
        self.grow(file_cost(length, self.unit)?)
    }
    /// Grow an already charged file, charging only newly occupied blocks.
    /// Its inode/directory allowance is paid once, not once per chunk.
    fn grow_file(&mut self, previous: u64, next: u64) -> Result<(), E> {
        if next < previous {
            return Err(E::Invalid);
        }
        self.grow(file_cost(next, self.unit)? - file_cost(previous, self.unit)?)
    }
    /// Charge additional modeled allocation.
    fn grow(&mut self, bytes: u64) -> Result<(), E> {
        let used = self.used.checked_add(bytes).ok_or(E::Quota)?;
        if used > self.limit {
            return Err(E::Space {
                required_bytes: used,
                available_bytes: self.limit,
            });
        }
        self.used = used;
        Ok(())
    }
}
/// Modeled work-volume need of restoring the uploaded bundle at `input`, read
/// from its framing without hashing or extracting anything: rounded upload,
/// catalog headroom, auxiliary files, spools and metadata-inclusive originals.
/// Returns the unique-original count and that conservative sizing estimate.
pub(crate) fn restore_disk_need(
    input: &Path,
    unit: u64,
    file_bytes: u64,
    cancel: &AtomicBool,
) -> Result<(u64, u64), E> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(input)?;
    let upload = file.metadata()?.len();
    let mut header = [0; 60];
    file.read_exact(&mut header)?;
    let section = if &header[..16] == PORTABLE_MAGIC {
        lytronaut_storage::portable::MAX_BYTES as u64
    } else if &header[..16] == MAGIC {
        MAX_DB
    } else {
        return Err(E::Invalid);
    };
    let count = u64::from(u32::from_le_bytes(
        header[16..20].try_into().map_err(|_| E::Invalid)?,
    ));
    let catalog = u64::from_le_bytes(header[20..28].try_into().map_err(|_| E::Invalid)?);
    if catalog == 0 || catalog > section {
        return Err(E::Quota);
    }
    // A count the upload cannot hold is invalid before anything is summed.
    if count.checked_mul(40).is_none_or(|framing| framing > upload) {
        return Err(E::Invalid);
    }
    let mut need = restore_base_need(upload, unit)?;
    for spool in [catalog, count * 40] {
        need = need.checked_add(file_cost(spool, unit)?).ok_or(E::Quota)?;
    }
    let mut at = file.seek(SeekFrom::Current(
        i64::try_from(catalog).map_err(|_| E::Invalid)?,
    ))?;
    for _ in 0..count {
        check(cancel)?;
        let mut record = [0; 40];
        file.read_exact(&mut record)?;
        let length = u64::from_le_bytes(record[32..].try_into().map_err(|_| E::Invalid)?);
        if length > file_bytes {
            return Err(E::Quota);
        }
        need = need.checked_add(file_cost(length, unit)?).ok_or(E::Quota)?;
        at = at.checked_add(40 + length).ok_or(E::Invalid)?;
        if at > upload {
            return Err(E::Invalid);
        }
        file.seek(SeekFrom::Start(at))?;
    }
    Ok((count, need.min(i64::MAX as u64)))
}
// The fixed bundle header is 60 bytes; each original contributes 40 framing
// bytes in addition to its opaque payload. All lengths are u64: the format has
// no 32-bit size or offset field and therefore no 4 GiB boundary.
pub(crate) fn archive_framing(files: u64) -> Result<u64, E> {
    60u64
        .checked_add(files.checked_mul(40).ok_or(E::Quota)?)
        .ok_or(E::Quota)
}
/// Modeled peak creation cost: rounded catalog spool, inventory spool and
/// artifact, plus auxiliary-job allowance. Only the bounded catalog and the
/// inventory records are duplicated; original bytes are counted once. The
/// allocation unit comes from the actual work filesystem, without a 1 MiB cap.
pub(crate) fn archive_need(original_bytes: u64, originals: u64, unit: u64) -> Result<u64, E> {
    let catalog = lytronaut_storage::portable::MAX_BYTES as u64;
    let artifact = original_bytes
        .checked_add(archive_framing(originals)?)
        .and_then(|n| n.checked_add(catalog))
        .ok_or(E::Quota)?;
    let inventory = originals.checked_mul(40).ok_or(E::Quota)?;
    job_cost(unit)?
        .checked_add(file_cost(catalog, unit)?)
        .and_then(|n| n.checked_add(file_cost(inventory, unit).ok()?))
        .and_then(|n| n.checked_add(file_cost(artifact, unit).ok()?))
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or(E::Quota)
}
/// Work-volume bytes a restore of a `declared`-byte upload reserves while it
/// uploads, before its framing is known:
/// the upload itself, its extracted originals and logical spool (together at
/// most the upload again) and headroom of one catalog maximum for a SQLite
/// destination catalog. Any positive size whose need fits i64 is acceptable;
/// whether it can run is decided by free-space admission alone. Before it
/// extracts anything, the restore worker re-admits the modeled need from the
/// uploaded framing (`restore_disk_need`), which adds rounded allocation and
/// metadata allowances; tiny originals cannot spend disk the upload-time reservation did not
/// cover.
pub(crate) fn restore_need(declared: u64) -> Result<u64, E> {
    (declared > 0)
        .then_some(declared)
        .and_then(|n| n.checked_mul(2))
        .and_then(|n| n.checked_add(lytronaut_storage::portable::MAX_BYTES as u64))
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or(E::Quota)
}
pub type Schema = Vec<(String, String, String)>;
#[derive(Debug, serde::Serialize)]
pub struct BundleReport {
    pub files: u64,
    pub bytes: u64,
}
fn check(cancel: &AtomicBool) -> Result<(), E> {
    if cancel.load(Ordering::Acquire) {
        Err(E::Cancelled)
    } else {
        Ok(())
    }
}
fn new_file(p: &Path) -> Result<File, E> {
    Ok(OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(p)?)
}
fn hash_file(p: &Path) -> Result<([u8; 32], u64), E> {
    let mut file = File::open(p)?;
    let mut h = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
        bytes += n as u64;
    }
    Ok((h.finalize().into(), bytes))
}
async fn schema(c: &mut SqliteConnection) -> Result<Schema, E> {
    let rows=sqlx::query("SELECT type,name,sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type,name LIMIT 129").fetch_all(c).await?;
    if rows.len() > 128 {
        return Err(E::Invalid);
    }
    rows.into_iter()
        .map(|r| {
            let kind: String = r.try_get(0)?;
            let name: String = r.try_get(1)?;
            let sql: String = r.try_get(2)?;
            if !matches!(kind.as_str(), "table" | "index")
                || sql.len() > 65536
                || sql.to_ascii_uppercase().contains("VIRTUAL")
            {
                return Err(E::Invalid);
            }
            Ok((kind, name, sql))
        })
        .collect()
}
pub async fn trusted_schema(catalog: &Catalog) -> Result<Schema, E> {
    if catalog.backend() == lytronaut_storage::Backend::Postgres {
        let sqlite = Catalog::open_application_database(Path::new(":memory:")).await?;
        fresh_schema(&sqlite).await?;
        let mut connection = sqlite.sqlite_snapshot_connection().await?;
        return schema(&mut connection).await;
    }
    let mut c = catalog.sqlite_snapshot_connection().await?;
    schema(&mut c).await
}
/// Read-only, query-only, trusted-schema-off typed cursor connection with
/// `row_buffer_size(1)`. No validation work budget: see `readonly`.
async fn readonly_cursor(path: &Path) -> Result<SqliteConnection, E> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .pragma("query_only", "ON")
            .pragma("trusted_schema", "OFF")
            .pragma("mmap_size", "0")
            .pragma("cache_size", "-2048")
            .pragma("temp_store", "MEMORY")
            .row_buffer_size(1),
    )
    .await?)
}
/// Validation connection for an UNTRUSTED inbound legacy database: bounded VM
/// steps and wall-clock time from open. Extraction after validation must not
/// use this connection, because its deadline also counts time spent awaiting
/// destination writes and would narrow legacy acceptance below the 32 MiB file.
async fn readonly(path: &Path) -> Result<SqliteConnection, E> {
    let mut c = readonly_cursor(path).await?;
    let mut handle = c.lock_handle().await?;
    handle.set_progress_handler(1000, {
        let mut steps = 0;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        move || {
            steps += 1000;
            steps < 20_000_000 && std::time::Instant::now() < deadline
        }
    });
    drop(handle);
    Ok(c)
}
/// Snapshot removes credentials, sessions and transient jobs; semantic/source records survive.
///
/// `budget` is the job's reserved work-volume bytes. The library can grow
/// between admission and capture (imports finishing meanwhile), so the
/// capture spools are checked at their modeled cost after capture completes.
/// They can transiently exceed this reservation: the logical spool is bounded
/// by MAX_BYTES and inventory by captured source rows (at most MAX_ROWS * 40).
/// Refusal cleans both spools before any artifact is created. Once admitted,
/// artifact growth is charged before each write, with retryable `Space`.
pub async fn backup(
    catalog: &Catalog,
    store: &Store,
    output: &Path,
    budget: u64,
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<BundleReport, E> {
    use lytronaut_storage::portable_database::{INVENTORY_RECORD, digest_hex, parse_record};
    check(cancel)?;
    let snapshot = output.with_extension("snapshot");
    let records = output.with_extension("inventory");
    if snapshot.exists() || records.exists() || output.exists() {
        return Err(E::Conflict);
    }
    let mut created = false;
    let result = async {
        // Catalog rows and the ascending original inventory come from one
        // snapshot and are both spooled to disk: memory does not grow with
        // the number or size of originals.
        let (spool, inventory_file, inventory) = lytronaut_storage::portable_database::capture(
            catalog,
            new_file(&snapshot)?,
            new_file(&records)?,
            cancel,
        )
        .await?;
        spool.sync_all()?;
        drop(spool);
        inventory_file.sync_all()?;
        drop(inventory_file);
        // The u32 header count is the format's only bound on the number of
        // unique originals; no deployment budget caps it.
        let count = u32::try_from(inventory.count).map_err(|_| E::Quota)?;
        let (catalog_hash, catalog_bytes) = hash_file(&snapshot)?;
        let (unit, _) = block_unit(output.parent().ok_or(E::Invalid)?)?;
        // Metadata-inclusive spool costs are checked after capture; artifact
        // writes below are precharged by their rounded growth, not raw bytes.
        let mut disk = DiskBudget::new(budget, job_cost(unit)?, unit);
        disk.charge(catalog_bytes)?;
        disk.charge(inventory.count * 40)?;
        let mut artifact_bytes = 60 + catalog_bytes;
        disk.charge(artifact_bytes)?;
        let mut out = new_file(output)?;
        created = true;
        out.write_all(PORTABLE_MAGIC)?;
        out.write_all(&count.to_le_bytes())?;
        out.write_all(&catalog_bytes.to_le_bytes())?;
        out.write_all(&catalog_hash)?;
        copy_checked(
            &mut File::open(&snapshot)?,
            &mut out,
            catalog_bytes,
            &catalog_hash,
            cancel,
            progress,
        )?;
        let mut bytes = catalog_bytes;
        let mut inventory_records = File::open(&records)?;
        for _ in 0..count {
            check(cancel)?;
            let mut record = [0; INVENTORY_RECORD];
            inventory_records.read_exact(&mut record)?;
            let (hash, length) = parse_record(&record);
            let next = artifact_bytes
                .checked_add(40)
                .and_then(|n| n.checked_add(length))
                .ok_or(E::Quota)?;
            disk.grow_file(artifact_bytes, next)?;
            artifact_bytes = next;
            let source = lytronaut_storage::SourceEntry {
                logical_path: String::new(),
                digest: digest_hex(&hash),
                length,
            };
            out.write_all(&record)?;
            copy_checked(
                &mut store.open_blob(&source)?,
                &mut out,
                length,
                &hash,
                cancel,
                progress,
            )?;
            // No library-size ceiling: the single-upload budget bounds one
            // browser import, not the whole library (lytronaut-lbk).
            bytes = bytes.checked_add(length).ok_or(E::Quota)?;
        }
        check(cancel)?;
        out.sync_all()?;
        File::open(output.parent().ok_or(E::Invalid)?)?.sync_all()?;
        Ok(BundleReport {
            files: inventory.count,
            bytes,
        })
    }
    .await;
    let _ = fs::remove_file(snapshot);
    let _ = fs::remove_file(records);
    if result.is_err() && created {
        let _ = fs::remove_file(output);
    }
    result
}
fn copy_checked(
    input: &mut File,
    out: &mut File,
    length: u64,
    expected: &[u8; 32],
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<(), E> {
    let mut h = Sha256::new();
    let mut remaining = length;
    let mut buffer = [0; COPY_BUFFER];
    while remaining > 0 {
        check(cancel)?;
        let n = remaining.min(buffer.len() as u64) as usize;
        input.read_exact(&mut buffer[..n])?;
        out.write_all(&buffer[..n])?;
        h.update(&buffer[..n]);
        remaining -= n as u64;
        progress.fetch_add(n as u64, Ordering::Relaxed);
    }
    if h.finalize().as_slice() != expected {
        return Err(E::Conflict);
    }
    Ok(())
}
struct OwnedRoot(PathBuf);
impl Drop for OwnedRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// Never opens an inbound database writable or executes its schema. Exact trusted app schema only.
/// Target must not exist. A completed copy is atomically made visible at target/store.
async fn restore_legacy(
    input: &Path,
    target: &Path,
    expected: &Schema,
    limits: &Limits,
    disk: &mut DiskBudget,
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<BundleReport, E> {
    check(cancel)?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(target)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                E::Conflict
            } else {
                E::Unavailable
            }
        })?;
    let mut guard = OwnedRoot(target.to_owned());
    let pending = target.join("pending");
    fs::DirBuilder::new().mode(0o700).create(&pending)?;
    let originals = pending.join("originals");
    fs::DirBuilder::new().mode(0o700).create(&originals)?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(pending.join("staging"))?;
    // No whole-bundle size cap: the 32 MiB catalog file and each original's
    // per-file limit are checked below; the originals' total is unbounded.
    let mut f = File::open(input)?;
    let mut header = [0; 60];
    f.read_exact(&mut header)?;
    if &header[..16] != MAGIC {
        return Err(E::Invalid);
    }
    let count = u32::from_le_bytes(header[16..20].try_into().unwrap());
    let dbsize = u64::from_le_bytes(header[20..28].try_into().unwrap());
    if dbsize == 0 || dbsize > MAX_DB {
        return Err(E::Quota);
    }
    // Each unique original needs at least its 64-character digest in the
    // version-1 catalog file, so that file (not a deployment budget) bounds
    // the count and the in-memory inventory below.
    if u64::from(count) > dbsize / 64 {
        return Err(E::Invalid);
    }
    let db = pending.join("catalog.db");
    disk.charge(dbsize)?;
    let mut dbout = new_file(&db)?;
    copy_checked(
        &mut f,
        &mut dbout,
        dbsize,
        header[28..60].try_into().unwrap(),
        cancel,
        progress,
    )?;
    dbout.sync_all()?;
    drop(dbout);
    let mut inventory = BTreeMap::new();
    let mut bytes = dbsize;
    for _ in 0..count {
        let mut record = [0; 40];
        f.read_exact(&mut record)?;
        let hash: [u8; 32] = record[..32].try_into().unwrap();
        let digest = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let length = u64::from_le_bytes(record[32..].try_into().unwrap());
        bytes = bytes.checked_add(length).ok_or(E::Quota)?;
        if length > limits.file_bytes || inventory.insert((digest.clone(), length), ()).is_some() {
            return Err(E::Quota);
        }
        disk.charge(length)?;
        let blob = originals.join(format!("{digest}-{length}"));
        let mut out = new_file(&blob)?;
        copy_checked(&mut f, &mut out, length, &hash, cancel, progress)?;
        out.sync_all()?;
        fs::set_permissions(blob, fs::Permissions::from_mode(0o400))?;
    }
    let mut tail = [0];
    if f.read(&mut tail)? != 0 {
        return Err(E::Invalid);
    }
    check(cancel)?;
    let mut c = readonly(&db).await?;
    // Bound all schema strings before extracting them.
    let n: i64 =
        sqlx::query_scalar("SELECT COALESCE(sum(length(CAST(sql AS BLOB))),0) FROM sqlite_master")
            .fetch_one(&mut c)
            .await?;
    if n > 4 * 1024 * 1024 {
        return Err(E::Quota);
    }
    let actual = schema(&mut c).await?;
    let pre_order: Schema = expected
        .iter()
        .filter(|(_, name, _)| name != "application_job_order")
        .cloned()
        .collect();
    let with_auth = |mut shape: Schema| {
        for (name, sql) in LEGACY_AUTH {
            shape.push(("table".into(), name.into(), sql.into()));
        }
        shape.sort();
        shape
    };
    let (has_order, legacy_auth) = if actual == *expected {
        (true, false)
    } else if actual == pre_order {
        (false, false)
    } else if actual == with_auth(expected.clone()) {
        (true, true)
    } else if actual == with_auth(pre_order) {
        (false, true)
    } else {
        return Err(E::Invalid);
    };
    let ok: String = sqlx::query_scalar("PRAGMA quick_check(1)")
        .fetch_one(&mut c)
        .await?;
    if ok != "ok" {
        return Err(E::Invalid);
    }
    if sqlx::query("PRAGMA foreign_key_check")
        .fetch_optional(&mut c)
        .await?
        .is_some()
    {
        return Err(E::Conflict);
    }
    for table in ["export_jobs"] {
        let query = format!("SELECT count(*) FROM {table}");
        let n: i64 = sqlx::query_scalar(AssertSqlSafe(query.as_str()))
            .fetch_one(&mut c)
            .await?;
        if n != 0 {
            return Err(E::Invalid);
        }
    }
    let pending_jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM intake_jobs WHERE state!='succeeded'")
            .fetch_one(&mut c)
            .await?;
    if pending_jobs != 0 {
        return Err(E::Invalid);
    }
    if has_order {
        // Only durable successful intake ordering belongs to this snapshot. Generated
        // snapshots scrub transient jobs and their order records together.
        let orphan_orders: i64 = sqlx::query_scalar("SELECT count(*) FROM application_job_order o LEFT JOIN intake_jobs j ON o.job=j.id WHERE o.kind!='intake' OR j.id IS NULL OR j.state!='succeeded'")
            .fetch_one(&mut c).await?;
        if orphan_orders != 0 {
            return Err(E::Invalid);
        }
        let missing_orders: i64 = sqlx::query_scalar("SELECT count(*) FROM intake_jobs j WHERE NOT EXISTS(SELECT 1 FROM application_job_order o WHERE o.kind='intake' AND o.job=j.id)")
            .fetch_one(&mut c).await?;
        if missing_orders != 0 {
            return Err(E::Conflict);
        }
    }
    let rows =
        sqlx::query("SELECT digest,length FROM intake_sources GROUP BY digest,length LIMIT ?")
            .bind(i64::from(count) + 1)
            .fetch_all(&mut c)
            .await?;
    if rows.len() != inventory.len() {
        return Err(E::Conflict);
    }
    for r in rows {
        let key = (
            r.try_get::<String, _>(0)?,
            u64::try_from(r.try_get::<i64, _>(1)?).map_err(|_| E::Invalid)?,
        );
        if !inventory.contains_key(&key) {
            return Err(E::Conflict);
        }
    }
    let mismatch:i64=sqlx::query_scalar("SELECT count(*) FROM catalog_sources s LEFT JOIN intake_sources i ON s.job=i.job AND s.path=i.path WHERE i.job IS NULL OR s.digest!=i.digest OR s.length!=i.length").fetch_one(&mut c).await?;
    if mismatch != 0 {
        return Err(E::Conflict);
    }
    let mismatch:i64=sqlx::query_scalar("SELECT count(*) FROM catalog_versions v JOIN intake_jobs j ON v.source_job=j.id WHERE j.state!='succeeded'").fetch_one(&mut c).await?;
    if mismatch != 0 {
        return Err(E::Conflict);
    }
    c.close().await?;
    if legacy_auth {
        // Only exact known schema reached this point, including checks on every row
        // association and immutable blob. Scrub credential state in the isolated copy.
        let mut c = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&db)
                .pragma("trusted_schema", "OFF"),
        )
        .await?;
        sqlx::raw_sql("PRAGMA secure_delete=ON; DROP TABLE auth_sessions; DROP TABLE auth_attempts; DROP TABLE auth_account; VACUUM;")
            .execute(&mut c).await?;
        c.close().await?;
    }
    check(cancel)?;
    File::open(&originals)?.sync_all()?;
    File::open(&pending)?.sync_all()?;
    fs::rename(pending, target.join("store"))?;
    File::open(target)?.sync_all()?;
    // Retain only the target we exclusively created. Never clean unrelated paths.
    guard.0 = PathBuf::new();
    Ok(BundleReport {
        files: u64::from(count),
        bytes,
    })
}

async fn fresh_schema(catalog: &Catalog) -> Result<(), E> {
    use lytronaut_storage::SchemaComponent::{Api, Catalog as Records, Export, Intake};
    let mut tx = catalog.begin_write().await?;
    for component in [Intake, Records, Api, Export] {
        for &statement in catalog.schema_statements(component) {
            sqlx::query(statement).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
fn bundle_magic(input: &Path) -> Result<[u8; 16], E> {
    let mut magic = [0; 16];
    File::open(input)?.read_exact(&mut magic)?;
    Ok(magic)
}
async fn sqlite_destination(directory: &Path) -> Result<Catalog, E> {
    let db = directory.join("catalog.db");
    crate::local::database_file(&db).map_err(|_| E::Unavailable)?;
    let catalog = Catalog::open_application_database(&db).await?;
    fresh_schema(&catalog).await?;
    Ok(catalog)
}
fn directory(path: &Path) -> Result<(), E> {
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}
fn publish(source: &Path, target: &Path) -> Result<(), E> {
    // Safe syscall wrapper provides NOREPLACE on Linux and macOS. A same-name
    // empty or populated unrelated directory is never overwritten by rename.
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        target,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(|_| E::Conflict)?;
    File::open(target.parent().ok_or(E::Invalid)?)?.sync_all()?;
    Ok(())
}
async fn validate_logical(
    catalog: &Catalog,
    inventory: &Path,
    cancel: &AtomicBool,
) -> Result<(), E> {
    for statement in [
        "SELECT count(*) FROM intake_jobs WHERE state!='succeeded'",
        "SELECT count(*) FROM catalog_versions v LEFT JOIN intake_jobs j ON v.source_job=j.id WHERE j.id IS NULL OR j.state!='succeeded'",
        "SELECT count(*) FROM catalog_sources s LEFT JOIN intake_sources i ON s.job=i.job AND s.path=i.path WHERE i.job IS NULL OR s.digest!=i.digest OR s.length!=i.length",
        "SELECT count(*) FROM application_job_order o LEFT JOIN intake_jobs j ON o.job=j.id WHERE o.kind!='intake' OR j.id IS NULL OR j.state!='succeeded'",
        "SELECT count(*) FROM intake_jobs j WHERE NOT EXISTS(SELECT 1 FROM application_job_order o WHERE o.kind='intake' AND o.job=j.id)",
    ] {
        let count: i64 = sqlx::query_scalar(statement)
            .fetch_one(catalog.pool())
            .await?;
        if count != 0 {
            return Err(E::Conflict);
        }
    }
    // Both sides are in ascending order: a streamed comparison, constant
    // memory for any number of originals.
    lytronaut_storage::portable_database::verify_inventory(
        catalog,
        std::io::BufReader::new(File::open(inventory)?),
        cancel,
    )
    .await
}
async fn restore_portable(
    input: &Path,
    directory: &Path,
    catalog: &Catalog,
    limits: &Limits,
    disk: &mut DiskBudget,
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<BundleReport, E> {
    check(cancel)?;
    // No whole-bundle size cap (lytronaut-lbk): the catalog section and each
    // original are bounded below; the total is limited only by admission.
    let mut input = File::open(input)?;
    let mut header = [0; 60];
    input.read_exact(&mut header)?;
    if &header[..16] != PORTABLE_MAGIC {
        return Err(E::Invalid);
    }
    let count = u32::from_le_bytes(header[16..20].try_into().map_err(|_| E::Invalid)?);
    let catalog_bytes = u64::from_le_bytes(header[20..28].try_into().map_err(|_| E::Invalid)?);
    // The u32 count is the format's own bound; no deployment budget caps the
    // number of originals a whole-library restore may carry.
    if catalog_bytes == 0 || catalog_bytes > lytronaut_storage::portable::MAX_BYTES as u64 {
        return Err(E::Quota);
    }
    // Every file is charged at its block cost before it is written, so a
    // bundle of many tiny originals cannot pass the job's reservation.
    disk.charge(catalog_bytes)?;
    disk.charge(u64::from(count) * 40)?;
    let spool = directory.join("logical.snapshot");
    let mut output = new_file(&spool)?;
    copy_checked(
        &mut input,
        &mut output,
        catalog_bytes,
        header[28..60].try_into().map_err(|_| E::Invalid)?,
        cancel,
        progress,
    )?;
    output.sync_all()?;
    drop(output);
    let originals = directory.join("originals");
    directory_create(&originals)?;
    directory_create(&directory.join("staging"))?;
    // Records must be strictly ascending (as every bundle writer emits them):
    // duplicates and disorder are rejected remembering only the previous one,
    // and the records are spooled for the streamed catalog comparison.
    use lytronaut_storage::portable_database::{
        Ascending, INVENTORY_RECORD, digest_hex, parse_record,
    };
    let inventory = directory.join("inventory.spool");
    let mut records = new_file(&inventory)?;
    let mut order = Ascending::default();
    let mut bytes = catalog_bytes;
    for _ in 0..count {
        check(cancel)?;
        let mut record = [0; INVENTORY_RECORD];
        input.read_exact(&mut record)?;
        let (hash, length) = parse_record(&record);
        bytes = bytes.checked_add(length).ok_or(E::Quota)?;
        if length > limits.file_bytes {
            return Err(E::Quota);
        }
        order.push(hash, length)?;
        disk.charge(length)?;
        records.write_all(&record)?;
        let digest = digest_hex(&hash);
        let path = originals.join(format!("{digest}-{length}"));
        let mut blob = new_file(&path)?;
        copy_checked(&mut input, &mut blob, length, &hash, cancel, progress)?;
        blob.sync_all()?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    }
    let mut tail = [0];
    if input.read(&mut tail)? != 0 {
        return Err(E::Invalid);
    }
    records.sync_all()?;
    drop(records);
    lytronaut_storage::portable_database::import(
        catalog,
        File::open(&spool)?,
        catalog_bytes as usize,
        cancel,
    )
    .await?;
    validate_logical(catalog, &inventory, cancel).await?;
    fs::remove_file(spool)?;
    fs::remove_file(inventory)?;
    File::open(&originals)?.sync_all()?;
    File::open(directory)?.sync_all()?;
    Ok(BundleReport {
        files: u64::from(count),
        bytes,
    })
}
fn directory_create(path: &Path) -> Result<(), E> {
    directory(path)
}
/// Backwards-compatible standalone SQLite restore; server jobs use restore_owned.
pub async fn restore(
    input: &Path,
    target: &Path,
    expected: &Schema,
    limits: &Limits,
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<BundleReport, E> {
    if bundle_magic(input)? == *MAGIC {
        let mut disk = DiskBudget::unlimited();
        return restore_legacy(input, target, expected, limits, &mut disk, cancel, progress).await;
    }
    directory(target)?;
    let mut guard = OwnedRoot(target.to_owned());
    let pending = target.join("pending");
    directory(&pending)?;
    let catalog = sqlite_destination(&pending).await?;
    let mut disk = DiskBudget::unlimited();
    let report = restore_portable(
        input, &pending, &catalog, limits, &mut disk, cancel, progress,
    )
    .await?;
    catalog.pool().close().await;
    check(cancel)?;
    publish(&pending, &target.join("store"))?;
    guard.0 = PathBuf::new();
    Ok(report)
}
/// Durable ownership identity of one restore job: the private results root its
/// journal lives under, the owning job id and the quota it has reserved.
#[derive(Clone, Copy)]
pub struct RestoreOwner<'a> {
    pub root: &'a Path,
    pub job: &'a str,
    pub reserved: u64,
}
/// Production isolated restore on the deployment-selected backend. The durable
/// journal survives EVERY error, including prepare returning an error before a
/// Journal value can be returned to the caller. Finalizer reloads it by job.
pub async fn restore_owned(
    input: &Path,
    active: &Catalog,
    owner: RestoreOwner<'_>,
    limits: &Limits,
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<BundleReport, E> {
    use crate::restore_journal::{Journal, Phase};
    let RestoreOwner {
        root,
        job,
        reserved,
    } = owner;
    let namespace = if active.backend() == lytronaut_storage::Backend::Postgres {
        let mut nonce = [0; 32];
        getrandom::fill(&mut nonce).map_err(|_| E::Unavailable)?;
        let nonce = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
        Some(lytronaut_storage::restore_namespace::RestoreNamespace::new(
            job, &nonce,
        )?)
    } else {
        None
    };
    // The reservation covers the upload already on disk, SQLite headroom and
    // a few job files; every extracted file is charged against the rest.
    let (unit, _) = block_unit(root)?;
    let mut disk = DiskBudget::new(
        reserved,
        restore_base_need(fs::metadata(input)?.len(), unit)?,
        unit,
    );
    let mut journal = Journal::prepare(root, job, namespace, reserved)?;
    let pending = journal.pending(root);
    let store = pending.join("store");
    directory(&store)?;
    let catalog = if let Some(namespace) = journal.namespace() {
        namespace.create(active).await?;
        // Test-only crash cut (no-op otherwise): namespace exists, journal Prepared.
        crate::restore_journal::fault("namespace-created")?;
        let catalog = active
            .open_restore_namespace(namespace)
            .await
            .map_err(|_| E::Unavailable)?;
        fresh_schema(&catalog).await?;
        journal.transition(root, Phase::SchemaOwned)?;
        catalog
    } else {
        sqlite_destination(&store).await?
    };
    let result = if bundle_magic(input)? == *MAGIC {
        let expected = trusted_schema(active).await?;
        let legacy = pending.join("legacy");
        let report = restore_legacy(
            input, &legacy, &expected, limits, &mut disk, cancel, progress,
        )
        .await?;
        let old_store = legacy.join("store");
        // The known old validator (budgeted) proved exact trusted schema, integrity,
        // records, references and inventory. Extraction is linear cursors over that
        // trusted schema in a <=32 MiB file, cancellable per row, so it carries no
        // separate time budget that destination write latency could exhaust.
        let mut source = readonly_cursor(&old_store.join("catalog.db")).await?;
        let has_order: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='application_job_order'").fetch_one(&mut source).await?;
        lytronaut_storage::portable_database::import_legacy(
            &catalog,
            &mut source,
            has_order == 1,
            cancel,
        )
        .await?;
        source.close().await?;
        publish(&old_store.join("originals"), &store.join("originals"))?;
        publish(&old_store.join("staging"), &store.join("staging"))?;
        fs::remove_dir_all(legacy)?;
        report
    } else {
        restore_portable(input, &store, &catalog, limits, &mut disk, cancel, progress).await?
    };
    journal.transition(root, Phase::RowsCommitted)?;
    catalog.pool().close().await;
    let descriptor = serde_json::json!({"version":1,"backend":if journal.namespace().is_some(){"postgres"}else{"sqlite"},"schema":journal.namespace().map(|n| n.schema())});
    let mut file = new_file(&store.join("restore.json"))?;
    file.write_all(&serde_json::to_vec(&descriptor).map_err(|_| E::Invalid)?)?;
    file.sync_all()?;
    File::open(&store)?.sync_all()?;
    File::open(&pending)?.sync_all()?;
    check(cancel)?;
    journal.transition(root, Phase::FilesReady)?;
    crate::restore_journal::fault("files-ready")?;
    publish(&pending, &journal.result(root))?;
    journal.transition(root, Phase::FilesPublished)?;
    crate::restore_journal::fault("files-published")?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capped_copy_checks_each_chunk_and_preserves_conflicts() {
        let root =
            std::env::temp_dir().join(format!("lytronaut-bundle-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let bytes = vec![13u8; 1024 * 1024];
        let input = root.join("input");
        fs::write(&input, &bytes).unwrap();
        let output = root.join("output");
        let hash: [u8; 32] = Sha256::digest(&bytes).into();
        let cancel = AtomicBool::new(false);
        let progress = std::sync::atomic::AtomicU64::new(0);
        copy_checked(
            &mut File::open(&input).unwrap(),
            &mut new_file(&output).unwrap(),
            bytes.len() as u64,
            &hash,
            &cancel,
            &progress,
        )
        .unwrap();
        assert_eq!(fs::read(&output).unwrap(), bytes);
        assert_eq!(progress.load(Ordering::Relaxed), bytes.len() as u64);
        assert!(new_file(&output).is_err());
        assert_eq!(fs::read(&output).unwrap(), bytes);
        let bad = root.join("bad");
        assert!(matches!(
            copy_checked(
                &mut File::open(&input).unwrap(),
                &mut new_file(&bad).unwrap(),
                bytes.len() as u64,
                &[0; 32],
                &cancel,
                &progress
            ),
            Err(E::Conflict)
        ));
        cancel.store(true, Ordering::Release);
        let cancelled = root.join("cancelled");
        assert!(matches!(
            copy_checked(
                &mut File::open(&input).unwrap(),
                &mut new_file(&cancelled).unwrap(),
                bytes.len() as u64,
                &hash,
                &cancel,
                &progress
            ),
            Err(E::Cancelled)
        ));
        assert_eq!(fs::metadata(cancelled).unwrap().len(), 0);
        for value in ["", "../secret", &"é".repeat(32), &"g".repeat(64)] {
            assert!(lytronaut_storage::portable_database::digest_bytes(value).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod portable_admission_tests {
    use super::*;

    #[test]
    fn accounting_file_cost_covers_metadata_beyond_payload_blocks() {
        assert_eq!(file_cost(0, 4096).unwrap(), 4096 + 1024);
        assert_eq!(file_cost(1, 4096).unwrap(), 4096 + 1024);
        assert_eq!(file_cost(4096, 4096).unwrap(), 4096 + 1024);
        assert_eq!(file_cost(4097, 4096).unwrap(), 8192 + 1024);
    }

    #[test]
    fn accounting_writer_refuses_when_only_the_payload_block_fits() {
        let mut budget = DiskBudget::new(4096, 0, 4096);
        assert!(matches!(
            budget.charge(1),
            Err(E::Space {
                required_bytes: 5120,
                available_bytes: 4096
            })
        ));
    }

    #[test]
    fn accounting_archive_uses_actual_large_allocation_units() {
        let unit = 8 * 1024 * 1024;
        let catalog = lytronaut_storage::portable::MAX_BYTES as u64;
        // Catalog is aligned; artifact needs one more block, inventory one;
        // three output files and sixteen job files each need metadata.
        assert_eq!(
            archive_need(0, 0, unit).unwrap(),
            2 * catalog + (JOB_FILES + 2) * unit + (JOB_FILES + 3) * 1024
        );
    }

    #[test]
    fn accounting_file_growth_pays_metadata_once_and_refuses_overflow() {
        let mut disk = DiskBudget::new(9216, 0, 4096);
        disk.charge(1).unwrap();
        disk.grow_file(1, 4096).unwrap();
        assert_eq!(disk.used, 5120);
        disk.grow_file(4096, 4097).unwrap();
        assert_eq!(disk.used, 9216);
        assert!(matches!(disk.grow_file(4097, 8193), Err(E::Space { .. })));
        assert!(matches!(disk.grow_file(2, 1), Err(E::Invalid)));
        assert!(matches!(file_cost(0, 0), Err(E::Quota)));
        assert!(matches!(file_cost(u64::MAX, 1), Err(E::Quota)));
    }

    #[test]
    fn accounting_inode_floor_includes_auxiliary_files_and_unknown_filesystems() {
        let required = 7 + JOB_FILES + INODE_FLOOR;
        assert!(check_restore_inodes(7, Some(required)).is_ok());
        assert!(matches!(
            check_restore_inodes(7, Some(required - 1)),
            Err(E::Inodes)
        ));
        assert!(check_restore_inodes(7, None).is_ok());
        assert!(matches!(
            check_restore_inodes(u64::MAX, Some(u64::MAX)),
            Err(E::Inodes)
        ));
    }

    #[test]
    fn needs_follow_real_bytes_and_ignore_upload_budgets() {
        let catalog = lytronaut_storage::portable::MAX_BYTES as u64;
        assert_eq!(archive_framing(0).unwrap(), 60);
        assert_eq!(archive_framing(2).unwrap(), 140);
        assert!(matches!(archive_framing(u64::MAX), Err(E::Quota)));
        // Creation: originals once, framing, the bounded catalog twice. A
        // library far beyond any single-upload budget is just a bigger number.
        let tib = 1u64 << 40;
        assert_eq!(
            archive_need(0, 0, 4096).unwrap(),
            2 * catalog + (JOB_FILES + 2) * 4096 + (JOB_FILES + 3) * FILE_METADATA
        );
        assert_eq!(
            archive_need(5 * tib, 3, 4096).unwrap(),
            5 * tib + 2 * catalog + (JOB_FILES + 2) * 4096 + (JOB_FILES + 3) * FILE_METADATA
        );
        assert!(matches!(
            archive_need(i64::MAX as u64, 0, 4096),
            Err(E::Quota)
        ));
        assert!(matches!(archive_need(u64::MAX, 1, 4096), Err(E::Quota)));
        assert!(matches!(archive_need(0, u64::MAX, 4096), Err(E::Quota)));
        // Restore: upload plus extracted copy plus one catalog of headroom.
        assert_eq!(restore_need(1).unwrap(), 2 + catalog);
        assert_eq!(restore_need(5 * tib).unwrap(), 10 * tib + catalog);
        assert!(matches!(restore_need(0), Err(E::Quota)));
        assert!(matches!(restore_need(i64::MAX as u64 / 2), Err(E::Quota)));
        assert!(matches!(restore_need(u64::MAX), Err(E::Quota)));
        assert_eq!(MAX_DB, 32 * 1024 * 1024);
        assert_eq!(COPY_BUFFER, 64 * 1024);
    }
}

#[cfg(test)]
#[path = "portable_bundle_tests.rs"]
pub(crate) mod portable_bundle_tests;
