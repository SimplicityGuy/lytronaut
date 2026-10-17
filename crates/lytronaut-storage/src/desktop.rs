//! Offline Desktop schema-23 adapter. Original bytes remain owned by intake.
//! This module deliberately exposes no HTTP routes or authorization decisions.
use crate::{Catalog, SourceEntry, StorageError, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{AssertSqlSafe, Connection, Row, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

static IMPORTS: Semaphore = Semaphore::const_new(2);
const MAX_DB: u64 = 32 * 1024 * 1024;
const MAX_CELL: i64 = 1024 * 1024;
const MAX_RECORD: usize = 8 * 1024 * 1024;
const TABLES: &[(&str, &[&str])] = &[
    ("library_metadata", &["version"]),
    (
        "picture",
        &[
            "uuid",
            "imagebin_uuid",
            "event_uuid",
            "name",
            "caption",
            "rating",
            "flag_status",
        ],
    ),
    ("picture_metadata", &["uuid", "lfe_major", "lfe_minor"]),
    ("album", &["uuid", "name", "sort_order", "public"]),
    (
        "album_to_picture",
        &["album_uuid", "picture_uuid", "picture_index"],
    ),
    ("event", &["uuid", "library_uuid"]),
    ("image_bin", &["uuid"]),
    ("picture_list", &["uuid", "filter", "sort_order"]),
    ("imported_picture_history", &["hash"]),
];

/// Text is stored as exact bytes (including non-UTF8 SQLite text). Integers and
/// IEEE float bits never pass through JSON numbers. Debug redacts all source data.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypedValue {
    Null,
    Integer(String),
    Real(String),
    Text(Vec<u8>),
    Blob(Vec<u8>),
}
impl fmt::Debug for TypedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TypedValue([redacted])")
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalRecord {
    pub fields: BTreeMap<String, TypedValue>,
}
impl fmt::Debug for OriginalRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OriginalRecord([redacted])")
    }
}
impl OriginalRecord {
    fn text(&self, key: &str) -> Result<&str, CatalogError> {
        match self.fields.get(key) {
            Some(TypedValue::Text(v)) => std::str::from_utf8(v).map_err(|_| CatalogError::Invalid),
            _ => Err(CatalogError::Invalid),
        }
    }
    fn integer(&self, key: &str) -> Result<i64, CatalogError> {
        match self.fields.get(key) {
            Some(TypedValue::Integer(v)) => v.parse().map_err(|_| CatalogError::Invalid),
            _ => Err(CatalogError::Invalid),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CatalogState {
    Ready,
    PreservedOnly,
    Quarantined,
}
impl CatalogState {
    fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::PreservedOnly => "preserved_only",
            Self::Quarantined => "quarantined",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    IntakeIncomplete,
    Invalid,
    Unavailable,
}
impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "catalog {:?}", self)
    }
}
impl std::error::Error for CatalogError {}
impl From<sqlx::Error> for CatalogError {
    fn from(_: sqlx::Error) -> Self {
        Self::Unavailable
    }
}
impl From<io::Error> for CatalogError {
    fn from(_: io::Error) -> Self {
        Self::Unavailable
    }
}
impl From<StorageError> for CatalogError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Unavailable | StorageError::Interrupted | StorageError::Busy => {
                Self::Unavailable
            }
            _ => Self::IntakeIncomplete,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSummary {
    pub version_id: i64,
    pub state: CatalogState,
    pub files: usize,
    pub pictures: usize,
    pub metadata: usize,
    pub albums: usize,
    pub memberships: usize,
    pub bins: usize,
    pub events: usize,
    pub picture_lists: usize,
    pub item_errors: usize,
    pub conflicting_identity: bool,
    pub reason: Option<String>,
}

