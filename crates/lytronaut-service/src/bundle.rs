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
#[cfg(test)]
static LARGEST_COPY_CHUNK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Test seam: the largest chunk any bundle transfer in this process moved.
#[cfg(test)]
pub(crate) fn largest_copy_chunk() -> usize {
    LARGEST_COPY_CHUNK.load(Ordering::Relaxed)
}
// The fixed bundle header is 60 bytes; each original contributes 40 framing
// bytes in addition to its opaque payload. All lengths are u64: the format has
// no 32-bit size or offset field and therefore no 4 GiB boundary.
pub(crate) fn archive_framing(files: u64) -> Result<u64, E> {
    60u64
        .checked_add(files.checked_mul(40).ok_or(E::Quota)?)
        .ok_or(E::Quota)
}
/// Work-volume bytes a bundle creation holds at its peak: the logical spool
/// (at most the v2 catalog maximum) beside the growing artifact, which holds the
/// catalog copy, the framing and every unique original. Only the bounded
/// catalog is counted twice; originals never are.
pub(crate) fn archive_need(original_bytes: u64, originals: u64) -> Result<u64, E> {
    let catalog = lytronaut_storage::portable::MAX_BYTES as u64;
    original_bytes
        .checked_add(archive_framing(originals)?)
        .and_then(|n| n.checked_add(2 * catalog))
        .filter(|n| *n <= i64::MAX as u64)
        .ok_or(E::Quota)
}
/// Work-volume bytes a restore of a `declared`-byte upload holds at its peak:
/// the upload itself, its extracted originals and logical spool (together at
/// most the upload again) and headroom of one catalog maximum for a SQLite
/// destination catalog. Any positive size whose need fits i64 is acceptable;
/// whether it can run is decided by free-space admission alone.
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
pub async fn backup(
    catalog: &Catalog,
    store: &Store,
    output: &Path,
    cancel: &AtomicBool,
    progress: &std::sync::atomic::AtomicU64,
) -> Result<BundleReport, E> {
    check(cancel)?;
    let snapshot = output.with_extension("snapshot");
    if snapshot.exists() || output.exists() {
        return Err(E::Conflict);
    }
    let mut created = false;
    let result = async {
        let file = new_file(&snapshot)?;
        let (spool, inventory) = lytronaut_storage::portable_database::capture(
            catalog,
            file,
            store.limits().files,
            cancel,
        )
        .await?;
        spool.sync_all()?;
        drop(spool);
        let (catalog_hash, catalog_bytes) = hash_file(&snapshot)?;
        let mut out = new_file(output)?;
        created = true;
        out.write_all(PORTABLE_MAGIC)?;
        out.write_all(&(inventory.len() as u32).to_le_bytes())?;
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
        for source in &inventory {
            check(cancel)?;
            let hash = parse_hash(&source.digest)?;
            out.write_all(&hash)?;
            out.write_all(&source.length.to_le_bytes())?;
            copy_checked(
                &mut store.open_blob(source)?,
                &mut out,
                source.length,
                &hash,
                cancel,
                progress,
            )?;
            // No library-size ceiling: the single-upload budget bounds one
            // browser import, not the whole library (lytronaut-lbk).
            bytes = bytes.checked_add(source.length).ok_or(E::Quota)?;
        }
        check(cancel)?;
        out.sync_all()?;
        File::open(output.parent().ok_or(E::Invalid)?)?.sync_all()?;
        Ok(BundleReport {
            files: inventory.len() as u64,
            bytes,
        })
    }
    .await;
    let _ = fs::remove_file(snapshot);
    if result.is_err() && created {
        let _ = fs::remove_file(output);
    }
    result
}
fn parse_hash(s: &str) -> Result<[u8; 32], E> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(E::Invalid);
    }
    let mut b = [0; 32];
    for (i, v) in b.iter_mut().enumerate() {
        *v = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| E::Invalid)?;
    }
    Ok(b)
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
        #[cfg(test)]
        LARGEST_COPY_CHUNK.fetch_max(n, Ordering::Relaxed);
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
    if count > limits.files || dbsize == 0 || dbsize > MAX_DB {
        return Err(E::Quota);
    }
    let db = pending.join("catalog.db");
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
            .bind(i64::from(limits.files) + 1)
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
    inventory: &BTreeMap<(String, u64), ()>,
    limits: &Limits,
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
    let rows =
        sqlx::query("SELECT digest,length FROM intake_sources GROUP BY digest,length LIMIT $1")
            .bind(i64::from(limits.files) + 1)
            .fetch_all(catalog.pool())
            .await?;
    if rows.len() != inventory.len() {
        return Err(E::Conflict);
    }
    for row in rows {
        let key = (
            row.try_get::<String, _>(0)?,
            u64::try_from(row.try_get::<i64, _>(1)?).map_err(|_| E::Invalid)?,
        );
        if !inventory.contains_key(&key) {
            return Err(E::Conflict);
        }
    }
    Ok(())
}
async fn restore_portable(
    input: &Path,
    directory: &Path,
    catalog: &Catalog,
    limits: &Limits,
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
    if count > limits.files
        || catalog_bytes == 0
        || catalog_bytes > lytronaut_storage::portable::MAX_BYTES as u64
    {
        return Err(E::Quota);
    }
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
    let mut inventory = BTreeMap::new();
    let mut bytes = catalog_bytes;
    for _ in 0..count {
        check(cancel)?;
        let mut record = [0; 40];
        input.read_exact(&mut record)?;
        let hash: [u8; 32] = record[..32].try_into().map_err(|_| E::Invalid)?;
        let digest = hash.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let length = u64::from_le_bytes(record[32..].try_into().map_err(|_| E::Invalid)?);
        bytes = bytes.checked_add(length).ok_or(E::Quota)?;
        if length > limits.file_bytes || inventory.insert((digest.clone(), length), ()).is_some() {
            return Err(E::Quota);
        }
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
    lytronaut_storage::portable_database::import(
        catalog,
        File::open(&spool)?,
        catalog_bytes as usize,
        cancel,
    )
    .await?;
    validate_logical(catalog, &inventory, limits).await?;
    fs::remove_file(spool)?;
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
        return restore_legacy(input, target, expected, limits, cancel, progress).await;
    }
    directory(target)?;
    let mut guard = OwnedRoot(target.to_owned());
    let pending = target.join("pending");
    directory(&pending)?;
    let catalog = sqlite_destination(&pending).await?;
    let report = restore_portable(input, &pending, &catalog, limits, cancel, progress).await?;
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
        let report = restore_legacy(input, &legacy, &expected, limits, cancel, progress).await?;
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
        restore_portable(input, &store, &catalog, limits, cancel, progress).await?
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
            assert!(parse_hash(value).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod portable_admission_tests {
    use super::*;

    #[test]
    fn needs_follow_real_bytes_and_ignore_upload_budgets() {
        let catalog = lytronaut_storage::portable::MAX_BYTES as u64;
        assert_eq!(archive_framing(0).unwrap(), 60);
        assert_eq!(archive_framing(2).unwrap(), 140);
        assert!(matches!(archive_framing(u64::MAX), Err(E::Quota)));
        // Creation: originals once, framing, the bounded catalog twice. A
        // library far beyond any single-upload budget is just a bigger number.
        let tib = 1u64 << 40;
        assert_eq!(archive_need(0, 0).unwrap(), 60 + 2 * catalog);
        assert_eq!(
            archive_need(5 * tib, 3).unwrap(),
            5 * tib + 60 + 3 * 40 + 2 * catalog
        );
        assert!(matches!(archive_need(i64::MAX as u64, 0), Err(E::Quota)));
        assert!(matches!(archive_need(u64::MAX, 1), Err(E::Quota)));
        assert!(matches!(archive_need(0, u64::MAX), Err(E::Quota)));
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
