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
const MAX_ROWS: usize = 100_000;
const MAX_CELL: i64 = 1024 * 1024;
const MAX_TYPED: usize = 8 * 1024 * 1024;
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
    let mut tx = catalog.pool().begin_with("BEGIN IMMEDIATE").await?;
    for sql in [
        "CREATE TABLE IF NOT EXISTS catalog_versions(id INTEGER PRIMARY KEY, fingerprint TEXT NOT NULL UNIQUE, source_job TEXT NOT NULL REFERENCES intake_jobs(id), library_uuid TEXT, state TEXT NOT NULL, reason TEXT, conflict INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE IF NOT EXISTS catalog_sources(version INTEGER NOT NULL REFERENCES catalog_versions(id), job TEXT NOT NULL, path TEXT NOT NULL, digest TEXT NOT NULL, length INTEGER NOT NULL, PRIMARY KEY(version,path), FOREIGN KEY(job,path) REFERENCES intake_sources(job,path))",
        "CREATE TABLE IF NOT EXISTS catalog_records(version INTEGER NOT NULL REFERENCES catalog_versions(id), kind TEXT NOT NULL, ordinal INTEGER NOT NULL, original BLOB NOT NULL, PRIMARY KEY(version,kind,ordinal))",
        "CREATE TABLE IF NOT EXISTS catalog_pictures(version INTEGER NOT NULL REFERENCES catalog_versions(id), uuid TEXT NOT NULL, bin_uuid TEXT NOT NULL, event_uuid TEXT NOT NULL, name TEXT NOT NULL, caption TEXT NOT NULL, rating INTEGER NOT NULL, flag_status INTEGER NOT NULL, PRIMARY KEY(version,uuid))",
        "CREATE TABLE IF NOT EXISTS catalog_albums(version INTEGER NOT NULL REFERENCES catalog_versions(id), uuid TEXT NOT NULL, name TEXT NOT NULL, sort_order INTEGER NOT NULL, PRIMARY KEY(version,uuid))",
        "CREATE TABLE IF NOT EXISTS catalog_memberships(version INTEGER NOT NULL, album_uuid TEXT NOT NULL, picture_uuid TEXT NOT NULL, picture_index INTEGER NOT NULL, PRIMARY KEY(version,album_uuid,picture_uuid), UNIQUE(version,album_uuid,picture_index), FOREIGN KEY(version,album_uuid) REFERENCES catalog_albums(version,uuid), FOREIGN KEY(version,picture_uuid) REFERENCES catalog_pictures(version,uuid))",
        "CREATE TABLE IF NOT EXISTS catalog_item_errors(version INTEGER NOT NULL REFERENCES catalog_versions(id), picture_uuid TEXT NOT NULL, code TEXT NOT NULL, PRIMARY KEY(version,picture_uuid,code))",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    let invalid:i64=sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name IN ('catalog_versions','catalog_sources','catalog_records','catalog_pictures','catalog_albums','catalog_memberships','catalog_item_errors') AND type<>'table'").fetch_one(&mut *tx).await?;
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
fn snapshot(store: Store, entry: SourceEntry) -> Result<Snapshot, CatalogError> {
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
    if io::copy(&mut src, &mut dst)? != entry.length {
        return Err(CatalogError::Invalid);
    }
    dst.sync_all()?;
    fs::set_permissions(&snapshot.path, fs::Permissions::from_mode(0o400))?;
    Ok(snapshot)
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
type Records = BTreeMap<String, Vec<OriginalRecord>>;
async fn decode(snapshot: &Snapshot) -> Result<Records, CatalogError> {
    let options = SqliteConnectOptions::new()
        .filename(&snapshot.path)
        .read_only(true)
        .immutable(true)
        .pragma("query_only", "ON")
        .pragma("trusted_schema", "OFF")
        .pragma("ignore_check_constraints", "ON")
        .pragma("mmap_size", "0")
        .pragma("cache_size", "-2048")
        .pragma("temp_store", "MEMORY");
    let mut conn = SqliteConnection::connect_with(&options).await?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ticks = 0u32;
    conn.lock_handle()
        .await?
        .set_progress_handler(1000, move || {
            ticks += 1;
            ticks <= 20_000 && Instant::now() < deadline
        });
    // Bound schema rows/bytes before retrieving strings. sqlite_master is built in.
    let schema = sqlx::query("SELECT count(*),max(length(CAST(sql AS BLOB))),sum(length(CAST(sql AS BLOB))),max(length(CAST(name AS BLOB))) FROM sqlite_master").fetch_one(&mut conn).await?;
    if schema.try_get::<i64, _>(0)? > 1024
        || schema.try_get::<Option<i64>, _>(1)?.unwrap_or(0) > 65536
        || schema.try_get::<Option<i64>, _>(2)?.unwrap_or(0) > 4 * 1024 * 1024
        || schema.try_get::<Option<i64>, _>(3)?.unwrap_or(0) > 256
    {
        return Err(CatalogError::Invalid);
    }
    // No source views, triggers or extension functions are executed.
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
    // quick_check can evaluate generated expressions or virtual integrity hooks.
    // Admit only ordinary tables with physical columns throughout the snapshot,
    // and suppress CHECK evaluation, including on opaque future tables.
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
    let mut records = Records::new();
    let mut total_rows = 0usize;
    let mut total_cells = 0usize;
    let mut total_bytes = 0usize;
    for (table, required) in TABLES {
        let columns = sqlx::query(AssertSqlSafe(format!(
            "PRAGMA table_xinfo({})",
            quote(table)
        )))
        .fetch_all(&mut conn)
        .await?;
        if columns
            .iter()
            .any(|r| r.try_get::<i64, _>("hidden").map_or(true, |v| v != 0))
            || columns.is_empty()
            || columns.len() > 128
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
        let count: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
            "SELECT count(*) FROM {}",
            quote(table)
        )))
        .fetch_one(&mut conn)
        .await?;
        total_rows = total_rows
            .checked_add(usize::try_from(count).map_err(|_| CatalogError::Invalid)?)
            .ok_or(CatalogError::Invalid)?;
        total_cells += count as usize * names.len();
        if total_cells > 100_000 || total_rows > MAX_ROWS {
            return Err(CatalogError::Invalid);
        }
        let fields = names
            .iter()
            .map(|n| {
                format!(
                    "max(length(CAST({0} AS BLOB))),sum(length(CAST({0} AS BLOB)))",
                    quote(n)
                )
            })
            .collect::<Vec<_>>();
        let sizes = sqlx::query(AssertSqlSafe(format!(
            "SELECT {} FROM {}",
            fields.join(","),
            quote(table)
        )))
        .fetch_all(&mut conn)
        .await?;
        for row in sizes {
            for i in 0..names.len() {
                let length = row.try_get::<Option<i64>, _>(i * 2)?.unwrap_or(0);
                if !(0..=MAX_CELL).contains(&length) {
                    return Err(CatalogError::Invalid);
                }
                total_bytes += row.try_get::<Option<i64>, _>(i * 2 + 1)?.unwrap_or(0) as usize;
                if total_bytes > MAX_TYPED {
                    return Err(CatalogError::Invalid);
                }
            }
        }
        let fields = names
            .iter()
            .map(|n| {
                let q = quote(n);
                format!("typeof({q}),{q},CAST({q} AS BLOB)")
            })
            .collect::<Vec<_>>();
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {} FROM {}",
            fields.join(","),
            quote(table)
        )))
        .fetch_all(&mut conn)
        .await?;
        let mut originals = Vec::with_capacity(rows.len());
        for row in rows {
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
            originals.push(OriginalRecord { fields });
        }
        records.insert(table.to_string(), originals);
    }
    conn.close().await?;
    Ok(records)
}
fn identities(records: &Records, table: &str) -> Result<BTreeSet<String>, CatalogError> {
    let mut ids = BTreeSet::new();
    for row in &records[table] {
        let id = row.text("uuid")?;
        if id.is_empty() || !ids.insert(id.to_string()) {
            return Err(CatalogError::Invalid);
        }
    }
    Ok(ids)
}
fn validate(
    records: &Records,
    entries: &[SourceEntry],
    prefix: &str,
) -> Result<(String, Vec<(String, String)>), CatalogError> {
    if records["library_metadata"].len() != 1
        || records["library_metadata"][0].integer("version")? != 23
    {
        return Err(CatalogError::Invalid);
    }
    let pictures = identities(records, "picture")?;
    let metadata = identities(records, "picture_metadata")?;
    let albums = identities(records, "album")?;
    let bins = identities(records, "image_bin")?;
    let events = identities(records, "event")?;
    identities(records, "picture_list")?;
    if pictures != metadata {
        return Err(CatalogError::Invalid);
    }
    let libraries = records["event"]
        .iter()
        .map(|r| r.text("library_uuid").map(str::to_string))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if libraries.len() != 1 || libraries.first().is_none_or(|s| s.is_empty()) {
        return Err(CatalogError::Invalid);
    }
    for rows in records.values() {
        let mut ids = BTreeSet::new();
        for r in rows {
            if let Some(id) = r.fields.get("id")
                && !ids.insert(serde_json::to_vec(id).map_err(|_| CatalogError::Invalid)?)
            {
                return Err(CatalogError::Invalid);
            }
        }
    }
    let mut pairs = BTreeSet::new();
    let mut indices = BTreeSet::new();
    for r in &records["album_to_picture"] {
        let a = r.text("album_uuid")?;
        let p = r.text("picture_uuid")?;
        let i = r.integer("picture_index")?;
        if !albums.contains(a)
            || !pictures.contains(p)
            || i < 0
            || !pairs.insert((a, p))
            || !indices.insert((a, i))
        {
            return Err(CatalogError::Invalid);
        }
    }
    let paths: BTreeSet<_> = entries.iter().map(|e| e.logical_path.as_str()).collect();
    let mut errors = Vec::new();
    for r in &records["album"] {
        r.text("name")?;
        r.integer("sort_order")?;
    }
    for r in &records["picture"] {
        r.text("name")?;
        r.text("caption")?;
        r.integer("rating")?;
        r.integer("flag_status")?;
        let id = r.text("uuid")?;
        let bin = r.text("imagebin_uuid")?;
        if !bins.contains(bin)
            || !events.contains(r.text("event_uuid")?)
            || [id, bin]
                .iter()
                .any(|s| s.contains('/') || s.contains('\\') || *s == "." || *s == "..")
        {
            return Err(CatalogError::Invalid);
        }
        for companion in [
            "raw.lfp",
            "dm.lfp",
            "stack.lfp",
            "stack.view.lfp",
            "thumbnail.jpg",
            "view_parameters.json",
        ] {
            if !paths.contains(format!("{prefix}{bin}/{id}/{companion}").as_str()) {
                errors.push((id.to_string(), format!("missing_{companion}")));
            }
        }
    }
    Ok((libraries.into_iter().next().unwrap(), errors))
}