/// Additive migration; a failed DDL transaction rolls back without touching auth/intake.
pub async fn migrate(catalog: &Catalog) -> Result<(), CatalogError> {
    // Schema statements are writes even when an existing object makes them no-ops.
    // Reserve the writer before SQLite reads its schema to avoid BUSY upgrades.
    let mut tx = catalog.begin_write().await?;
    for &sql in catalog.schema_statements(crate::SchemaComponent::Catalog) {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    let invalid:i64=sqlx::query_scalar(catalog.dialect(
        "SELECT count(*) FROM sqlite_master WHERE name IN ('catalog_versions','catalog_sources','catalog_records','catalog_pictures','catalog_albums','catalog_memberships','catalog_item_errors') AND type<>'table'",
        "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=current_schema() AND c.relname IN ('catalog_versions','catalog_sources','catalog_records','catalog_pictures','catalog_albums','catalog_memberships','catalog_item_errors') AND c.relkind<>'r'"
    )).fetch_one(&mut *tx).await?;
    if invalid != 0 {
        return Err(CatalogError::Invalid);
    }
    tx.commit().await?;
    Ok(())
}

struct Snapshot {
    path: PathBuf,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn snapshot(
    store: Store,
    entry: SourceEntry,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Snapshot, CatalogError> {
    if entry.length > MAX_DB {
        return Err(CatalogError::Invalid);
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| CatalogError::Unavailable)?;
    // Catalog adaptation must stay inside the configured writable boundary, even
    // when the host/container root (including /tmp) is read-only.
    let path = store.staging_root().join(format!(
        "lytronaut-desktop-{:032x}.db",
        u128::from_le_bytes(random)
    ));
    let mut src = store.open_blob(&entry)?;
    let mut dst = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    // Own cleanup only after exclusive creation succeeds. Flat staging files are
    // also removed by Store recovery after an interrupted adaptation.
    let snapshot = Snapshot { path };
    copy_verified(&mut src, &mut dst, &entry, cancel)?;
    dst.sync_all()?;
    fs::set_permissions(&snapshot.path, fs::Permissions::from_mode(0o400))?;
    Ok(snapshot)
}
fn copy_verified(
    src: &mut impl io::Read,
    dst: &mut impl io::Write,
    entry: &SourceEntry,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), CatalogError> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        if let Some(cancel) = cancel {
            cancelled(cancel)?;
        }
        let n = src.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        bytes = bytes.checked_add(n as u64).ok_or(CatalogError::Invalid)?;
        if bytes > entry.length {
            return Err(CatalogError::Invalid);
        }
        hash.update(&buffer[..n]);
        dst.write_all(&buffer[..n])?;
    }
    if bytes != entry.length || format!("{:x}", hash.finalize()) != entry.digest {
        return Err(CatalogError::Invalid);
    }
    Ok(())
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
struct SourceTable {
    name: &'static str,
    names: Vec<String>,
    select: String,
}
struct Decoded {
    conn: SqliteConnection,
    tables: Vec<SourceTable>,
}
// The deadline applies to work while requesting a row, excluding downstream
// backpressure (including a remote PostgreSQL destination). VM work is bounded
// per scan; no counter is tied to the number of captures or table cells.
struct ReadBudget(std::sync::Arc<std::sync::Mutex<Instant>>);
impl ReadBudget {
    async fn arm(
        conn: &mut SqliteConnection,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, CatalogError> {
        let deadline = std::sync::Arc::new(std::sync::Mutex::new(
            Instant::now() + Duration::from_secs(10),
        ));
        let shared = deadline.clone();
        let mut ticks = 0u32;
        conn.lock_handle()
            .await?
            .set_progress_handler(1000, move || {
                ticks += 1;
                !cancel.load(std::sync::atomic::Ordering::Relaxed)
                    && ticks <= 20_000
                    && shared
                        .lock()
                        .is_ok_and(|deadline| Instant::now() < *deadline)
            });
        Ok(Self(deadline))
    }
    fn next(&self) -> Result<(), CatalogError> {
        *self.0.lock().map_err(|_| CatalogError::Unavailable)? =
            Instant::now() + Duration::from_secs(10);
        Ok(())
    }
}
fn cancelled(cancel: &std::sync::atomic::AtomicBool) -> Result<(), CatalogError> {
    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
        Err(CatalogError::Unavailable)
    } else {
        Ok(())
    }
}
async fn decode(
    snapshot: &Snapshot,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Decoded, CatalogError> {
    let options = SqliteConnectOptions::new()
        .filename(&snapshot.path)
        .read_only(true)
        .immutable(true)
        .row_buffer_size(1)
        .pragma("query_only", "ON")
        .pragma("trusted_schema", "OFF")
        .pragma("ignore_check_constraints", "ON")
        .pragma("mmap_size", "0")
        .pragma("cache_size", "-2048")
        .pragma("temp_store", "MEMORY");
    let mut conn = SqliteConnection::connect_with(&options).await?;
    let _budget = ReadBudget::arm(&mut conn, cancel).await?;
    // Bound schema rows/bytes before retrieving strings. sqlite_master is built in.
    let schema = sqlx::query("SELECT count(*),max(length(CAST(sql AS BLOB))),sum(length(CAST(sql AS BLOB))),max(length(CAST(name AS BLOB))) FROM sqlite_master").fetch_one(&mut conn).await?;
    if schema.try_get::<i64, _>(0)? > 1024
        || schema.try_get::<Option<i64>, _>(1)?.unwrap_or(0) > 65536
        || schema.try_get::<Option<i64>, _>(2)?.unwrap_or(0) > 4 * 1024 * 1024
        || schema.try_get::<Option<i64>, _>(3)?.unwrap_or(0) > 256
    {
        return Err(CatalogError::Invalid);
    }
    let objects = sqlx::query("SELECT name,type,sql FROM sqlite_master LIMIT 1025")
        .fetch_all(&mut conn)
        .await?;
    if objects.len() > 1024 {
        return Err(CatalogError::Invalid);
    }
    for (name, _) in TABLES {
        let object = objects
            .iter()
            .find(|r| r.try_get::<String, _>("name").is_ok_and(|v| v == *name))
            .ok_or(CatalogError::Invalid)?;
        let definition: String = object.try_get("sql")?;
        if object.try_get::<String, _>("type")? != "table"
            || !definition
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE TABLE")
        {
            return Err(CatalogError::Invalid);
        }
    }
    for object in &objects {
        if object.try_get::<String, _>("type")? == "table" {
            let definition: String = object.try_get("sql")?;
            if !definition
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("CREATE TABLE")
            {
                return Err(CatalogError::Invalid);
            }
            let name: String = object.try_get("name")?;
            let columns = sqlx::query(AssertSqlSafe(format!(
                "PRAGMA table_xinfo({})",
                quote(&name)
            )))
            .fetch_all(&mut conn)
            .await?;
            if columns.len() > 128
                || columns
                    .iter()
                    .any(|r| r.try_get::<i64, _>("hidden").map_or(true, |v| v != 0))
            {
                return Err(CatalogError::Invalid);
            }
        }
    }
    let check: String = sqlx::query_scalar("PRAGMA quick_check(1)")
        .fetch_one(&mut conn)
        .await?;
    if check != "ok" {
        return Err(CatalogError::Invalid);
    }
    let mut tables = Vec::new();
    for &(table, required) in TABLES {
        let columns = sqlx::query(AssertSqlSafe(format!(
            "PRAGMA table_xinfo({})",
            quote(table)
        )))
        .fetch_all(&mut conn)
        .await?;
        if columns.is_empty()
            || columns.len() > 128
            || columns
                .iter()
                .any(|r| r.try_get::<i64, _>("hidden").map_or(true, |v| v != 0))
        {
            return Err(CatalogError::Invalid);
        }
        let names: Vec<String> = columns
            .iter()
            .map(|r| r.try_get("name"))
            .collect::<Result<_, _>>()?;
        if names.iter().any(|n| n.len() > 256)
            || required.iter().any(|r| !names.iter().any(|n| n == r))
        {
            return Err(CatalogError::Invalid);
        }
        let lengths: Vec<_> = names
            .iter()
            .map(|n| format!("length(CAST({} AS BLOB))", quote(n)))
            .collect();
        let mut fields: Vec<_> = lengths
            .iter()
            .map(|length| format!("max({length})"))
            .collect();
        // Every formerly admitted row had <=8MiB because the entire input did.
        // Retain that bound per row instead of rejecting the whole catalog.
        fields.push(format!(
            "max({})",
            lengths
                .iter()
                .map(|length| format!("coalesce({length},0)"))
                .collect::<Vec<_>>()
                .join("+")
        ));
        let sizes = sqlx::query(AssertSqlSafe(format!(
            "SELECT {} FROM {}",
            fields.join(","),
            quote(table)
        )))
        .fetch_one(&mut conn)
        .await?;
        for i in 0..names.len() {
            if !(0..=MAX_CELL).contains(&sizes.try_get::<Option<i64>, _>(i)?.unwrap_or(0)) {
                return Err(CatalogError::Invalid);
            }
        }
        if sizes.try_get::<Option<i64>, _>(names.len())?.unwrap_or(0) > MAX_RECORD as i64 {
            return Err(CatalogError::Invalid);
        }
        let fields = names
            .iter()
            .map(|n| {
                let q = quote(n);
                format!("typeof({q}),{q},CAST({q} AS BLOB)")
            })
            .collect::<Vec<_>>();
        tables.push(SourceTable {
            name: table,
            names,
            select: format!("SELECT {} FROM {}", fields.join(","), quote(table)),
        });
    }
    Ok(Decoded { conn, tables })
}
fn record(row: sqlx::sqlite::SqliteRow, names: &[String]) -> Result<OriginalRecord, CatalogError> {
    let mut fields = BTreeMap::new();
    for (i, name) in names.iter().enumerate() {
        let value = match row.try_get::<String, _>(i * 3)?.as_str() {
            "null" => TypedValue::Null,
            "integer" => TypedValue::Integer(row.try_get::<i64, _>(i * 3 + 1)?.to_string()),
            "real" => TypedValue::Real(format!(
                "{:016x}",
                row.try_get::<f64, _>(i * 3 + 1)?.to_bits()
            )),
            "text" => TypedValue::Text(row.try_get(i * 3 + 2)?),
            "blob" => TypedValue::Blob(row.try_get(i * 3 + 2)?),
            _ => return Err(CatalogError::Invalid),
        };
        fields.insert(name.clone(), value);
    }
    Ok(OriginalRecord { fields })
}
fn scratch_plan(source_bytes: u64, unit: u64) -> Result<(u64, u64), CatalogError> {
    if unit == 0 {
        return Err(CatalogError::Unavailable);
    }
    let pages = source_bytes
        .checked_mul(16)
        .and_then(|n| n.checked_add(4 * 1024 * 1024))
        .ok_or(CatalogError::Invalid)?
        .div_ceil(4096);
    let file_cost = |bytes: u64| {
        bytes
            .div_ceil(unit)
            .max(1)
            .checked_mul(unit)
            .and_then(|n| n.checked_add(1024))
            .ok_or(CatalogError::Invalid)
    };
    let index_bytes = pages.checked_mul(4096).ok_or(CatalogError::Invalid)?;
    let reserved = file_cost(source_bytes)?
        .checked_add(file_cost(index_bytes)?)
        .ok_or(CatalogError::Invalid)?;
    Ok((pages, reserved))
}
async fn scratch(path: &std::path::Path, pages: u64) -> Result<SqliteConnection, CatalogError> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .row_buffer_size(1)
            .pragma("journal_mode", "OFF")
            .pragma("synchronous", "OFF")
            .pragma("mmap_size", "0")
            .pragma("page_size", "4096")
            .pragma("cache_size", "-2048")
            .pragma("temp_store", "MEMORY")
            .pragma("max_page_count", pages.to_string()),
    )
    .await?)
}
async fn initialize_scratch(conn: &mut SqliteConnection) -> Result<(), CatalogError> {
    sqlx::raw_sql("CREATE TABLE identities(kind TEXT NOT NULL,value BLOB NOT NULL,PRIMARY KEY(kind,value)) WITHOUT ROWID;
CREATE TABLE ids(kind TEXT NOT NULL,value BLOB NOT NULL,PRIMARY KEY(kind,value)) WITHOUT ROWID;
CREATE TABLE pictures(uuid BLOB PRIMARY KEY,bin BLOB NOT NULL,event BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE events(uuid BLOB PRIMARY KEY,library BLOB NOT NULL) WITHOUT ROWID;
CREATE TABLE members(album BLOB NOT NULL,picture BLOB NOT NULL,idx INTEGER NOT NULL,PRIMARY KEY(album,picture),UNIQUE(album,idx)) WITHOUT ROWID;")
        .execute(conn).await?;
    Ok(())
}
async fn validate(
    decoded: &mut Decoded,
    scratch: &mut SqliteConnection,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<String, CatalogError> {
    use futures_util::TryStreamExt;
    let mut tx = scratch.begin().await?;
    let mut libraries = 0usize;
    for table in &decoded.tables {
        let budget = ReadBudget::arm(&mut decoded.conn, cancel.clone()).await?;
        let mut rows = sqlx::query(AssertSqlSafe(table.select.clone())).fetch(&mut decoded.conn);
        loop {
            cancelled(&cancel)?;
            budget.next()?;
            let Some(row) = rows.try_next().await? else {
                break;
            };
            let r = record(row, &table.names)?;
            if let Some(value) = r.fields.get("id") {
                sqlx::query("INSERT INTO ids VALUES($1,$2)")
                    .bind(table.name)
                    .bind(serde_json::to_vec(value).map_err(|_| CatalogError::Invalid)?)
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| CatalogError::Invalid)?;
            }
            if [
                "picture",
                "picture_metadata",
                "album",
                "image_bin",
                "event",
                "picture_list",
            ]
            .contains(&table.name)
            {
                let value = r.text("uuid")?;
                if value.is_empty() {
                    return Err(CatalogError::Invalid);
                }
                sqlx::query("INSERT INTO identities VALUES($1,$2)")
                    .bind(table.name)
                    .bind(value.as_bytes())
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| CatalogError::Invalid)?;
            }
            match table.name {
                "library_metadata" => {
                    libraries += 1;
                    if r.integer("version")? != 23 || libraries > 1 {
                        return Err(CatalogError::Invalid);
                    }
                }
                "picture" => {
                    r.text("name")?;
                    r.text("caption")?;
                    r.integer("rating")?;
                    r.integer("flag_status")?;
                    let id = r.text("uuid")?;
                    let bin = r.text("imagebin_uuid")?;
                    if [id, bin]
                        .iter()
                        .any(|s| s.contains('/') || s.contains('\\') || *s == "." || *s == "..")
                    {
                        return Err(CatalogError::Invalid);
                    }
                    sqlx::query("INSERT INTO pictures VALUES($1,$2,$3)")
                        .bind(id.as_bytes())
                        .bind(bin.as_bytes())
                        .bind(r.text("event_uuid")?.as_bytes())
                        .execute(&mut *tx)
                        .await?;
                }
                "album" => {
                    r.text("name")?;
                    r.integer("sort_order")?;
                }
                "event" => {
                    let library = r.text("library_uuid")?;
                    if library.is_empty() {
                        return Err(CatalogError::Invalid);
                    }
                    sqlx::query("INSERT INTO events VALUES($1,$2)")
                        .bind(r.text("uuid")?.as_bytes())
                        .bind(library.as_bytes())
                        .execute(&mut *tx)
                        .await?;
                }
                "album_to_picture" => {
                    let index = r.integer("picture_index")?;
                    if index < 0 {
                        return Err(CatalogError::Invalid);
                    }
                    sqlx::query("INSERT INTO members VALUES($1,$2,$3)")
                        .bind(r.text("album_uuid")?.as_bytes())
                        .bind(r.text("picture_uuid")?.as_bytes())
                        .bind(index)
                        .execute(&mut *tx)
                        .await
                        .map_err(|_| CatalogError::Invalid)?;
                }
                _ => {}
            }
        }
    }
    if libraries != 1 {
        return Err(CatalogError::Invalid);
    }
    let library: Option<Vec<u8>> = sqlx::query_scalar("SELECT library FROM events LIMIT 1")
        .fetch_optional(&mut *tx)
        .await?;
    let library = library.ok_or(CatalogError::Invalid)?;
    let mismatch: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM events WHERE library<>$1)")
        .bind(&library)
        .fetch_one(&mut *tx)
        .await?;
    if mismatch != 0 {
        return Err(CatalogError::Invalid);
    }
    for query in [
        "SELECT EXISTS(SELECT 1 FROM identities p WHERE p.kind='picture' AND NOT EXISTS(SELECT 1 FROM identities m WHERE m.kind='picture_metadata' AND m.value=p.value))",
        "SELECT EXISTS(SELECT 1 FROM identities m WHERE m.kind='picture_metadata' AND NOT EXISTS(SELECT 1 FROM identities p WHERE p.kind='picture' AND p.value=m.value))",
        "SELECT EXISTS(SELECT 1 FROM pictures p WHERE NOT EXISTS(SELECT 1 FROM identities b WHERE b.kind='image_bin' AND b.value=p.bin) OR NOT EXISTS(SELECT 1 FROM identities e WHERE e.kind='event' AND e.value=p.event))",
        "SELECT EXISTS(SELECT 1 FROM members m WHERE NOT EXISTS(SELECT 1 FROM identities a WHERE a.kind='album' AND a.value=m.album) OR NOT EXISTS(SELECT 1 FROM identities p WHERE p.kind='picture' AND p.value=m.picture))",
    ] {
        let invalid: i64 = sqlx::query_scalar(query).fetch_one(&mut *tx).await?;
        if invalid != 0 {
            return Err(CatalogError::Invalid);
        }
    }
    tx.commit().await?;
    String::from_utf8(library).map_err(|_| CatalogError::Invalid)
}

