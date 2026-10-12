#[path = "exports.rs"]
mod exports;
// Authenticated application API. Source keys stay private, mapped to durable random IDs.
use crate::{AppState, error};
use axum::{
    Extension, Router,
    body::{Body, Bytes, HttpBody},
    extract::{Path, RawQuery},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use lytronaut_contracts::*;
use lytronaut_storage::{Catalog, SourceEntry, Store, desktop};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Row, sqlite::SqliteRow};
use std::future::Future;
use std::os::unix::fs::FileExt;
use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Seek, SeekFrom},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
const PAGE_BYTES: usize = 256 * 1024;
const CHUNK: u64 = 8 * 1024 * 1024;
const CACHE_RECORDS: usize = 128;
#[derive(Clone)]
pub struct Api(Arc<Inner>);
struct Inner {
    catalog: Catalog,
    store: Store,
    transfers: Arc<Semaphore>,
    validation: Arc<Semaphore>,
    cache: Mutex<Cache>,
}
#[derive(Default)]
struct Cache {
    values: HashMap<String, Arc<Descriptor>>,
    order: VecDeque<String>,
}
struct Descriptor {
    file: Arc<std::fs::File>,
    length: u64,
    manifest: Option<AssetManifest>,
}
#[derive(Debug, Clone, Copy)]
struct Failure(StatusCode, ErrorCode);
impl From<sqlx::Error> for Failure {
    fn from(_: sqlx::Error) -> Self {
        Self(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        )
    }
}
impl Failure {
    fn invalid() -> Self {
        Self(StatusCode::BAD_REQUEST, ErrorCode::InvalidInput)
    }
    fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, ErrorCode::NotFound)
    }
    fn response(self) -> Response {
        error(self.0, self.1)
    }
}
type Result<T> = std::result::Result<T, Failure>;
fn opaque(s: String) -> OpaqueId {
    OpaqueId::try_from(s).expect("generated application ID")
}
fn random_id() -> Result<String> {
    let mut b = [0; 32];
    getrandom::fill(&mut b)
        .map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Internal))?;
    Ok(format!(
        "app-{}",
        b.iter().map(|v| format!("{v:02x}")).collect::<String>()
    ))
}
fn state(s: &str) -> LibraryState {
    match s {
        "ready" => LibraryState::Ready,
        "quarantined" => LibraryState::Quarantined,
        _ => LibraryState::PreservedOnly,
    }
}
fn capabilities(preview: bool) -> CapabilityReport {
    let mut c = CapabilityReport::foundation();
    c.originals = CapabilityState::Available;
    c.static_preview = if preview {
        CapabilityState::Available
    } else {
        CapabilityState::Unavailable(ErrorCode::UnsupportedFormat)
    };
    c.calibrated_synthesis = CapabilityState::Disabled;
    c
}
fn role(path: &str) -> OriginalRole {
    match path.rsplit('/').next().unwrap_or("") {
        "raw.lfp" => OriginalRole::Raw,
        "dm.lfp" => OriginalRole::Depth,
        "stack.lfp" => OriginalRole::RefocusStack,
        "stacklq.lfp" => OriginalRole::JpegFocusStack,
        "stack.view.lfp" => OriginalRole::ViewStack,
        "thumbnail.jpg" => OriginalRole::Thumbnail,
        "view_parameters.json" => OriginalRole::LegacyEdits,
        "lytrolibrary.db" => OriginalRole::Catalog,
        _ => OriginalRole::Other,
    }
}
fn api_error(code: ErrorCode) -> ApiError {
    ApiError {
        version: WireVersion::V1,
        code,
        retryable: false,
    }
}
fn text_bounded(s: String, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_owned(), true)
}
async fn migrate(c: &Catalog) -> Result<()> {
    desktop::migrate(c).await.map_err(|_| {
        Failure(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        )
    })?;
    let mut tx = c.pool().begin_with("BEGIN IMMEDIATE").await?;
    for sql in [
        "CREATE TABLE IF NOT EXISTS api_ids(id TEXT PRIMARY KEY,kind TEXT NOT NULL,version INTEGER NOT NULL REFERENCES catalog_versions(id),source_key TEXT NOT NULL,UNIQUE(kind,version,source_key))",
        "CREATE TABLE IF NOT EXISTS api_imports(job TEXT PRIMARY KEY REFERENCES intake_jobs(id),version INTEGER NOT NULL REFERENCES catalog_versions(id))",
        "CREATE INDEX IF NOT EXISTS api_ids_lookup ON api_ids(kind,version,source_key)",
        "CREATE TABLE IF NOT EXISTS api_adaptation_errors(job TEXT PRIMARY KEY REFERENCES intake_jobs(id),code TEXT NOT NULL)",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    let tables:i64=sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('api_ids','api_imports','api_adaptation_errors')").fetch_one(&mut *tx).await?;
    if tables != 3 {
        return Err(Failure(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        ));
    }
    for sql in [
        "SELECT id,kind,version,source_key FROM api_ids LIMIT 0",
        "SELECT job,version FROM api_imports LIMIT 0",
        "SELECT job,code FROM api_adaptation_errors LIMIT 0",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
/// Idempotent mapping transaction, including deduplicated intakes whose catalog source_job differs.
async fn map_version(c: &Catalog, job: &str, version: i64) -> Result<String> {
    // Reserve the writer before reading keys: a deferred read upgrade can fail
    // immediately with SQLITE_BUSY when import progress holds another writer.
    let mut tx = c.pool().begin_with("BEGIN IMMEDIATE").await?;
    let mut keys = vec![("library".to_string(), String::new())];
    for (kind, sql) in [
        (
            "album",
            "SELECT uuid AS key FROM catalog_albums WHERE version=$1",
        ),
        (
            "capture",
            "SELECT uuid AS key FROM catalog_pictures WHERE version=$1",
        ),
        (
            "file",
            "SELECT path AS key FROM catalog_sources WHERE version=$1",
        ),
    ] {
        for row in sqlx::query(sql).bind(version).fetch_all(&mut *tx).await? {
            keys.push((kind.into(), row.get("key")));
        }
    }
    for (kind, key) in keys {
        sqlx::query("INSERT INTO api_ids VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(random_id()?)
            .bind(kind)
            .bind(version)
            .bind(key)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("INSERT INTO api_imports VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(job)
        .bind(version)
        .execute(&mut *tx)
        .await?;
    let id = sqlx::query_scalar("SELECT id FROM api_ids WHERE kind='library' AND version=$1")
        .bind(version)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}
/// Called only after intake completes; never hashes a whole library during browsing.
pub async fn adapt(
    c: &Catalog,
    s: &Store,
    job: &str,
) -> std::result::Result<(), desktop::CatalogError> {
    migrate(c)
        .await
        .map_err(|_| desktop::CatalogError::Unavailable)?;
    let result = async {
        let done: i64 = sqlx::query_scalar("SELECT count(*) FROM api_imports WHERE job=$1")
            .bind(job)
            .fetch_one(c.pool())
            .await?;
        if done == 0 {
            // If adaptation committed before an interruption, reuse that catalog rather than rehashing.
            let version: Option<i64> =
                sqlx::query_scalar("SELECT id FROM catalog_versions WHERE source_job=$1")
                    .bind(job)
                    .fetch_optional(c.pool())
                    .await?;
            let version = match version {
                Some(v) => v,
                None => desktop::import_desktop(c, s, job).await?.version_id,
            };
            map_version(c, job, version)
                .await
                .map_err(|_| desktop::CatalogError::Unavailable)?;
        }
        Ok::<(), desktop::CatalogError>(())
    }
    .await;
    match &result {
        Ok(()) => {
            sqlx::query("DELETE FROM api_adaptation_errors WHERE job=$1")
                .bind(job)
                .execute(c.pool())
                .await?;
        }
        Err(_) => {
            sqlx::query("INSERT INTO api_adaptation_errors VALUES($1,\'storage_unavailable\') ON CONFLICT(job) DO UPDATE SET code=excluded.code").bind(job).execute(c.pool()).await?;
        }
    }
    result
}
pub async fn progress(
    c: &Catalog,
    s: &Store,
    id: &str,
) -> std::result::Result<ImportProgress, lytronaut_storage::StorageError> {
    let intake = s.job(id).await?;
    migrate(c)
        .await
        .map_err(|_| lytronaut_storage::StorageError::Unavailable)?;
    let row=sqlx::query("SELECT a.id,v.state FROM api_imports i JOIN catalog_versions v ON v.id=i.version JOIN api_ids a ON a.version=v.id AND a.kind='library' WHERE i.job=$1").bind(id).fetch_optional(c.pool()).await.map_err(|_|lytronaut_storage::StorageError::Unavailable)?;
    let (library, catalog_state) = row
        .map(|r| {
            (
                Some(opaque(r.get("id"))),
                Some(state(&r.get::<String, _>("state"))),
            )
        })
        .unwrap_or((None, None));
    let failed: i64 = sqlx::query_scalar("SELECT count(*) FROM api_adaptation_errors WHERE job=$1")
        .bind(id)
        .fetch_one(c.pool())
        .await
        .map_err(|_| lytronaut_storage::StorageError::Unavailable)?;
    let phase = match intake.state {
        JobState::Succeeded => {
            if catalog_state.is_some() {
                ImportPhase::CatalogComplete
            } else if failed > 0 {
                ImportPhase::CatalogFailed
            } else {
                ImportPhase::CatalogPending
            }
        }
        JobState::Failed => ImportPhase::Failed,
        JobState::Cancelled => ImportPhase::Cancelled,
        JobState::Cancelling => ImportPhase::Cancelling,
        _ => ImportPhase::Intake,
    };
    Ok(ImportProgress {
        intake,
        phase,
        library,
        catalog_state,
        catalog_error: (failed > 0).then_some(ApiError {
            version: WireVersion::V1,
            code: ErrorCode::StorageUnavailable,
            retryable: true,
        }),
    })
}
impl Api {
    /// Startup recovers completed-but-unadapted intakes, retaining existing auth/storage tables.
    pub async fn open(
        catalog: Catalog,
        store: Store,
    ) -> std::result::Result<Self, desktop::CatalogError> {
        migrate(&catalog)
            .await
            .map_err(|_| desktop::CatalogError::Unavailable)?;
        let jobs:Vec<String>=sqlx::query_scalar("SELECT id FROM intake_jobs WHERE state='succeeded' AND id NOT IN (SELECT job FROM api_imports) ORDER BY rowid").fetch_all(catalog.pool()).await?;
        for job in jobs {
            if adapt(&catalog, &store, &job).await.is_err() {
                // A source failure is isolated only after its redacted status is durable.
                // Missing status or failed application queries mean global storage failure.
                let recorded: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM api_adaptation_errors WHERE job=$1")
                        .bind(&job)
                        .fetch_one(catalog.pool())
                        .await?;
                if recorded != 1 {
                    return Err(desktop::CatalogError::Unavailable);
                }
                catalog.healthy().await?;
            }
        }
        exports::migrate(&catalog, &store)
            .await
            .map_err(|_| desktop::CatalogError::Unavailable)?;
        Ok(Self(Arc::new(Inner {
            catalog,
            store,
            transfers: Arc::new(Semaphore::new(2)),
            validation: Arc::new(Semaphore::new(1)),
            cache: Mutex::new(Cache::default()),
        })))
    }
    pub fn routes(self) -> Router<AppState> {
        Router::new()
            .merge(exports::routes())
            .route("/libraries", get(libraries))
            .route("/libraries/{id}/albums", get(albums))
            .route("/libraries/{id}/captures", get(captures))
            .route("/libraries/{id}/files", get(files))
            .route("/albums/{id}/captures", get(album_captures))
            .route("/captures/{id}", get(detail))
            .route("/assets/{id}/focus", get(focus_manifest))
            .route("/assets/{id}/focus/{index}/{orientation}", get(focus_plane))
            .route("/assets/{id}/manifest", get(manifest))
            .route("/assets/{id}/chunks/{index}", get(chunk))
            .route("/assets/{id}/download", get(download))
            .layer(Extension(self))
    }
    async fn lookup(&self, id: &str, kind: &str, ready: bool) -> Result<(i64, String)> {
        if OpaqueId::try_from(id.to_owned()).is_err() {
            return Err(Failure::invalid());
        }
        let row=sqlx::query("SELECT a.version,a.source_key,v.state FROM api_ids a JOIN catalog_versions v ON v.id=a.version WHERE a.id=$1 AND a.kind=$2").bind(id).bind(kind).fetch_optional(self.0.catalog.pool()).await?.ok_or(Failure::missing())?;
        if ready && row.get::<String, _>("state") != "ready" {
            return Err(Failure(StatusCode::CONFLICT, ErrorCode::UnsupportedVersion));
        }
        Ok((row.get("version"), row.get("source_key")))
    }
    async fn descriptor(&self, id: &str, viewer: bool) -> Result<Arc<Descriptor>> {
        let (v, key) = self.lookup(id, "file", false).await?;
        let cache_key = format!("{id}-{viewer}");
        if let Some(d) = self.0.cache.lock().unwrap().values.get(&cache_key).cloned() {
            return Ok(d);
        }
        // Serialize validations: at most one conservative <=96MiB JPEG working allocation.
        let _permit = self
            .0
            .validation
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| {
                Failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::StorageUnavailable,
                )
            })?;
        if let Some(d) = self.0.cache.lock().unwrap().values.get(&cache_key).cloned() {
            return Ok(d);
        }
        let row = sqlx::query(
            "SELECT path,digest,length FROM catalog_sources WHERE version=$1 AND path=$2",
        )
        .bind(v)
        .bind(&key)
        .fetch_one(self.0.catalog.pool())
        .await?;
        let entry = SourceEntry {
            logical_path: row.get("path"),
            digest: row.get("digest"),
            length: row.get::<i64, _>("length") as u64,
        };
        let ready: String = sqlx::query_scalar("SELECT state FROM catalog_versions WHERE id=$1")
            .bind(v)
            .fetch_one(self.0.catalog.pool())
            .await?;
        let preview = viewer && ready == "ready" && key.ends_with("/thumbnail.jpg");
        let store = self.0.store.clone();
        let id = id.to_owned();
        let descriptor = tokio::task::spawn_blocking(move || -> Result<Descriptor> {
            let _validation_permit = _permit;
            let mut file = store.open_blob(&entry).map_err(|_| {
                Failure(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::StorageUnavailable,
                )
            })?;
            let mut manifest = if entry.length == 0 {
                None
            } else {
                Some(AssetManifest {
                    version: WireVersion::V1,
                    id: opaque(id),
                    content_sha256: entry
                        .digest
                        .clone()
                        .try_into()
                        .map_err(|_| Failure::invalid())?,
                    representation: Representation::Original,
                    encoded_bytes: entry.length,
                    decoded_bytes: None,
                    dimensions: None,
                    color_space: ColorSpace::NotApplicable,
                    chunks: vec![],
                    capabilities: capabilities(false),
                })
            };
            if preview {
                if entry.length > 48 * 1024 * 1024 {
                    return Err(Failure(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        ErrorCode::ResourceLimit,
                    ));
                }
                let mut bytes = Vec::with_capacity(entry.length as usize);
                file.read_to_end(&mut bytes).map_err(|_| {
                    Failure(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::StorageUnavailable,
                    )
                })?;
                let limits = lytronaut_format::Limits {
                    dimension: 2048,
                    pixels: 2048 * 2048,
                    decoded_bytes: 2048 * 2048 * 4,
                    decoder_working_bytes: 96 * 1024 * 1024,
                    ..Default::default()
                };
                let decoded = lytronaut_format::decode_jpeg(&bytes, limits)
                    .map_err(|e| Failure(StatusCode::UNPROCESSABLE_ENTITY, e.error_code()))?;
                let m = manifest.as_mut().ok_or(Failure(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    ErrorCode::CorruptAsset,
                ))?;
                m.representation = Representation::JpegPreview;
                m.dimensions = Some(Dimensions {
                    width: decoded.geometry.width,
                    height: decoded.geometry.height,
                });
                m.decoded_bytes = Some(decoded.rgba.len() as u64);
                m.color_space = ColorSpace::Unknown;
                m.capabilities = capabilities(true);
                let mut offset = 0;
                while offset < entry.length {
                    let length = (entry.length - offset).min(CHUNK);
                    m.chunks.push(ChunkRange { offset, length });
                    offset += length;
                }
                file.seek(SeekFrom::Start(0)).map_err(|_| {
                    Failure(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::StorageUnavailable,
                    )
                })?;
            }
            if let Some(m) = &manifest {
                m.validate().map_err(|_| {
                    Failure(StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::CorruptAsset)
                })?;
                if serde_json::to_vec(m).map_err(|_| Failure::invalid())?.len() > PAGE_BYTES {
                    return Err(Failure(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        ErrorCode::ResourceLimit,
                    ));
                }
            }
            Ok(Descriptor {
                file: Arc::new(file),
                length: entry.length,
                manifest,
            })
        })
        .await
        .map_err(|_| {
            Failure(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::StorageUnavailable,
            )
        })??;
        let d = Arc::new(descriptor);
        let mut cache = self.0.cache.lock().unwrap();
        if cache.values.len() >= CACHE_RECORDS
            && let Some(old) = cache.order.pop_front()
        {
            cache.values.remove(&old);
        }
        cache.order.push_back(cache_key.clone());
        cache.values.insert(cache_key, d.clone());
        Ok(d)
    }
}
#[derive(Clone)]
struct Browse {
    limit: u32,
    offset: u32,
    search: String,
    sort: String,
}
impl Browse {
    fn parse(query: Option<String>) -> Result<Self> {
        let query = query.unwrap_or_default();
        if query.len() > 1024 {
            return Err(Failure::invalid());
        }
        let mut b = Self {
            limit: 50,
            offset: 0,
            search: String::new(),
            sort: "order".into(),
        };
        let mut seen = std::collections::HashSet::new();
        for pair in query.split('&').filter(|v| !v.is_empty()) {
            let (k, v) = pair.split_once('=').ok_or(Failure::invalid())?;
            if !seen.insert(k) {
                return Err(Failure::invalid());
            }
            let decoded = decode_query(v)?;
            match k {
                "limit" => b.limit = decoded.parse().map_err(|_| Failure::invalid())?,
                "offset" => b.offset = decoded.parse().map_err(|_| Failure::invalid())?,
                "search" => b.search = decoded,
                "sort" => b.sort = decoded,
                _ => return Err(Failure::invalid()),
            }
        }
        if !(1..=100).contains(&b.limit)
            || b.offset > 100_000
            || b.search.len() > 128
            || b.search.chars().any(char::is_control)
            || !matches!(b.sort.as_str(), "order" | "name" | "rating")
        {
            return Err(Failure::invalid());
        }
        Ok(b)
    }
    fn search_pattern(&self) -> String {
        format!(
            "%{}%",
            self.search
                .replace('~', "~~")
                .replace('%', "~%")
                .replace('_', "~_")
        )
    }
}
fn decode_query(v: &str) -> Result<String> {
    let mut out = Vec::with_capacity(v.len());
    let mut i = 0;
    let bytes = v.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let pair = bytes.get(i + 1..i + 3).ok_or(Failure::invalid())?;
                let hex = std::str::from_utf8(pair).map_err(|_| Failure::invalid())?;
                out.push(u8::from_str_radix(hex, 16).map_err(|_| Failure::invalid())?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| Failure::invalid())
}
fn page<T: Serialize>(mut items: Vec<T>, b: &Browse) -> Result<Response> {
    let more = items.len() > b.limit as usize;
    items.truncate(b.limit as usize);
    json_bounded(Page {
        version: WireVersion::V1,
        items,
        offset: b.offset,
        next_offset: more.then_some(b.offset + b.limit),
    })
}
fn json_bounded<T: Serialize>(v: T) -> Result<Response> {
    let bytes = serde_json::to_vec(&v)
        .map_err(|_| Failure(StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal))?;
    if bytes.len() > PAGE_BYTES {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ResourceLimit,
        ));
    }
    Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
}
fn path(
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Result<String> {
    p.map(|Path(v)| v).map_err(|_| Failure::invalid())
}
async fn libraries(Extension(a): Extension<Api>, RawQuery(q): RawQuery) -> Response {
    let result = async {
        let browse = Browse::parse(q)?;
        if !browse.search.is_empty() || browse.sort != "order" {
            return Err(Failure::invalid());
        }
        let rows = sqlx::query(
            "SELECT a.id,v.state,
             (SELECT count(*) FROM catalog_pictures WHERE version=v.id) AS captures,
             (SELECT count(*) FROM catalog_albums WHERE version=v.id) AS albums,
             (SELECT count(*) FROM catalog_sources WHERE version=v.id) AS files
             FROM catalog_versions v JOIN api_ids a ON a.version=v.id AND a.kind='library'
             ORDER BY v.id LIMIT $1 OFFSET $2",
        )
        .bind(i64::from(browse.limit) + 1)
        .bind(i64::from(browse.offset))
        .fetch_all(a.0.catalog.pool())
        .await?;
        let items = rows
            .into_iter()
            .map(|row| LibrarySummary {
                id: opaque(row.get("id")),
                state: state(&row.get::<String, _>("state")),
                captures: row.get::<i64, _>("captures") as u32,
                albums: row.get::<i64, _>("albums") as u32,
                files: row.get::<i64, _>("files") as u32,
            })
            .collect();
        page(items, &browse)
    }
    .await;
    result.unwrap_or_else(Failure::response)
}
async fn albums(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
    RawQuery(q): RawQuery,
) -> Response {
    let result = async {
        let browse = Browse::parse(q)?;
        if browse.sort == "rating" {
            return Err(Failure::invalid());
        }
        let (version, _) = a.lookup(&path(p)?, "library", true).await?;
        let rows = sqlx::query(
            "SELECT a.id,substr(c.name,1,256) AS name,
             length(CAST(c.name AS BLOB))>256 AS truncated,c.sort_order,
             (SELECT count(*) FROM catalog_memberships
              WHERE version=c.version AND album_uuid=c.uuid) AS captures
             FROM catalog_albums c JOIN api_ids a
              ON a.version=c.version AND a.source_key=c.uuid AND a.kind='album'
             WHERE c.version=$1 AND c.name LIKE $2 ESCAPE '~'
             ORDER BY CASE WHEN $3='name' THEN c.name END COLLATE NOCASE,c.sort_order,a.id
             LIMIT $4 OFFSET $5",
        )
        .bind(version)
        .bind(browse.search_pattern())
        .bind(&browse.sort)
        .bind(i64::from(browse.limit) + 1)
        .bind(i64::from(browse.offset))
        .fetch_all(a.0.catalog.pool())
        .await?;
        let items = rows
            .into_iter()
            .map(|row| {
                let (name, truncated) = text_bounded(row.get("name"), 256);
                AlbumSummary {
                    id: opaque(row.get("id")),
                    name,
                    name_truncated: truncated || row.get::<i64, _>("truncated") != 0,
                    sort_order: row.get("sort_order"),
                    captures: row.get::<i64, _>("captures") as u32,
                }
            })
            .collect();
        page(items, &browse)
    }
    .await;
    result.unwrap_or_else(Failure::response)
}
fn capture_summary(row: &SqliteRow) -> CaptureSummary {
    let (name, truncated) = text_bounded(row.get("name"), 256);
    CaptureSummary {
        id: opaque(row.get("id")),
        name,
        name_truncated: truncated || row.get::<i64, _>("truncated") != 0,
        rating: row.get("rating"),
        flag_status: row.get("flag_status"),
        picture_index: row.get("picture_index"),
    }
}
async fn capture_page(a: &Api, id: &str, album: bool, q: Option<String>) -> Result<Response> {
    let browse = Browse::parse(q)?;
    let (version, key) = a
        .lookup(id, if album { "album" } else { "library" }, true)
        .await?;
    let rows = sqlx::query(
        "SELECT a.id,substr(c.name,1,256) AS name,
         length(CAST(c.name AS BLOB))>256 AS truncated,c.rating,c.flag_status,m.picture_index
         FROM catalog_pictures c JOIN api_ids a
          ON a.version=c.version AND a.source_key=c.uuid AND a.kind='capture'
         LEFT JOIN catalog_memberships m
          ON m.version=c.version AND m.picture_uuid=c.uuid AND m.album_uuid=$1
         WHERE c.version=$2 AND ($3=0 OR m.album_uuid IS NOT NULL)
          AND (c.name LIKE $4 ESCAPE '~' OR c.caption LIKE $5 ESCAPE '~')
         ORDER BY CASE WHEN $6='name' THEN c.name END COLLATE NOCASE,
          CASE WHEN $7='rating' THEN c.rating END DESC,
          CASE WHEN $8='order' THEN m.picture_index END,a.id LIMIT $9 OFFSET $10",
    )
    .bind(key)
    .bind(version)
    .bind(album as i64)
    .bind(browse.search_pattern())
    .bind(browse.search_pattern())
    .bind(&browse.sort)
    .bind(&browse.sort)
    .bind(&browse.sort)
    .bind(i64::from(browse.limit) + 1)
    .bind(i64::from(browse.offset))
    .fetch_all(a.0.catalog.pool())
    .await?;
    page(rows.iter().map(capture_summary).collect(), &browse)
}
async fn captures(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
    RawQuery(q): RawQuery,
) -> Response {
    let result = async { capture_page(&a, &path(p)?, false, q).await }.await;
    result.unwrap_or_else(Failure::response)
}
async fn album_captures(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
    RawQuery(q): RawQuery,
) -> Response {
    let result = async { capture_page(&a, &path(p)?, true, q).await }.await;
    result.unwrap_or_else(Failure::response)
}
async fn files(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
    RawQuery(q): RawQuery,
) -> Response {
    let result = async {
        let browse = Browse::parse(q)?;
        if !browse.search.is_empty() || browse.sort != "order" {
            return Err(Failure::invalid());
        }
        let (version, _) = a.lookup(&path(p)?, "library", false).await?;
        let rows = sqlx::query(
            "SELECT a.id,s.path,s.length FROM catalog_sources s JOIN api_ids a
             ON a.version=s.version AND a.source_key=s.path AND a.kind='file'
             WHERE s.version=$1 ORDER BY a.id LIMIT $2 OFFSET $3",
        )
        .bind(version)
        .bind(i64::from(browse.limit) + 1)
        .bind(i64::from(browse.offset))
        .fetch_all(a.0.catalog.pool())
        .await?;
        let items = rows
            .into_iter()
            .map(|row| PreservedFile {
                id: opaque(row.get("id")),
                encoded_bytes: row.get::<i64, _>("length") as u64,
                role: role(&row.get::<String, _>("path")),
            })
            .collect();
        page(items, &browse)
    }
    .await;
    result.unwrap_or_else(Failure::response)
}
async fn detail(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let result = async {
        let id = path(p)?;
        let (version, key) = a.lookup(&id, "capture", true).await?;
        let row = sqlx::query(
            "SELECT a.id,substr(c.name,1,256) AS name,
             length(CAST(c.name AS BLOB))>256 AS truncated,c.rating,c.flag_status,
             NULL AS picture_index,substr(c.caption,1,1024) AS caption,
             length(CAST(c.caption AS BLOB))>1024 AS caption_truncated,c.bin_uuid
             FROM catalog_pictures c JOIN api_ids a
              ON a.version=c.version AND a.source_key=c.uuid AND a.kind='capture'
             WHERE c.version=$1 AND c.uuid=$2",
        )
        .bind(version)
        .bind(&key)
        .fetch_one(a.0.catalog.pool())
        .await?;
        // Source identities/prefixes are validated private catalog data, never request paths.
        let database: String = sqlx::query_scalar(
            "SELECT path FROM catalog_sources WHERE version=$1
             AND (path='lytrolibrary.db' OR path LIKE '%/lytrolibrary.db') LIMIT 1",
        )
        .bind(version)
        .fetch_one(a.0.catalog.pool())
        .await?;
        let prefix = format!(
            "{}{}/{}/",
            database
                .strip_suffix("lytrolibrary.db")
                .ok_or(Failure::missing())?,
            row.get::<String, _>("bin_uuid"),
            key
        );
        let sources = sqlx::query(
            "SELECT a.id,s.path,s.length FROM catalog_sources s JOIN api_ids a
              ON a.version=s.version AND a.source_key=s.path AND a.kind='file'
             WHERE s.version=$1 AND substr(s.path,1,$2)=$3 ORDER BY a.id LIMIT 257",
        )
        .bind(version)
        .bind(prefix.chars().count() as i64)
        .bind(prefix)
        .fetch_all(a.0.catalog.pool())
        .await?;
        if sources.len() > 256 {
            return Err(Failure(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::ResourceLimit,
            ));
        }
        let mut preview = None;
        let mut preview_error = Some(api_error(ErrorCode::NotFound));
        let mut originals = Vec::new();
        let mut discrete_focus = CapabilityState::Disabled;
        for source in sources {
            let file_id: String = source.get("id");
            let source_role = role(&source.get::<String, _>("path"));
            originals.push(PreservedFile {
                id: opaque(file_id.clone()),
                encoded_bytes: source.get::<i64, _>("length") as u64,
                role: source_role,
            });
            if source_role == OriginalRole::JpegFocusStack {
                discrete_focus = match a.focus(&file_id, None).await {
                    Ok(_) => CapabilityState::Available,
                    Err(failure) => CapabilityState::Unavailable(failure.1),
                };
            }
            if source_role == OriginalRole::Thumbnail {
                match a.descriptor(&file_id, true).await {
                    Ok(descriptor)
                        if descriptor
                            .manifest
                            .as_ref()
                            .is_some_and(|m| m.representation == Representation::JpegPreview) =>
                    {
                        preview = Some(opaque(file_id));
                        preview_error = None;
                    }
                    Err(failure) => preview_error = Some(api_error(failure.1)),
                    _ => {}
                }
            }
        }
        let missing = sqlx::query(
            "SELECT code FROM catalog_item_errors WHERE version=$1 AND picture_uuid=$2 LIMIT 32",
        )
        .bind(version)
        .bind(key)
        .fetch_all(a.0.catalog.pool())
        .await?;
        let item_errors = missing
            .iter()
            .map(|row| ItemError {
                role: role(
                    row.get::<String, _>("code")
                        .strip_prefix("missing_")
                        .unwrap_or(""),
                ),
                error: api_error(ErrorCode::NotFound),
            })
            .collect();
        let (caption, truncated) = text_bounded(row.get("caption"), 1024);
        json_bounded(CaptureDetail {
            version: WireVersion::V1,
            capture: capture_summary(&row),
            caption,
            caption_truncated: truncated || row.get::<i64, _>("caption_truncated") != 0,
            capabilities: {
                let mut c = capabilities(preview.is_some());
                c.discrete_focus = discrete_focus;
                c
            },
            preview,
            preview_error,
            item_errors,
            originals,
        })
    }
    .await;
    result.unwrap_or_else(Failure::response)
}
async fn manifest(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r = async {
        let d = a.descriptor(&path(p)?, true).await?;
        json_bounded(d.manifest.as_ref().ok_or(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnsupportedFormat,
        ))?)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
fn focus_limits() -> lytronaut_format::Limits {
    lytronaut_format::Limits {
        container_bytes: 8 * 1024 * 1024,
        json_bytes: PAGE_BYTES,
        sections: 256,
        json_nodes: 16_384,
        dimension: 512,
        pixels: 512 * 512,
        decoded_bytes: 512 * 512 * 4,
        decoder_working_bytes: 64 * 1024 * 1024,
        jpeg_bytes: CHUNK as usize,
        ..Default::default()
    }
}
impl Api {
    async fn focus(
        &self,
        id: &str,
        selection: Option<(usize, u8)>,
    ) -> Result<(FocusStack, Vec<u8>)> {
        let (_, key) = self.lookup(id, "file", true).await?;
        if role(&key) != OriginalRole::JpegFocusStack {
            return Err(Failure::missing());
        }
        let d = self.descriptor(id, false).await?;
        if d.length > focus_limits().container_bytes as u64 {
            return Err(Failure(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::ResourceLimit,
            ));
        }
        let permit = self
            .0
            .validation
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Failure::missing())?;
        let id = opaque(id.to_owned());
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut bytes = vec![0; d.length as usize];
            d.file.read_exact_at(&mut bytes, 0).map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, ErrorCode::StorageUnavailable))?;
            let limits = focus_limits();
            let error = |e: lytronaut_format::FormatError| Failure(StatusCode::UNPROCESSABLE_ENTITY, e.error_code());
            let container = lytronaut_format::Container::parse(&bytes, limits).map_err(error)?;
            let processed = lytronaut_format::processed_asset(&container, limits).map_err(error)?;
            let planes = &processed.planes;
            let first = planes.first().ok_or(Failure::missing())?;
            if planes.len() > 64 || planes.iter().any(|p| p.representation != lytronaut_format::Representation::Jpeg || p.geometry != first.geometry || p.block_index != first.block_index) {
                return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::UnsupportedFormat));
            }
            let stack = FocusStack { version: WireVersion::V1, id,
                lambdas: planes.iter().map(|p| p.lambda.unwrap_or(f64::NAN)).collect(),
                source_dimensions: Dimensions { width: first.geometry.width, height: first.geometry.height },
                orientation_policy: "Processed recipe may be baked; default identity. Explicit viewer EXIF transform only; catalog orientation and sidecar edits preserved.".into() };
            stack.validate().map_err(|_| Failure::invalid())?;
            let output = if let Some((index, orientation)) = selection {
                let plane = planes.get(index).ok_or(Failure::missing())?;
                let decoded = plane.decode_jpeg(limits).map_err(error)?;
                let oriented = lytronaut_format::orient_derivative(decoded, orientation, limits).map_err(error)?;
                lytronaut_format::encode_derivative(&oriented, lytronaut_format::Limits { jpeg_bytes: 2 * 1024 * 1024, ..limits }).map_err(error)?
            } else { Vec::new() };
            Ok((stack, output))
        }).await.map_err(|_| Failure(StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Internal))?
    }
}
async fn focus_manifest(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r = async {
        let (stack, _) = a.focus(&path(p)?, None).await?;
        json_bounded(stack)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
struct DerivativeBody {
    bytes: Option<Bytes>,
    _permit: OwnedSemaphorePermit,
}
impl HttpBody for DerivativeBody {
    type Data = Bytes;
    type Error = std::io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Bytes>, Self::Error>>> {
        Poll::Ready(self.bytes.take().map(|b| Ok(http_body::Frame::data(b))))
    }
    fn is_end_stream(&self) -> bool {
        self.bytes.is_none()
    }
}
async fn focus_plane(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<(String, String, String)>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r = async {
        let Path((id, index, orientation)) = p.map_err(|_| Failure::invalid())?;
        let index = index.parse().map_err(|_| Failure::invalid())?;
        let orientation: u8 = orientation.parse().map_err(|_| Failure::invalid())?;
        if !(1..=8).contains(&orientation) {
            return Err(Failure::invalid());
        }
        let permit =
            a.0.transfers
                .clone()
                .try_acquire_owned()
                .map_err(|_| Failure(StatusCode::TOO_MANY_REQUESTS, ErrorCode::ResourceLimit))?;
        let (stack, bytes) = a.focus(&id, Some((index, orientation))).await?;
        let source = stack.source_dimensions.clone();
        let output = if orientation >= 5 { Dimensions { width: source.height, height: source.width } } else { source.clone() };
        let derivative = FocusDerivative { version: WireVersion::V1, representation: Representation::JpegFocusPlane, id: stack.id.clone(),
            plane_index: index, lambda: stack.lambdas[index], source_dimensions: source,
            output_dimensions: output, orientation, encoded_bytes: bytes.len() as u64,
            content_sha256: format!("{:x}", Sha256::digest(&bytes)).try_into().map_err(|_| Failure::invalid())?,
            color_space: ColorSpace::Unknown,
            provenance: "Lossy JPEG re-encode of stored JPEG plane; explicit viewer EXIF transform. Processed recipe may be baked; imported recipe/sidecar/catalog orientation preserved, never replayed.".into() };
        derivative.validate(&stack).map_err(|_| Failure(StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::ResourceLimit))?;
        let metadata = serde_json::to_string(&derivative).map_err(|_| Failure::invalid())?;
        if metadata.len() > PAGE_BYTES { return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::ResourceLimit)); }
        let length = bytes.len();
        let mut response = Body::new(DerivativeBody {
            bytes: Some(bytes.into()),
            _permit: permit,
        })
        .into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, "image/jpeg".parse().unwrap());
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());
        response.headers_mut().insert("X-Lytronaut-Derivative", metadata.parse().map_err(|_| Failure::invalid())?);
        Ok(response)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