/// Run once per intake, never on a browsing query. Unsupported/hostile snapshots
/// receive a durable preservation-only version, keeping every source reference.
pub async fn import_desktop(
    catalog: &Catalog,
    store: &Store,
    job: &str,
) -> Result<ImportSummary, CatalogError> {
    let _permit = IMPORTS
        .acquire()
        .await
        .map_err(|_| CatalogError::Unavailable)?;
    migrate(catalog).await?;
    let entries = store.manifest(job).await?;
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
        sqlx::query_scalar::<_, i64>("SELECT id FROM catalog_versions WHERE fingerprint=?")
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
    let mut records = None;
    let mut identity = None;
    let mut errors = Vec::new();
    let mut reason = "unsupported_snapshot";
    if active.len() == 1 {
        let db = active[0];
        let prefix = db.logical_path.strip_suffix("lytrolibrary.db").unwrap();
        let journal = entries.iter().any(|e| {
            ["-wal", "-journal", "-shm"]
                .iter()
                .any(|suffix| e.logical_path == format!("{}{suffix}", db.logical_path))
        });
        if journal {
            reason = "journal_snapshot";
        } else {
            let s = store.clone();
            let e = db.clone();
            if let Ok(Ok(snapshot)) = tokio::task::spawn_blocking(move || snapshot(s, e)).await {
                if let Ok(decoded) = decode(&snapshot).await {
                    if let Ok((id, item_errors)) = validate(&decoded, &entries, prefix) {
                        identity = Some(id);
                        errors = item_errors;
                        records = Some(decoded);
                        reason = "";
                    } else {
                        reason = "invalid_or_unsupported_catalog";
                    }
                } else {
                    reason = "invalid_database";
                }
            } else {
                reason = "invalid_database";
            }
        }
    }
    let mut tx = catalog.pool().begin().await?;
    // Claim fingerprint before checking identity: SQLite serializes competing writers.
    sqlx::query("INSERT OR IGNORE INTO catalog_versions(fingerprint,source_job,library_uuid,state,reason) VALUES(?,?,?,'preserved_only',?)")
        .bind(&fingerprint).bind(job).bind(&identity).bind(reason).execute(&mut *tx).await?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM catalog_versions WHERE fingerprint=?")
        .bind(&fingerprint)
        .fetch_one(&mut *tx)
        .await?;
    let installed: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_sources WHERE version=?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if installed > 0 {
        tx.rollback().await?;
        return summary(catalog, id).await;
    }
    let state: String = sqlx::query_scalar("SELECT state FROM intake_jobs WHERE id=?")
        .bind(job)
        .fetch_one(&mut *tx)
        .await?;
    if state != "succeeded" {
        return Err(CatalogError::IntakeIncomplete);
    }
    for e in &entries {
        let inserted=sqlx::query("INSERT INTO catalog_sources(version,job,path,digest,length) SELECT ?,job,path,digest,length FROM intake_sources WHERE job=? AND path=? AND digest=? AND length=?")
        .bind(id).bind(job).bind(&e.logical_path).bind(&e.digest).bind(e.length as i64).execute(&mut *tx).await?;
        if inserted.rows_affected() != 1 {
            return Err(CatalogError::IntakeIncomplete);
        }
    }
    let conflict: i64 = if let Some(identity) = &identity {
        sqlx::query_scalar("SELECT count(*) FROM catalog_versions WHERE library_uuid=? AND id<>?")
            .bind(identity)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?
    } else {
        0
    };
    if let Some(records) = records {
        for (kind, rows) in &records {
            for (ordinal, r) in rows.iter().enumerate() {
                sqlx::query("INSERT INTO catalog_records VALUES(?,?,?,?)")
                    .bind(id)
                    .bind(kind)
                    .bind(ordinal as i64)
                    .bind(serde_json::to_vec(r).map_err(|_| CatalogError::Invalid)?)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        for r in &records["picture"] {
            sqlx::query("INSERT INTO catalog_pictures VALUES(?,?,?,?,?,?,?,?)")
                .bind(id)
                .bind(r.text("uuid")?)
                .bind(r.text("imagebin_uuid")?)
                .bind(r.text("event_uuid")?)
                .bind(r.text("name")?)
                .bind(r.text("caption")?)
                .bind(r.integer("rating")?)
                .bind(r.integer("flag_status")?)
                .execute(&mut *tx)
                .await?;
        }
        for r in &records["album"] {
            sqlx::query("INSERT INTO catalog_albums VALUES(?,?,?,?)")
                .bind(id)
                .bind(r.text("uuid")?)
                .bind(r.text("name")?)
                .bind(r.integer("sort_order")?)
                .execute(&mut *tx)
                .await?;
        }
        for r in &records["album_to_picture"] {
            sqlx::query("INSERT INTO catalog_memberships VALUES(?,?,?,?)")
                .bind(id)
                .bind(r.text("album_uuid")?)
                .bind(r.text("picture_uuid")?)
                .bind(r.integer("picture_index")?)
                .execute(&mut *tx)
                .await?;
        }
        for (picture, code) in &errors {
            sqlx::query("INSERT INTO catalog_item_errors VALUES(?,?,?)")
                .bind(id)
                .bind(picture)
                .bind(code)
                .execute(&mut *tx)
                .await?;
        }
    }
    let state = if identity.is_none() {
        CatalogState::PreservedOnly
    } else if conflict > 0 {
        CatalogState::Quarantined
    } else {
        CatalogState::Ready
    };
    sqlx::query("UPDATE catalog_versions SET state=?,conflict=? WHERE id=?")
        .bind(state.label())
        .bind(conflict > 0)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    summary(catalog, id).await
}
/// Aggregate diagnostics disclose no source identifiers, names, paths or hashes.
pub async fn summary(catalog: &Catalog, id: i64) -> Result<ImportSummary, CatalogError> {
    let version = sqlx::query("SELECT state,conflict,reason FROM catalog_versions WHERE id=?")
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
        sqlx::query("SELECT kind,count(*) AS n FROM catalog_records WHERE version=? GROUP BY kind")
            .bind(id)
            .fetch_all(catalog.pool())
            .await?
    {
        counts.insert(r.get::<String, _>("kind"), r.get::<i64, _>("n") as usize);
    }
    let files: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_sources WHERE version=?")
        .bind(id)
        .fetch_one(catalog.pool())
        .await?;
    let errors: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog_item_errors WHERE version=?")
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
        let copy = snapshot(store.clone(), entry.clone()).unwrap();
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
        let interrupted = snapshot(store.clone(), entry.clone()).unwrap();
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