struct Resources {
    store: Store,
    snapshot: Option<Snapshot>,
    decoded: Option<Decoded>,
    scratch: Option<SqliteConnection>,
    scratch_file: Option<Snapshot>,
    reservation: Option<String>,
    worker: Option<crate::intake::CatalogWorker>,
    #[cfg(test)]
    probe: Option<std::sync::Arc<ImportProbe>>,
}
#[cfg(test)]
struct ImportProbe {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Resources {
    async fn cleanup(mut self) -> Result<(), CatalogError> {
        // The owning worker is never aborted by dropping its caller. All SQLite
        // workers are drained before unlink and before releasing reserved bytes.
        if let Some(decoded) = self.decoded.take() {
            decoded.conn.close().await?;
        }
        if let Some(conn) = self.scratch.take() {
            conn.close().await?;
        }
        for file in [&self.snapshot, &self.scratch_file].into_iter().flatten() {
            match fs::remove_file(&file.path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        fs::File::open(self.store.staging_root())?.sync_all()?;
        if let Some(owner) = self.reservation.take() {
            self.store.release_catalog_scratch(&owner).await?;
        }
        // Registration owns the Store root locks through the final quota release.
        self.worker.take();
        Ok(())
    }
}
struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
/// Cancellation requests the task-owned worker to stop and drain. It never
/// detaches SQLite IO from its reservation or publishes a half-imported catalog.
pub async fn import_desktop(
    catalog: &Catalog,
    store: &Store,
    job: &str,
) -> Result<ImportSummary, CatalogError> {
    import_with_resources(
        catalog,
        store,
        job,
        Resources {
            store: store.clone(),
            snapshot: None,
            decoded: None,
            scratch: None,
            scratch_file: None,
            reservation: None,
            worker: None,
            #[cfg(test)]
            probe: None,
        },
    )
    .await
}
async fn import_with_resources(
    catalog: &Catalog,
    store: &Store,
    job: &str,
    mut resources: Resources,
) -> Result<ImportSummary, CatalogError> {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _cancel = CancelOnDrop(cancel.clone());
    resources.worker = Some(store.catalog_worker(cancel.clone())?);
    let catalog = catalog.clone();
    let store = store.clone();
    let job = job.to_string();
    tokio::spawn(async move {
        let _permit = IMPORTS
            .acquire()
            .await
            .map_err(|_| CatalogError::Unavailable)?;
        use futures_util::FutureExt;
        let result = std::panic::AssertUnwindSafe(import_owned(
            &catalog,
            &store,
            &job,
            cancel,
            &mut resources,
        ))
        .catch_unwind()
        .await
        .unwrap_or(Err(CatalogError::Unavailable));
        resources.cleanup().await?;
        result
    })
    .await
    .map_err(|_| CatalogError::Unavailable)?
}

async fn import_owned(
    catalog: &Catalog,
    store: &Store,
    job: &str,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    resources: &mut Resources,
) -> Result<ImportSummary, CatalogError> {
    cancelled(&cancel)?;
    migrate(catalog).await?;
    cancelled(&cancel)?;
    let entries = store.catalog_manifest(job, cancel.clone()).await?;
    let mut hash = Sha256::new();
    // Strip an optional common selection wrapper; source paths themselves remain exact.
    let wrapper = entries
        .iter()
        .find(|e| e.logical_path.ends_with("/lytrolibrary.db"))
        .and_then(|e| {
            e.logical_path
                .rsplit_once('/')
                .map(|(p, _)| format!("{p}/"))
        })
        .filter(|prefix| entries.iter().all(|e| e.logical_path.starts_with(prefix)));
    for e in &entries {
        let logical = wrapper
            .as_ref()
            .and_then(|p| e.logical_path.strip_prefix(p))
            .unwrap_or(&e.logical_path);
        hash.update((logical.len() as u64).to_le_bytes());
        hash.update(logical.as_bytes());
        hash.update(e.digest.as_bytes());
        hash.update(e.length.to_le_bytes());
    }
    let fingerprint = format!("{:x}", hash.finalize());
    if let Some(id) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM catalog_versions WHERE fingerprint=$1")
            .bind(&fingerprint)
            .fetch_optional(catalog.pool())
            .await?
    {
        return summary(catalog, id).await;
    }
    let active: Vec<_> = entries
        .iter()
        .filter(|e| {
            e.logical_path == "lytrolibrary.db" || e.logical_path.ends_with("/lytrolibrary.db")
        })
        .collect();
    let mut identity = None;
    let mut reason = "unsupported_snapshot";
    let mut prefix = "";
    if active.len() == 1 {
        let db = active[0];
        prefix = db.logical_path.strip_suffix("lytrolibrary.db").unwrap();
        let journal = entries.iter().any(|e| {
            ["-wal", "-journal", "-shm"]
                .iter()
                .any(|suffix| e.logical_path == format!("{}{suffix}", db.logical_path))
        });
        if journal {
            reason = "journal_snapshot";
        } else if db.length <= MAX_DB {
            // Bounded disk index replaces the unbounded Records/identity maps.
            // No SQL transaction spans source copying, hashing or validation.
            let unit = fs2::allocation_granularity(store.staging_root())?;
            let (pages, reserved) = scratch_plan(db.length, unit)?;
            cancelled(&cancel)?;
            let owner = store.reserve_catalog_scratch(job, reserved).await?;
            resources.reservation = Some(owner.clone());
            let s = store.clone();
            let e = db.clone();
            let cancelled = cancel.clone();
            resources.snapshot = Some(
                tokio::task::spawn_blocking(move || snapshot(s, e, Some(&cancelled)))
                    .await
                    .map_err(|_| CatalogError::Unavailable)??,
            );
            let path = store
                .staging_root()
                .join(format!("lytronaut-desktop-{owner}-index.db"));
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            resources.scratch_file = Some(Snapshot { path: path.clone() });
            resources.scratch = Some(scratch(&path, pages).await?);
            initialize_scratch(resources.scratch.as_mut().unwrap()).await?;
            #[cfg(test)]
            if let Some(probe) = resources.probe.as_ref() {
                probe.entered.notify_one();
                probe.release.notified().await;
            }
            cancelled(&cancel)?;
            match decode(resources.snapshot.as_ref().unwrap(), cancel.clone()).await {
                Ok(decoded) => {
                    resources.decoded = Some(decoded);
                    match validate(
                        resources.decoded.as_mut().unwrap(),
                        resources.scratch.as_mut().unwrap(),
                        cancel.clone(),
                    )
                    .await
                    {
                        Ok(id) => {
                            identity = Some(id);
                            reason = "";
                        }
                        Err(_) => {
                            reason = "invalid_or_unsupported_catalog";
                        }
                    }
                }
                Err(_) => {
                    reason = "invalid_database";
                }
            }
        } else {
            reason = "invalid_database";
        }
    }
    cancelled(&cancel)?;
    let mut tx = catalog.begin_write().await?;
    // Writer admission serializes identity publication on both engines.
    let stored_identity = identity
        .as_ref()
        .map(|value| catalog.encode_source_key(value));
    // PostgreSQL follows retained-table rowid allocation under writer admission,
    // avoiding nontransactional sequence drift/exhaustion. SQLite keeps its native
    // algorithm, including bounded positive allocation at signed64 MAX.
    if let Some(id) = catalog.catalog_insert_id(&mut tx).await? {
        sqlx::query("INSERT INTO catalog_versions(id,fingerprint,source_job,library_uuid,state,reason) VALUES($1,$2,$3,$4,'preserved_only',$5) ON CONFLICT(fingerprint) DO NOTHING")
            .bind(id).bind(&fingerprint).bind(job).bind(&stored_identity).bind(reason).execute(&mut *tx).await?;
    } else {
        sqlx::query("INSERT INTO catalog_versions(fingerprint,source_job,library_uuid,state,reason) VALUES($1,$2,$3,'preserved_only',$4) ON CONFLICT(fingerprint) DO NOTHING")
            .bind(&fingerprint).bind(job).bind(&stored_identity).bind(reason).execute(&mut *tx).await?;
    }
    let id: i64 = sqlx::query_scalar("SELECT id FROM catalog_versions WHERE fingerprint=$1")
        .bind(&fingerprint)
        .fetch_one(&mut *tx)
        .await?;
    let installed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog_sources WHERE version=$1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    if installed > 0 {
        tx.rollback().await?;
        return summary(catalog, id).await;
    }
    let state: String = sqlx::query_scalar("SELECT state FROM intake_jobs WHERE id=$1")
        .bind(job)
        .fetch_one(&mut *tx)
        .await?;
    if state != "succeeded" {
        return Err(CatalogError::IntakeIncomplete);
    }
    for e in &entries {
        let inserted=sqlx::query("INSERT INTO catalog_sources(version,job,path,digest,length) SELECT $1,job,path,digest,length FROM intake_sources WHERE job=$2 AND path=$3 AND digest=$4 AND length=$5")
        .bind(id).bind(job).bind(&e.logical_path).bind(&e.digest).bind(e.length as i64).execute(&mut *tx).await?;
        if inserted.rows_affected() != 1 {
            return Err(CatalogError::IntakeIncomplete);
        }
    }
    let conflict: i64 = if let Some(identity) = &identity {
        sqlx::query_scalar("SELECT count(*) FROM catalog_versions WHERE library_uuid=$1 AND id<>$2")
            .bind(catalog.encode_source_key(identity))
            .bind(id)
            .fetch_one(&mut *tx)
            .await?
    } else {
        0
    };
    if identity.is_some() {
        use futures_util::TryStreamExt;
        let paths: BTreeSet<_> = entries.iter().map(|e| e.logical_path.as_str()).collect();
        let decoded = resources.decoded.as_mut().ok_or(CatalogError::Invalid)?;
        for table in &decoded.tables {
            let budget = ReadBudget::arm(&mut decoded.conn, cancel.clone()).await?;
            let mut rows =
                sqlx::query(AssertSqlSafe(table.select.clone())).fetch(&mut decoded.conn);
            let mut ordinal = 0i64;
            loop {
                cancelled(&cancel)?;
                budget.next()?;
                let Some(row) = rows.try_next().await? else {
                    break;
                };
                let r = record(row, &table.names)?;
                sqlx::query("INSERT INTO catalog_records VALUES($1,$2,$3,$4)")
                    .bind(id)
                    .bind(table.name)
                    .bind(ordinal)
                    .bind(serde_json::to_vec(&r).map_err(|_| CatalogError::Invalid)?)
                    .execute(&mut *tx)
                    .await?;
                ordinal = ordinal.checked_add(1).ok_or(CatalogError::Invalid)?;
                match table.name {
                    "picture" => {
                        let statement = catalog.dialect(
                "INSERT INTO catalog_pictures VALUES($1,$2,$3,$4,$5,$6,$7,$8)",
                "INSERT INTO catalog_pictures(version,uuid,bin_uuid,event_uuid,name,caption,rating,flag_status,name_display,name_fold,name_bytes,caption_display,caption_fold,caption_bytes) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"
            );
                        let name = crate::source_text::TextProjection::new(r.text("name")?, 256);
                        let caption =
                            crate::source_text::TextProjection::new(r.text("caption")?, 1024);
                        let query = sqlx::query(statement)
                            .bind(id)
                            .bind(catalog.encode_source_key(r.text("uuid")?))
                            .bind(catalog.encode_source_key(r.text("imagebin_uuid")?))
                            .bind(catalog.encode_source_key(r.text("event_uuid")?));
                        match catalog.backend() {
                            crate::Backend::Sqlite => {
                                query
                                    .bind(r.text("name")?)
                                    .bind(r.text("caption")?)
                                    .bind(r.integer("rating")?)
                                    .bind(r.integer("flag_status")?)
                                    .execute(&mut *tx)
                                    .await?;
                            }
                            crate::Backend::Postgres => {
                                query
                                    .bind(name.original)
                                    .bind(caption.original)
                                    .bind(r.integer("rating")?)
                                    .bind(r.integer("flag_status")?)
                                    .bind(name.display)
                                    .bind(name.folded_prefix)
                                    .bind(name.original_bytes)
                                    .bind(caption.display)
                                    .bind(caption.folded_prefix)
                                    .bind(caption.original_bytes)
                                    .execute(&mut *tx)
                                    .await?;
                            }
                        }

                        for companion in [
                            "raw.lfp",
                            "dm.lfp",
                            "stack.lfp",
                            "stack.view.lfp",
                            "thumbnail.jpg",
                            "view_parameters.json",
                        ] {
                            if !paths.contains(
                                format!(
                                    "{prefix}{}/{}/{companion}",
                                    r.text("imagebin_uuid")?,
                                    r.text("uuid")?
                                )
                                .as_str(),
                            ) {
                                sqlx::query("INSERT INTO catalog_item_errors VALUES($1,$2,$3)")
                                    .bind(id)
                                    .bind(catalog.encode_source_key(r.text("uuid")?))
                                    .bind(format!("missing_{companion}"))
                                    .execute(&mut *tx)
                                    .await?;
                            }
                        }
                    }
                    "album" => {
                        let name = crate::source_text::TextProjection::new(r.text("name")?, 256);
                        let query = sqlx::query(catalog.dialect(
                "INSERT INTO catalog_albums VALUES($1,$2,$3,$4)",
                "INSERT INTO catalog_albums(version,uuid,name,sort_order,name_display,name_fold,name_bytes) VALUES($1,$2,$3,$4,$5,$6,$7)"
            ))
                .bind(id)
                .bind(catalog.encode_source_key(r.text("uuid")?));
                        match catalog.backend() {
                            crate::Backend::Sqlite => {
                                query
                                    .bind(r.text("name")?)
                                    .bind(r.integer("sort_order")?)
                                    .execute(&mut *tx)
                                    .await?;
                            }
                            crate::Backend::Postgres => {
                                query
                                    .bind(name.original)
                                    .bind(r.integer("sort_order")?)
                                    .bind(name.display)
                                    .bind(name.folded_prefix)
                                    .bind(name.original_bytes)
                                    .execute(&mut *tx)
                                    .await?;
                            }
                        }
                    }
                    "album_to_picture" => {
                        sqlx::query("INSERT INTO catalog_memberships VALUES($1,$2,$3,$4)")
                            .bind(id)
                            .bind(catalog.encode_source_key(r.text("album_uuid")?))
                            .bind(catalog.encode_source_key(r.text("picture_uuid")?))
                            .bind(r.integer("picture_index")?)
                            .execute(&mut *tx)
                            .await?;
                    }
                    _ => {}
                }
            }
        }
    }
    cancelled(&cancel)?;
    let state = if identity.is_none() {
        CatalogState::PreservedOnly
    } else if conflict > 0 {
        CatalogState::Quarantined
    } else {
        CatalogState::Ready
    };
    sqlx::query("UPDATE catalog_versions SET state=$1,conflict=$2 WHERE id=$3")
        .bind(state.label())
        .bind(i64::from(conflict > 0))
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    summary(catalog, id).await
}
/// Aggregate diagnostics disclose no source identifiers, names, paths or hashes.
pub async fn summary(catalog: &Catalog, id: i64) -> Result<ImportSummary, CatalogError> {
    let version = sqlx::query("SELECT state,conflict,reason FROM catalog_versions WHERE id=$1")
        .bind(id)
        .fetch_one(catalog.pool())
        .await?;
    let state = match version.get::<String, _>("state").as_str() {
        "ready" => CatalogState::Ready,
        "quarantined" => CatalogState::Quarantined,
        _ => CatalogState::PreservedOnly,
    };
    let mut counts = BTreeMap::new();
    for r in
        sqlx::query("SELECT kind,count(*) AS n FROM catalog_records WHERE version=$1 GROUP BY kind")
            .bind(id)
            .fetch_all(catalog.pool())
            .await?
    {
        counts.insert(r.get::<String, _>("kind"), r.get::<i64, _>("n") as usize);
    }
    let files: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_sources WHERE version=$1")
        .bind(id)
        .fetch_one(catalog.pool())
        .await?;
    let errors: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog_item_errors WHERE version=$1")
            .bind(id)
            .fetch_one(catalog.pool())
            .await?;
    let n = |kind: &str| counts.get(kind).copied().unwrap_or(0);
    Ok(ImportSummary {
        version_id: id,
        state,
        files: files as usize,
        pictures: n("picture"),
        metadata: n("picture_metadata"),
        albums: n("album"),
        memberships: n("album_to_picture"),
        bins: n("image_bin"),
        events: n("event"),
        picture_lists: n("picture_list"),
        item_errors: errors as usize,
        conflicting_identity: version.get::<i64, _>("conflict") != 0,
        reason: {
            let reason: String = version.try_get("reason")?;
            if reason.is_empty() {
                None
            } else {
                Some(reason)
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_budget_uses_actual_work_unit_and_metadata_for_each_file() {
        let unit = 8 * 1024 * 1024;
        let (pages, reserved) = scratch_plan(4096, unit).unwrap();
        assert!(pages * 4096 < unit);
        assert_eq!(reserved, 2 * (unit + 1024));
        assert_eq!(scratch_plan(u64::MAX, 4096), Err(CatalogError::Invalid));
        assert_eq!(scratch_plan(1, 0), Err(CatalogError::Unavailable));
    }
    #[test]
    fn copy_rejects_changed_bytes_even_when_length_is_unchanged() {
        let entry = SourceEntry {
            logical_path: "lytrolibrary.db".into(),
            digest: format!("{:x}", Sha256::digest(b"sealed")),
            length: 6,
        };
        let mut out = Vec::new();
        assert_eq!(
            copy_verified(&mut &b"mutate"[..], &mut out, &entry, None),
            Err(CatalogError::Invalid)
        );
        assert!(copy_verified(&mut &b"sealed"[..], &mut Vec::new(), &entry, None).is_ok());
    }
    #[tokio::test]
    async fn dropped_caller_keeps_quota_until_worker_drains_and_unlinks() {
        dropped_caller_case(false).await;
    }
    #[tokio::test]
    async fn cleanup_error_retains_quota_until_owned_restart_can_reconcile() {
        dropped_caller_case(true).await;
    }
    async fn dropped_caller_case(cleanup_error: bool) {
        use crate::Limits;
        use lytronaut_contracts::{ImportKind, ImportRequest, WireVersion};
        let root =
            std::env::temp_dir().join(format!("lytronaut-desktop-cancel-{}", std::process::id()));
        // Randomized suffix is needed for concurrent/previous runs.
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = root.with_extension(format!("{:x}", u128::from_le_bytes(random)));
        fs::create_dir(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let mut store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                free_floor: 0,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        let job = store
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: ImportKind::Directory,
                declared_bytes: 64,
                declared_files: 1,
            })
            .await
            .unwrap();
        store
            .append_directory_file(
                &job,
                "lytrolibrary.db",
                &b"independent invalid SQLite source"[..],
            )
            .await
            .unwrap();
        store.finish(&job).await.unwrap();
        let probe = std::sync::Arc::new(ImportProbe {
            entered: Default::default(),
            release: Default::default(),
        });
        let resources = Resources {
            store: store.clone(),
            snapshot: None,
            decoded: None,
            scratch: None,
            scratch_file: None,
            reservation: None,
            worker: None,
            probe: Some(probe.clone()),
        };
        let c = catalog.clone();
        let s = store.clone();
        let j = job.clone();
        let caller =
            tokio::spawn(async move { import_with_resources(&c, &s, &j, resources).await });
        tokio::time::timeout(Duration::from_secs(10), probe.entered.notified())
            .await
            .unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        let reserved: i64 = sqlx::query_scalar("SELECT sum(bytes) FROM intake_scratch")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert!(reserved > 0);
        assert!(fs::read_dir(store.staging_root()).unwrap().count() > 1);
        let shutdown_store = store.clone();
        let shutdown = tokio::spawn(async move { shutdown_store.shutdown().await });
        tokio::task::yield_now().await;
        assert!(
            !shutdown.is_finished(),
            "shutdown cannot pass a paused scratch owner"
        );
        let blocked = if cleanup_error {
            let path = fs::read_dir(store.staging_root())
                .unwrap()
                .map(|e| e.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .ends_with("-index.db")
                })
                .unwrap();
            let held = path.with_extension("held");
            fs::rename(&path, &held).unwrap();
            fs::create_dir(&path).unwrap();
            Some((path, held))
        } else {
            None
        };
        probe.release.notify_one();
        tokio::time::timeout(Duration::from_secs(10), shutdown)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Some((path, held)) = blocked {
            let charged: i64 = sqlx::query_scalar("SELECT count(*) FROM intake_scratch")
                .fetch_one(catalog.pool())
                .await
                .unwrap();
            assert_eq!(charged, 1, "cleanup failure must retain durable quota");
            let limits = store.limits().clone();
            drop(store);
            assert!(
                Store::open(
                    catalog.clone(),
                    &root.join("originals"),
                    &root.join("staging"),
                    limits.clone()
                )
                .await
                .is_err(),
                "startup must refuse non-file scratch ownership without releasing quota"
            );
            let charged: i64 = sqlx::query_scalar("SELECT count(*) FROM intake_scratch")
                .fetch_one(catalog.pool())
                .await
                .unwrap();
            assert_eq!(charged, 1);
            fs::remove_dir(path.clone()).unwrap();
            fs::rename(held, path).unwrap();
            store = Store::open(
                catalog.clone(),
                &root.join("originals"),
                &root.join("staging"),
                limits,
            )
            .await
            .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM intake_scratch")
                    .fetch_one(catalog.pool())
                    .await
                    .unwrap();
                if remaining == 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fs::read_dir(store.staging_root()).unwrap().count(), 1);
        let versions: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_versions")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(versions, 0);
        assert_eq!(store.manifest(&job).await.unwrap().len(), 1);
        drop(store);
        catalog.pool().close().await;
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn snapshot_uses_private_staging_and_cleans_copy_without_changing_original() {
        use crate::Limits;
        use lytronaut_contracts::{ImportKind, ImportRequest, WireVersion};
        use std::io::Read;
        use std::os::unix::fs::DirBuilderExt;
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-snapshot-test-{:x}",
            u128::from_le_bytes(random)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                free_floor: 0,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        let original = b"independent opaque original source bytes";
        let id = store
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: ImportKind::Directory,
                declared_bytes: original.len() as u64,
                declared_files: 1,
            })
            .await
            .unwrap();
        store
            .append_directory_file(&id, "lytrolibrary.db", &original[..])
            .await
            .unwrap();
        store.finish(&id).await.unwrap();
        let entry = store.manifest(&id).await.unwrap().remove(0);
        let copy = snapshot(store.clone(), entry.clone(), None).unwrap();
        assert_eq!(copy.path.parent(), Some(store.staging_root()));
        assert_eq!(
            fs::metadata(store.staging_root())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(fs::read(&copy.path).unwrap(), original);
        assert_eq!(
            fs::metadata(&copy.path).unwrap().permissions().mode() & 0o777,
            0o400
        );
        let scratch = copy.path.clone();
        drop(copy);
        assert!(!scratch.exists());
        let mut preserved = Vec::new();
        store
            .open_blob(&entry)
            .unwrap()
            .read_to_end(&mut preserved)
            .unwrap();
        assert_eq!(preserved, original);
        let interrupted = snapshot(store.clone(), entry.clone(), None).unwrap();
        let interrupted_path = interrupted.path.clone();
        std::mem::forget(interrupted);
        catalog.pool().close().await;
        drop(store);
        let reopened_catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        let reopened = Store::open(
            reopened_catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                free_floor: 0,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        assert!(!interrupted_path.exists());
        let mut after_recovery = Vec::new();
        reopened
            .open_blob(&entry)
            .unwrap()
            .read_to_end(&mut after_recovery)
            .unwrap();
        assert_eq!(after_recovery, original);
        reopened_catalog.pool().close().await;
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn source_vm_progress_budget_interrupts_work() {
        let mut conn =
            SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(":memory:"))
                .await
                .unwrap();
        let mut ticks = 0;
        conn.lock_handle()
            .await
            .unwrap()
            .set_progress_handler(1000, move || {
                ticks += 1;
                ticks <= 2
            });
        let result=sqlx::query_scalar::<_,i64>("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000) SELECT sum(x) FROM n").fetch_one(&mut conn).await;
        assert!(result.is_err());
    }
}