/// File body owns its admission permit until EOF, error, or drop. No whole-file buffering.
struct FileBody {
    file: Arc<std::fs::File>,
    offset: u64,
    remaining: u64,
    permit: Option<OwnedSemaphorePermit>,
    pending: Option<tokio::task::JoinHandle<std::io::Result<Vec<u8>>>>,
}
impl HttpBody for FileBody {
    type Data = Bytes;
    type Error = std::io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<std::result::Result<http_body::Frame<Bytes>, Self::Error>>> {
        if self.remaining == 0 {
            self.permit.take();
            return Poll::Ready(None);
        }
        if self.pending.is_none() {
            let file = self.file.clone();
            let offset = self.offset;
            let length = self.remaining.min(64 * 1024) as usize;
            self.pending = Some(tokio::task::spawn_blocking(move || {
                let mut bytes = vec![0; length];
                let mut n = 0;
                while n < length {
                    let got = file.read_at(&mut bytes[n..], offset + n as u64)?;
                    if got == 0 {
                        return Err(std::io::ErrorKind::UnexpectedEof.into());
                    }
                    n += got;
                }
                Ok(bytes)
            }));
        }
        match Pin::new(self.pending.as_mut().unwrap()).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                self.pending.take();
                match result {
                    Ok(Ok(bytes)) => {
                        self.remaining -= bytes.len() as u64;
                        self.offset += bytes.len() as u64;
                        Poll::Ready(Some(Ok(http_body::Frame::data(Bytes::from(bytes)))))
                    }
                    _ => {
                        self.permit.take();
                        self.remaining = 0;
                        Poll::Ready(Some(Err(std::io::ErrorKind::Other.into())))
                    }
                }
            }
        }
    }
    fn is_end_stream(&self) -> bool {
        self.remaining == 0
    }
}
async fn file_response(
    a: &Api,
    id: &str,
    range: Option<ChunkRange>,
    preview_required: bool,
) -> Result<Response> {
    let permit =
        a.0.transfers
            .clone()
            .try_acquire_owned()
            .map_err(|_| Failure(StatusCode::TOO_MANY_REQUESTS, ErrorCode::ResourceLimit))?;
    let d = a.descriptor(id, preview_required).await?;
    if preview_required
        && !d
            .manifest
            .as_ref()
            .is_some_and(|m| m.representation == Representation::JpegPreview)
    {
        return Err(Failure::invalid());
    }
    let offset = range.as_ref().map(|r| r.offset).unwrap_or(0);
    let length = range.as_ref().map(|r| r.length).unwrap_or(d.length);
    if offset.checked_add(length).is_none_or(|end| end > d.length) {
        return Err(Failure::invalid());
    }
    let content_type = if preview_required {
        "image/jpeg"
    } else {
        "application/octet-stream"
    };
    let mut response = Body::new(FileBody {
        file: d.file.clone(),
        offset,
        remaining: length,
        permit: Some(permit),
        pending: None,
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type.parse().unwrap());
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());
    if !preview_required {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{id}.bin\"")
                .parse()
                .unwrap(),
        );
    }
    Ok(response)
}
async fn download(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
    headers: axum::http::HeaderMap,
) -> Response {
    let r = async {
        if headers.contains_key(header::RANGE) {
            return Err(Failure::invalid());
        }
        let id = path(p)?;
        file_response(&a, &id, None, false).await
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
async fn chunk(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<(String, String)>, axum::extract::rejection::PathRejection>,
    headers: axum::http::HeaderMap,
) -> Response {
    let r = async {
        if headers.contains_key(header::RANGE) {
            return Err(Failure::invalid());
        }
        let Path((id, index)) = p.map_err(|_| Failure::invalid())?;
        let index: usize = index.parse().map_err(|_| Failure::invalid())?;
        let d = a.descriptor(&id, true).await?;
        let range = d
            .manifest
            .as_ref()
            .and_then(|m| m.chunks.get(index))
            .cloned()
            .ok_or(Failure::missing())?;
        range.validate().map_err(|_| Failure::invalid())?;
        file_response(&a, &id, Some(range), true).await
    }
    .await;
    r.unwrap_or_else(Failure::response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::{io::Cursor, os::unix::fs::DirBuilderExt};
    #[tokio::test]
    async fn transfer_permits_cover_final_frame_drop_and_independent_offsets() {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root =
            std::env::temp_dir().join(format!("lytronaut-body-test-{:x}", u128::from_le_bytes(b)));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let catalog = Catalog::open_application_database(&root.join("catalog.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let bytes: Vec<u8> = (0..150_000).map(|i| (i % 251) as u8).collect();
        let job = store
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: ImportKind::Directory,
                declared_bytes: bytes.len() as u64,
                declared_files: 1,
            })
            .await
            .unwrap();
        store
            .append_directory_file(&job, "unknown.bin", Cursor::new(bytes.clone()))
            .await
            .unwrap();
        store.finish(&job).await.unwrap();
        let api = Api::open(catalog.clone(), store).await.unwrap();
        let id: String = sqlx::query_scalar("SELECT id FROM api_ids WHERE kind='file'")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        let mut first = file_response(&api, &id, None, false)
            .await
            .unwrap()
            .into_body();
        let mut second = file_response(&api, &id, None, false)
            .await
            .unwrap()
            .into_body();
        assert_eq!(api.0.transfers.available_permits(), 0);
        assert_eq!(
            file_response(&api, &id, None, false).await.unwrap_err().0,
            StatusCode::TOO_MANY_REQUESTS
        );
        let frame = first.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(frame.as_ref(), &bytes[..65536]);
        assert_eq!(api.0.transfers.available_permits(), 0);
        let mut other = Vec::new();
        while let Some(frame) = second.frame().await {
            other.extend(frame.unwrap().into_data().unwrap());
        }
        assert_eq!(other, bytes);
        assert_eq!(api.0.transfers.available_permits(), 1);
        // A body with one final data frame still owns the permit until EOF or drop.
        let mut tail = file_response(
            &api,
            &id,
            Some(ChunkRange {
                offset: 149_990,
                length: 10,
            }),
            false,
        )
        .await
        .unwrap()
        .into_body();
        assert_eq!(api.0.transfers.available_permits(), 0);
        assert_eq!(
            tail.frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap()
                .as_ref(),
            &bytes[149_990..]
        );
        assert_eq!(api.0.transfers.available_permits(), 0);
        drop(tail);
        assert_eq!(api.0.transfers.available_permits(), 1);
        let remainder = first.collect().await.unwrap().to_bytes();
        assert_eq!(remainder.as_ref(), &bytes[65536..]);
        assert_eq!(api.0.transfers.available_permits(), 2);
        let response = file_response(&api, &id, None, false).await.unwrap();
        assert_eq!(api.0.transfers.available_permits(), 1);
        drop(response);
        assert_eq!(api.0.transfers.available_permits(), 2);
        assert!(
            file_response(
                &api,
                &id,
                Some(ChunkRange {
                    offset: u64::MAX,
                    length: 2
                }),
                false
            )
            .await
            .is_err()
        );
        assert_eq!(api.0.transfers.available_permits(), 2);
        drop(api);
        drop(catalog);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bounded_literal_search_and_wire_page() {
        assert!(Browse::parse(Some(format!("search={}", "a".repeat(1025)))).is_err());
        assert!(Browse::parse(Some("search=%00".into())).is_err());
        let b = Browse::parse(Some("search=%25_&limit=100".into())).unwrap();
        assert_eq!(b.search_pattern(), "%~%~_%");
        assert!(json_bounded("x".repeat(PAGE_BYTES)).is_err());
    }
    #[tokio::test]
    async fn recovery_isolates_source_failure_durably_and_retries_same_intake() {
        use std::os::unix::fs::PermissionsExt;
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-recovery-test-{:x}",
            u128::from_le_bytes(b)
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let catalog = Catalog::open_application_database(&root.join("catalog.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        async fn intake(s: &Store, bytes: &[u8]) -> String {
            let id = s
                .begin(&ImportRequest {
                    version: WireVersion::V1,
                    kind: ImportKind::Directory,
                    declared_bytes: bytes.len() as u64,
                    declared_files: 1,
                })
                .await
                .unwrap();
            s.append_directory_file(&id, "unknown.bin", Cursor::new(bytes.to_vec()))
                .await
                .unwrap();
            s.finish(&id).await.unwrap();
            id
        }
        let bad = intake(&store, b"synthetic fault recovery original").await;
        let good = intake(&store, b"other independent preserved bytes").await;
        let entry = store.manifest(&bad).await.unwrap().remove(0);
        let blob = root
            .join("originals")
            .join(format!("{}-{}", entry.digest, entry.length));
        std::fs::remove_file(&blob).unwrap();
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        assert_eq!(
            progress(&catalog, &store, &bad).await.unwrap().phase,
            ImportPhase::CatalogFailed
        );
        assert!(
            progress(&catalog, &store, &bad)
                .await
                .unwrap()
                .catalog_error
                .unwrap()
                .retryable
        );
        assert_eq!(
            progress(&catalog, &store, &good)
                .await
                .unwrap()
                .catalog_state,
            Some(LibraryState::PreservedOnly)
        );
        drop(api);
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        assert_eq!(
            progress(&catalog, &store, &bad).await.unwrap().phase,
            ImportPhase::CatalogFailed
        );
        std::fs::write(&blob, b"synthetic fault recovery original").unwrap();
        std::fs::set_permissions(&blob, std::fs::Permissions::from_mode(0o400)).unwrap();
        adapt(&catalog, &store, &bad).await.unwrap();
        let report = progress(&catalog, &store, &bad).await.unwrap();
        assert_eq!(report.phase, ImportPhase::CatalogComplete);
        assert!(report.catalog_error.is_none());
        assert_eq!(report.catalog_state, Some(LibraryState::PreservedOnly));
        // A global API schema failure must not be disguised as per-job recovery success.
        sqlx::raw_sql("DROP TABLE api_adaptation_errors;CREATE VIEW api_adaptation_errors AS SELECT 'invalid' AS job").execute(catalog.pool()).await.unwrap();
        assert!(Api::open(catalog.clone(), store.clone()).await.is_err());
        drop(api);
        drop(store);
        drop(catalog);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn catalog_publication_waits_for_an_existing_writer() {
        let root = std::env::temp_dir().join(random_id().unwrap());
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .unwrap();
        let catalog = Catalog::open_application_database(&root.join("catalog.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let job = store
            .begin(&ImportRequest {
                version: WireVersion::V1,
                kind: ImportKind::Directory,
                declared_bytes: 3,
                declared_files: 1,
            })
            .await
            .unwrap();
        store
            .append_directory_file(&job, "unknown.bin", Cursor::new(b"abc".to_vec()))
            .await
            .unwrap();
        store.finish(&job).await.unwrap();
        migrate(&catalog).await.unwrap();
        let version = desktop::import_desktop(&catalog, &store, &job)
            .await
            .unwrap()
            .version_id;
        let mut writer = catalog.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("UPDATE catalog_versions SET reason=reason WHERE id=$1")
            .bind(version)
            .execute(&mut *writer)
            .await
            .unwrap();
        let c = catalog.clone();
        let j = job.clone();
        let publishing = tokio::spawn(async move { map_version(&c, &j, version).await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !publishing.is_finished(),
            "publication must wait for writer, not fail its read-to-write upgrade"
        );
        writer.commit().await.unwrap();
        publishing.await.unwrap().unwrap();
        assert_eq!(
            progress(&catalog, &store, &job).await.unwrap().phase,
            ImportPhase::CatalogComplete
        );
        drop(store);
        drop(catalog);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod derivative_body_tests {
    use super::*;
    use http_body_util::BodyExt;
    #[tokio::test]
    async fn derivative_permit_covers_emitted_frame_eof_and_drop() {
        let permits = Arc::new(Semaphore::new(2));
        let mut body = DerivativeBody {
            bytes: Some(Bytes::from_static(b"synthetic")),
            _permit: permits.clone().acquire_owned().await.unwrap(),
        };
        assert_eq!(permits.available_permits(), 1);
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            Bytes::from_static(b"synthetic")
        );
        assert_eq!(permits.available_permits(), 1);
        assert!(body.frame().await.is_none());
        drop(body);
        assert_eq!(permits.available_permits(), 2);
        let body = DerivativeBody {
            bytes: Some(Bytes::from_static(b"cancel")),
            _permit: permits.clone().acquire_owned().await.unwrap(),
        };
        drop(body);
        assert_eq!(permits.available_permits(), 2);
    }
}
