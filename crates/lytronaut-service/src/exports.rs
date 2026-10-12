//! Durable deployment-protected export/isolated restore jobs. Shares viewer codec and transfer admission.
use super::*;
use crate::bundle;
use axum::{Json, extract::Request, routing::post};
use http_body_util::BodyExt;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
const OUTPUT_CAP: usize = 8 * 1024 * 1024;
static RECONCILE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
fn root(store: &Store) -> PathBuf {
    store.staging_root().with_extension("exports")
}
fn artifact(a: &Api, id: &str) -> PathBuf {
    crate::transient_owner::Owner::directory(&root(&a.0.store), id).join("artifact")
}
fn storage(_: impl std::fmt::Debug) -> Failure {
    Failure(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::StorageUnavailable,
    )
}
pub(super) async fn migrate(c: &Catalog, s: &Store) -> Result<()> {
    crate::local::private_directory(&root(s)).map_err(storage)?;
    let mut tx = c.begin_write().await?;
    for &sql in c.schema_statements(lytronaut_storage::SchemaComponent::Export) {
        sqlx::query(sql).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    c.backfill_job_order(lytronaut_storage::SchemaComponent::Export)
        .await?;
    // Reconcile durable SQL/filesystem ownership BEFORE generic interruption
    // cleanup. Denied/disconnected cleanup retains the journal and reservation.
    // Only whole-store failures (unreadable export root or journal, database
    // unavailable) refuse startup; one unprovable job is blocked, not fatal.
    recover_restores(c, s).await?;
    sqlx::query("UPDATE export_jobs SET state='failed',error='interrupted' WHERE state IN ('queued','running','cancelling')").execute(c.pool()).await?;
    // Artifacts of interrupted jobs are never offered. Owned files only, no source mutation.
    // A previously ready result that failed verification is never removed
    // automatically; only an explicit DELETE (with ownership proof) may.
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM export_jobs WHERE state IN ('failed','cancelled') AND (error IS NULL OR error<>'unverified-result')")
        .fetch_all(c.pool())
        .await?;
    for id in ids {
        if OpaqueId::try_from(id.clone()).is_ok() {
            // A restore result without a journal is not guessed-owned by its
            // filename. New restore creation always journals before any side effect.
            if cleanup_result(c, s, &id).await.is_err() {
                store_available(c, s).await?;
                block(c, &id).await?;
            }
        }
    }
    Ok(())
}
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/exports", post(begin))
        .route("/exports/{id}", get(status).delete(cancel))
        .route("/exports/{id}/download", get(download))
        .route("/exports/{id}/recipe", get(recipe))
        .route("/restores", post(restore_upload))
}
async fn begin(
    Extension(a): Extension<Api>,
    payload: std::result::Result<Json<ExportRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let r = async {
        let Json(request) = payload.map_err(|_| Failure::invalid())?;
        request.validate().map_err(|_| Failure::invalid())?;
        if let ExportRequest::Flatten {
            asset, plane_index, ..
        } = &request
        {
            a.lookup(asset.as_str(), "file", true).await?;
            if plane_index.is_none() {
                a.descriptor(asset.as_str(), true).await?;
            }
        }
        let need=if matches!(request, ExportRequest::ArchiveOriginals{..}) {let n:i64=sqlx::query_scalar("SELECT CAST(COALESCE(sum(length),0) AS BIGINT) FROM (SELECT digest,length FROM intake_sources GROUP BY digest,length) AS unique_sources").fetch_one(a.0.catalog.pool()).await?; u64::try_from(n).map_err(|_| Failure::invalid())?.checked_add(2 * lytronaut_storage::portable::MAX_BYTES as u64).and_then(|n| n.checked_add(bundle::archive_framing(a.0.store.limits().files).ok()?)).ok_or(Failure::invalid())?}else{OUTPUT_CAP as u64};
        let id = create(
            &a,
            if matches!(request, ExportRequest::ArchiveOriginals { .. }) {
                "archive"
            } else {
                "flatten"
            }, need,
        )
        .await?;
        let worker = a.clone();
        let job = id.clone();
        tokio::spawn(async move {
            run(worker, job, request, None).await;
        });
        json_bounded(public_report(&a, &id).await?)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
async fn create(a: &Api, kind: &str, need: u64) -> Result<String> {
    // Reserve atomically before spawning; at most two pending jobs, including uploads.
    let mut tx = a.0.catalog.begin_write().await?;
    sqlx::query("DELETE FROM export_jobs WHERE id IN (SELECT id FROM export_jobs WHERE state IN ('cancelled','failed') AND retained=0 AND reserved=0 ORDER BY (SELECT ordinal FROM application_job_order WHERE kind='export' AND job=export_jobs.id) LIMIT (SELECT CASE WHEN count(*)>127 THEN count(*)-127 ELSE 0 END FROM export_jobs))").execute(&mut *tx).await?;
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM export_jobs WHERE state IN ('queued','running','cancelling')",
    )
    .fetch_one(&mut *tx)
    .await?;
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM export_jobs")
        .fetch_one(&mut *tx)
        .await?;
    if active >= 2 || retained >= 128 {
        return Err(Failure(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::ResourceLimit,
        ));
    }
    let reserved: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(sum(retained+reserved),0) AS BIGINT) FROM export_jobs",
    )
    .fetch_one(&mut *tx)
    .await?;
    let active_reserved: i64 =
        sqlx::query_scalar("SELECT CAST(COALESCE(sum(reserved),0) AS BIGINT) FROM export_jobs")
            .fetch_one(&mut *tx)
            .await?;
    if need.saturating_add(reserved as u64) > a.0.store.limits().reserved_bytes
        || a.0.store.available_staging_bytes().map_err(storage)?
            < need
                .saturating_add(active_reserved as u64)
                .saturating_add(a.0.store.limits().free_floor)
    {
        return Err(Failure(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::ResourceLimit,
        ));
    }
    let id = random_id()?;
    sqlx::query("INSERT INTO export_jobs(id,kind,state,reserved) VALUES($1,$2,'queued',$3)")
        .bind(&id)
        .bind(kind)
        .bind(need as i64)
        .execute(&mut *tx)
        .await?;
    a.0.catalog
        .record_job_order(&mut tx, lytronaut_storage::SchemaComponent::Export, &id)
        .await?;
    sqlx::query("DELETE FROM application_job_order WHERE kind='export' AND job NOT IN (SELECT id FROM export_jobs)").execute(&mut *tx).await?;
    tx.commit().await?;
    if let Err(error) = crate::transient_owner::Owner::prepare(&root(&a.0.store), &id) {
        sqlx::query("UPDATE export_jobs SET state='failed',error='storage' WHERE id=$1")
            .bind(&id)
            .execute(a.0.catalog.pool())
            .await?;
        // Persisted intent (if any) and quota survive initial marker/mkdir errors.
        return Err(storage(error));
    }
    Ok(id)
}
async fn report(a: &Api, id: &str) -> Result<JobReport> {
    if OpaqueId::try_from(id.to_owned()).is_err() {
        return Err(Failure::invalid());
    }
    let r = sqlx::query("SELECT state,completed,total,error FROM export_jobs WHERE id=$1")
        .bind(id)
        .fetch_optional(a.0.catalog.pool())
        .await?
        .ok_or(Failure::missing())?;
    let state = match r.try_get::<String, _>(0)?.as_str() {
        "queued" => JobState::Queued,
        "running" => JobState::Running,
        "cancelling" => JobState::Cancelling,
        "cancelled" => JobState::Cancelled,
        "succeeded" => JobState::Succeeded,
        _ => JobState::Failed,
    };
    let completed = r.try_get::<i64, _>(1)? as u64;
    let total = r.try_get::<i64, _>(2)? as u64;
    Ok(JobReport {
        version: WireVersion::V1,
        id: opaque(id.to_owned()),
        state,
        completed_bytes: completed,
        total_bytes: total.max(completed),
        error: if state == JobState::Failed {
            Some(ApiError {
                version: WireVersion::V1,
                code: match r.try_get::<Option<String>, _>(3)?.as_deref() {
                    Some("unsupported") => ErrorCode::UnsupportedFormat,
                    Some("cleanup-blocked" | "unverified-result") => ErrorCode::StorageUnavailable,
                    _ => ErrorCode::CorruptAsset,
                },
                retryable: true,
            })
        } else {
            None
        },
    })
}
async fn public_report(a: &Api, id: &str) -> Result<ExportProgress> {
    let job = report(a, id).await?;
    let kind: String = sqlx::query_scalar("SELECT kind FROM export_jobs WHERE id=$1")
        .bind(id)
        .fetch_one(a.0.catalog.pool())
        .await?;
    let kind = match kind.as_str() {
        "flatten" => ExportKind::Flatten,
        "archive" => ExportKind::Archive,
        "restore" => ExportKind::Restore,
        _ => return Err(Failure::invalid()),
    };
    Ok(ExportProgress { job, kind })
}
async fn status(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r = async { json_bounded(public_report(&a, &path(p)?).await?) }.await;
    r.unwrap_or_else(Failure::response)
}
async fn cleanup_result(catalog: &Catalog, store: &Store, id: &str) -> Result<()> {
    let directory = root(store);
    let mut journal = crate::restore_journal::Journal::load(&directory, id).map_err(storage)?;
    if let Some(journal) = journal.as_mut() {
        journal
            .cleanup(catalog, &directory)
            .await
            .map_err(storage)?;
    }
    if journal.is_none() {
        // Without a journal no restore result or pending store is provably ours.
        // Releasing quota while such a path remains would leak its bytes, and
        // deleting it could destroy unrelated data: fail closed, retain quota.
        let pending = format!("{id}.pending-");
        for entry in fs::read_dir(&directory).map_err(storage)? {
            let name = entry.map_err(storage)?.file_name();
            let name = name.to_string_lossy();
            if name == format!("{id}.restored") || name.starts_with(&pending) {
                return Err(Failure::invalid());
            }
        }
    }
    let transient = crate::transient_owner::Owner::load(&directory, id).map_err(storage)?;
    if let Some(owner) = transient.as_ref() {
        owner.cleanup(&directory).map_err(storage)?;
    } else {
        // Neither an old filename nor an unmarked directory proves ownership.
        // Fail closed across startup, DELETE and worker finalization.
        for path in [
            crate::transient_owner::Owner::directory(&directory, id),
            directory.join(format!("{id}.artifact")),
            directory.join(format!("{id}.snapshot")),
            directory.join(format!("{id}.upload")),
        ] {
            match fs::symlink_metadata(path) {
                Ok(_) => return Err(Failure::invalid()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(storage(e)),
            }
        }
    }
    fs::File::open(&directory)
        .map_err(storage)?
        .sync_all()
        .map_err(storage)?;
    // No journal means no restore-created SQL namespace or directory remains
    // (checked above); only then is the reservation released.
    let mut tx = catalog.begin_write().await?;
    // A previously blocked job is now clean; its original reason was replaced
    // by the blocked marker, so it is reported as interrupted from here on.
    sqlx::query("UPDATE export_jobs SET retained=0,reserved=0,error=CASE WHEN error IN ('cleanup-blocked','unverified-result') THEN 'interrupted' ELSE error END WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if let Some(journal) = journal {
        journal
            .forget_after_quota_release(&directory)
            .map_err(storage)?;
    }
    if let Some(owner) = transient {
        owner.forget(&directory).map_err(storage)?;
    }
    Ok(())
}
async fn recover_restores(catalog: &Catalog, store: &Store) -> Result<()> {
    let _guard = RECONCILE.lock().await;
    let directory = root(store);
    for entry in fs::read_dir(&directory).map_err(storage)? {
        let entry = entry.map_err(storage)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".restore-journal") else {
            continue;
        };
        if OpaqueId::try_from(id.to_owned()).is_err() {
            // Not a name this application writes: foreign, left untouched.
            eprintln!("Lytronaut ignored a foreign restore-journal name in the export root");
            continue;
        }
        // An unreadable or malformed journal is a whole-store failure.
        let Some(mut journal) =
            crate::restore_journal::Journal::load(&directory, id).map_err(storage)?
        else {
            continue;
        };
        let row = sqlx::query("SELECT state,error FROM export_jobs WHERE id=$1")
            .bind(id)
            .fetch_optional(catalog.pool())
            .await?;
        let (state, error) = match row {
            Some(row) => (
                Some(row.try_get::<String, _>(0)?),
                row.try_get::<Option<String>, _>(1)?,
            ),
            None => (None, None),
        };
        if error.as_deref() == Some("unverified-result") {
            // Retained until an explicit DELETE; never cleaned automatically.
            continue;
        }
        if state.as_deref() == Some("succeeded")
            && matches!(
                journal.phase(),
                crate::restore_journal::Phase::FilesPublished
                    | crate::restore_journal::Phase::JobSucceeded
            )
        {
            let verified = match journal.verify_published(&directory) {
                Ok(()) => match journal.namespace() {
                    Some(namespace) => namespace.verify(catalog).await,
                    None => Ok(()),
                },
                Err(error) => Err(error),
            };
            if verified.is_err() {
                store_available(catalog, store).await?;
                // Never offer an unverifiable result as ready, and never delete
                // a previously ready result without an explicit DELETE.
                mark_unverified(catalog, id).await?;
                continue;
            }
            // A failed journal fsync here is retried at the next startup; the
            // database success and verified published files are durable.
            let _ = journal.transition(&directory, crate::restore_journal::Phase::JobSucceeded);
        } else {
            // Keep an already recorded cancellation/failure reason; only a job
            // that never durably finished is marked interrupted.
            sqlx::query("UPDATE export_jobs SET state=CASE WHEN state='cancelled' THEN state ELSE 'failed' END,error=CASE WHEN state IN ('cancelled','failed') THEN error ELSE 'interrupted' END WHERE id=$1")
                .bind(id).execute(catalog.pool()).await?;
            if cleanup_result(catalog, store, id).await.is_err() {
                store_available(catalog, store).await?;
                block(catalog, id).await?;
            }
        }
    }
    Ok(())
}
/// Distinguish a per-job failure from a whole-store outage after a cleanup
/// error: the export root must be readable and the database reachable.
async fn store_available(catalog: &Catalog, store: &Store) -> Result<()> {
    fs::read_dir(root(store)).map_err(storage)?;
    catalog.healthy().await?;
    Ok(())
}
/// Persist a visible blocked state for a job whose owned cleanup cannot be
/// proved or completed. Files, journal and quota are all retained; DELETE and
/// every startup retry it. A blocked job is failed, so it is never ready.
/// Failing to persist the blocked state is itself fatal to startup: a job that
/// cannot be durably marked non-ready must not be left in its previous state.
async fn block(catalog: &Catalog, id: &str) -> Result<()> {
    eprintln!(
        "Lytronaut job {id}: owned cleanup is blocked; its files, journal and quota are retained. Resolve the reported conflict, then DELETE the job or restart"
    );
    let persisted =
        sqlx::query("UPDATE export_jobs SET state='failed',error='cleanup-blocked' WHERE id=$1")
            .bind(id)
            .execute(catalog.pool())
            .await;
    if persisted.is_err() {
        eprintln!("Lytronaut job {id}: the blocked state could not be persisted");
    }
    persisted?;
    Ok(())
}
/// A ready result that no longer verifies becomes visibly non-ready with all of
/// it retained. Unlike a blocked cleanup it is not retried automatically: the
/// user's restored copy is removed only by an explicit DELETE.
async fn mark_unverified(catalog: &Catalog, id: &str) -> Result<()> {
    eprintln!(
        "Lytronaut job {id}: its published restore result failed verification and is no longer ready; it is retained until DELETE"
    );
    let persisted =
        sqlx::query("UPDATE export_jobs SET state='failed',error='unverified-result' WHERE id=$1")
            .bind(id)
            .execute(catalog.pool())
            .await;
    if persisted.is_err() {
        eprintln!("Lytronaut job {id}: the unverified state could not be persisted");
    }
    persisted?;
    Ok(())
}
/// Runtime cleanup (DELETE, worker finalization, failed upload). The caller's
/// job is already durably failed or cancelled, so it is never ready even when
/// the blocked marker cannot be written; the cleanup error is returned.
async fn reconcile_or_block(catalog: &Catalog, store: &Store, id: &str) -> Result<()> {
    let result = cleanup_result(catalog, store, id).await;
    if result.is_err() {
        let _ = block(catalog, id).await;
    }
    result
}
async fn remove_owned(a: &Api, id: &str) -> Result<()> {
    let _guard = RECONCILE.lock().await;
    reconcile_or_block(&a.0.catalog, &a.0.store, id).await
}

async fn cancel(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r=async {
        let id=path(p)?;report(&a,&id).await?;
        sqlx::query("UPDATE export_jobs SET state=CASE WHEN state IN ('queued','running') THEN 'cancelling' WHEN state='succeeded' THEN 'cancelled' ELSE state END WHERE id=$1").bind(&id).execute(a.0.catalog.pool()).await?;
        // Cancelled and failed (including cleanup-blocked) jobs retry owned cleanup.
        if matches!(report(&a,&id).await?.state,JobState::Cancelled|JobState::Failed) {
            remove_owned(&a,&id).await?;
            sqlx::query("UPDATE export_jobs SET retained=0,reserved=0 WHERE id=$1").bind(&id).execute(a.0.catalog.pool()).await?;
        }
        json_bounded(public_report(&a,&id).await?)
    }.await;
    r.unwrap_or_else(Failure::response)
}
async fn recipe(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r = async {
        let id = path(p)?;
        if report(&a, &id).await?.state != JobState::Succeeded {
            return Err(Failure::missing());
        }
        let value: Option<String> =
            sqlx::query_scalar("SELECT recipe FROM export_jobs WHERE id=$1")
                .bind(id)
                .fetch_one(a.0.catalog.pool())
                .await?;
        let recipe: ExportRecipe =
            serde_json::from_str(&value.ok_or(Failure::missing())?).map_err(storage)?;
        json_bounded(recipe)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
async fn download(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r = async {
        let id = path(p)?;
        if report(&a, &id).await?.state != JobState::Succeeded {
            return Err(Failure::missing());
        }
        let permit =
            a.0.transfers
                .clone()
                .try_acquire_owned()
                .map_err(|_| Failure(StatusCode::TOO_MANY_REQUESTS, ErrorCode::ResourceLimit))?;
        let owner = crate::transient_owner::Owner::load(&root(&a.0.store), &id)
            .map_err(storage)?
            .ok_or_else(Failure::invalid)?;
        owner.verify(&root(&a.0.store)).map_err(storage)?;
        let f = fs::File::open(artifact(&a, &id)).map_err(storage)?;
        let length = f.metadata().map_err(storage)?.len();
        let value: Option<String> =
            sqlx::query_scalar("SELECT recipe FROM export_jobs WHERE id=$1")
                .bind(&id)
                .fetch_one(a.0.catalog.pool())
                .await?;
        let (mime, ext) = if let Some(s) = value {
            let recipe: ExportRecipe = serde_json::from_str(&s).map_err(storage)?;
            if recipe.format == ExportFormat::Png {
                ("image/png", "png")
            } else {
                ("image/jpeg", "jpg")
            }
        } else {
            ("application/octet-stream", "lytronaut")
        };
        let mut response = Body::new(FileBody {
            file: Arc::new(f),
            offset: 0,
            remaining: length,
            permit: Some(permit),
            pending: None,
        })
        .into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, mime.parse().unwrap());
        response
            .headers_mut()
            .insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{id}.{ext}\"")
                .parse()
                .unwrap(),
        );
        Ok(response)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}
struct WorkerGuard(Api);
impl WorkerGuard {
    fn new(api: &Api) -> Self {
        api.0.exports_inflight.fetch_add(1, Ordering::AcqRel);
        Self(api.clone())
    }
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.0.exports_inflight.fetch_sub(1, Ordering::AcqRel);
    }
}
async fn run(a: Api, id: String, request: ExportRequest, restore: Option<PathBuf>) {
    let _worker_guard = WorkerGuard::new(&a);
    let cancelled = Arc::new(AtomicBool::new(false));
    let initial = restore
        .as_ref()
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let bytes = Arc::new(AtomicU64::new(initial));
    let retained = Arc::new(AtomicU64::new(0));
    let poll_a = a.clone();
    let poll_id = id.clone();
    let flag = cancelled.clone();
    let progress = bytes.clone();
    let monitor = tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            match report(&poll_a, &poll_id).await {
                Ok(r) => {
                    if matches!(
                        r.state,
                        JobState::Cancelling | JobState::Cancelled | JobState::Failed
                    ) {
                        flag.store(true, Ordering::Release)
                    }
                }
                Err(_) => flag.store(true, Ordering::Release),
            }
            let n = progress.load(Ordering::Relaxed) as i64;
            let _=sqlx::query("UPDATE export_jobs SET completed=$1,total=CASE WHEN total>$2 THEN total ELSE $2 END WHERE id=$3 AND state IN ('queued','running','cancelling')").bind(n).bind(n).bind(&poll_id).execute(poll_a.0.catalog.pool()).await;
        }
    });
    let result = async {
        sqlx::query("UPDATE export_jobs SET state='running' WHERE id=$1 AND state='queued'")
            .bind(&id)
            .execute(a.0.catalog.pool())
            .await?;
        let permit = if matches!(request, ExportRequest::ArchiveOriginals { .. }) {
            Some(
                a.0.validation
                    .clone()
                    .acquire_owned()
                    .await
                    .map_err(storage)?,
            )
        } else {
            None
        };
        if cancelled.load(Ordering::Acquire) || report(&a, &id).await?.state == JobState::Cancelling
        {
            return Err(Failure::invalid());
        }
        let rt = tokio::runtime::Handle::current();
        let worker = a.clone();
        let job = id.clone();
        let flag = cancelled.clone();
        let progress = bytes.clone();
        let stored = retained.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            rt.block_on(async move {
                if let Some(input) = restore {
                    let reserved: i64 =
                        sqlx::query_scalar("SELECT reserved FROM export_jobs WHERE id=$1")
                            .bind(&job)
                            .fetch_one(worker.0.catalog.pool())
                            .await?;
                    let result = crate::bundle::restore_owned(
                        &input,
                        &worker.0.catalog,
                        crate::bundle::RestoreOwner {
                            root: &root(&worker.0.store),
                            job: &job,
                            reserved: u64::try_from(reserved).map_err(storage)?,
                        },
                        worker.0.store.limits(),
                        &flag,
                        &progress,
                    )
                    .await
                    .map_err(storage);
                    let _ = fs::remove_file(input);
                    let result = result?;
                    stored.store(result.bytes, Ordering::Relaxed);
                    return Ok(None);
                }
                match request {
                    ExportRequest::ArchiveOriginals { .. } => {
                        crate::bundle::backup(
                            &worker.0.catalog,
                            &worker.0.store,
                            &artifact(&worker, &job),
                            &flag,
                            &progress,
                        )
                        .await
                        .map_err(storage)?;
                        stored.store(
                            fs::metadata(artifact(&worker, &job))
                                .map_err(storage)?
                                .len(),
                            Ordering::Relaxed,
                        );
                        Ok(None)
                    }
                    ExportRequest::Flatten {
                        asset,
                        format,
                        plane_index,
                        orientation,
                        ..
                    } => {
                        let (output, recipe) =
                            flatten(&worker, asset, format, plane_index, orientation).await?;
                        if flag.load(Ordering::Acquire) {
                            return Err(Failure::invalid());
                        }
                        let mut f = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(artifact(&worker, &job))
                            .map_err(storage)?;
                        f.write_all(&output).map_err(storage)?;
                        f.sync_all().map_err(storage)?;
                        fs::File::open(root(&worker.0.store))
                            .map_err(storage)?
                            .sync_all()
                            .map_err(storage)?;
                        progress.store(output.len() as u64, Ordering::Relaxed);
                        stored.store(output.len() as u64, Ordering::Relaxed);
                        Ok(Some(serde_json::to_string(&recipe).map_err(storage)?))
                    }
                }
            })
        })
        .await
        .map_err(storage)?
    }
    .await;
    monitor.abort();
    let _ = monitor.await;
    let cancelling = report(&a, &id)
        .await
        .is_ok_and(|r| matches!(r.state, JobState::Cancelling | JobState::Cancelled));
    let (state, error, recipe) = if cancelling || cancelled.load(Ordering::Acquire) {
        ("cancelled", None, None)
    } else {
        match result {
            Ok(recipe) => ("succeeded", None, recipe),
            Err(f) => (
                "failed",
                Some(if f.1 == ErrorCode::UnsupportedFormat {
                    "unsupported"
                } else {
                    "failed"
                }),
                None,
            ),
        }
    };
    let n = bytes.load(Ordering::Relaxed) as i64;
    let stored = if state == "succeeded" {
        retained.load(Ordering::Relaxed) as i64
    } else {
        0
    };
    // Keep reservations on failed/cancelled transitions until owned cleanup
    // succeeds. Concurrent DELETE cannot resurrect success or free quota early.
    let saved = sqlx::query("UPDATE export_jobs SET state=CASE WHEN state='cancelling' THEN 'cancelled' ELSE $1 END,error=CASE WHEN state='cancelling' THEN NULL ELSE $2 END,recipe=$3,completed=$4,total=$5,retained=CASE WHEN state='cancelling' THEN 0 ELSE $6 END,reserved=CASE WHEN state!='cancelling' AND $1='succeeded' THEN 0 ELSE reserved END WHERE id=$7 AND state IN ('queued','running','cancelling')")
        .bind(state).bind(error).bind(recipe).bind(n).bind(n).bind(stored).bind(&id).execute(a.0.catalog.pool()).await;
    if saved.is_err() {
        eprintln!("Lytronaut export status could not be persisted; owned recovery is required");
        // Best effort status transition; never erase ownership/quota when the
        // database is unreachable or a final UPDATE failed after publication.
        // The error may also be ambiguous (the commit landed but the client saw
        // a failure), so the decision below re-reads the durable state.
        let _ = sqlx::query("UPDATE export_jobs SET state='failed',error='interrupted' WHERE id=$1 AND state IN ('queued','running','cancelling')")
            .bind(&id).execute(a.0.catalog.pool()).await;
    }
    // Act only on the durably observed state: keep a committed success, remove
    // owned output only for a durable failure/cancellation, and otherwise leave
    // ownership, files and quota to startup reconciliation.
    match report(&a, &id).await {
        Ok(report) if report.state == JobState::Succeeded => {
            let _guard = RECONCILE.lock().await;
            if let Ok(Some(mut journal)) =
                crate::restore_journal::Journal::load(&root(&a.0.store), &id)
            {
                // A failed journal fsync is retryable startup reconciliation;
                // database success and owned published files are already durable.
                let _ = journal.transition(
                    &root(&a.0.store),
                    crate::restore_journal::Phase::JobSucceeded,
                );
            }
        }
        Ok(report) if matches!(report.state, JobState::Failed | JobState::Cancelled) => {
            let _ = remove_owned(&a, &id).await;
        }
        Ok(_) | Err(_) => {
            eprintln!("Lytronaut export reconciliation is pending database recovery");
        }
    }
}

async fn flatten(
    a: &Api,
    asset: OpaqueId,
    format: ExportFormat,
    index: Option<usize>,
    orientation: u8,
) -> Result<(Vec<u8>, ExportRecipe)> {
    let limits = lytronaut_format::Limits {
        dimension: 2048,
        pixels: 2048 * 2048,
        decoded_bytes: 2048 * 2048 * 4,
        decoder_working_bytes: 80 * 1024 * 1024,
        jpeg_bytes: if format == ExportFormat::Jpeg {
            4 * 1024 * 1024
        } else {
            OUTPUT_CAP
        },
        container_bytes: 8 * 1024 * 1024,
        ..Default::default()
    };
    let d = a.descriptor(asset.as_str(), index.is_none()).await?;
    let _permit =
        a.0.validation
            .clone()
            .acquire_owned()
            .await
            .map_err(storage)?;
    // Shared CPU admission covers read/decode/orient/encode after descriptor validation.
    let source_cap = if index.is_some() {
        8 * 1024 * 1024
    } else {
        16 * 1024 * 1024
    };
    if d.length > source_cap {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ResourceLimit,
        ));
    }
    let mut source = vec![0; d.length as usize];
    d.file.read_exact_at(&mut source, 0).map_err(storage)?;
    let err = |e: lytronaut_format::FormatError| {
        Failure(StatusCode::UNPROCESSABLE_ENTITY, e.error_code())
    };
    let (decoded, lambda, variant) = if let Some(i) = index {
        let (_, key) = a.lookup(asset.as_str(), "file", true).await?;
        if role(&key) != OriginalRole::JpegFocusStack {
            return Err(Failure(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::UnsupportedFormat,
            ));
        }
        let c = lytronaut_format::Container::parse(&source, focus_limits()).map_err(err)?;
        let p = lytronaut_format::processed_asset(&c, focus_limits()).map_err(err)?;
        let first = p.planes.first().ok_or(Failure::missing())?;
        if p.planes.len() > 64
            || p.planes.iter().any(|v| {
                v.representation != lytronaut_format::Representation::Jpeg
                    || v.geometry != first.geometry
                    || v.block_index != first.block_index
            })
        {
            return Err(Failure(
                StatusCode::UNPROCESSABLE_ENTITY,
                ErrorCode::UnsupportedFormat,
            ));
        }
        let plane = p.planes.get(i).ok_or(Failure::missing())?;
        (
            plane.decode_jpeg(focus_limits()).map_err(err)?,
            plane.lambda,
            "stored_jpeg_focus_plane",
        )
    } else {
        (
            lytronaut_format::decode_jpeg(&source, limits).map_err(err)?,
            None,
            "verified_static_jpeg",
        )
    };
    drop(source);
    let source_dimensions = Dimensions {
        width: decoded.geometry.width,
        height: decoded.geometry.height,
    };
    let oriented =
        lytronaut_format::orient_derivative(decoded, orientation, limits).map_err(err)?;
    let output_dimensions = Dimensions {
        width: oriented.geometry.width,
        height: oriented.geometry.height,
    };
    let output = match format {
        ExportFormat::Png => lytronaut_format::encode_png_derivative(&oriented, limits),
        ExportFormat::Jpeg => lytronaut_format::encode_derivative(&oriented, limits),
    }
    .map_err(err)?;
    let recipe=ExportRecipe{version:WireVersion::V1,asset,source_variant:variant.into(),plane_index:index,lambda,orientation,source_dimensions,visible_geometry:[0,0,output_dimensions.width,output_dimensions.height],output_dimensions,format,color_space:ColorSpace::Unknown,encoded_bytes:output.len() as u64,content_sha256:format!("{:x}",Sha256::digest(&output)).try_into().map_err(storage)?,color_policy:"Decoded channel values retained without color conversion; ICC/profile and EXIF metadata are not copied; source color remains unknown.".into(),processing_loss:match format{ExportFormat::Png=>"PNG preserves decoded stored JPEG pixels; original JPEG was already lossy; profile unverified",ExportFormat::Jpeg=>"Lossy quality-90 JPEG re-encode of decoded stored JPEG pixels; profile unverified"}.into(),refocus_loss:true,imported_adjustments:"Processed recipe may already be baked; imported recipe/sidecar/catalog orientation preserved only. No aperture, perspective, tone, denoise, animation or calibrated synthesis applied.".into()};
    Ok((output, recipe))
}
async fn restore_upload(Extension(a): Extension<Api>, request: Request) -> Response {
    let r = async {
        let _transfer =
            a.0.transfers
                .clone()
                .try_acquire_owned()
                .map_err(|_| Failure(StatusCode::TOO_MANY_REQUESTS, ErrorCode::ResourceLimit))?;
        let declared = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|n| *n > 0 && bundle::archive_limit(a.0.store.limits()).is_ok_and(|maximum| *n <= maximum))
            .ok_or(Failure::invalid())?;
        // Upload + extracted originals/catalog spool coexist. Reserve the v2
        // spool separately; failed cleanup retains this reservation.
        let reservation = declared.checked_mul(2)
            .and_then(|n| n.checked_add(lytronaut_storage::portable::MAX_BYTES as u64))
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(Failure::invalid())?;
        let id = create(&a, "restore", reservation).await?;
        let path = crate::transient_owner::Owner::directory(&root(&a.0.store), &id).join("upload");
        let mut upload_created = false;
        let result = async {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .await
                .map_err(storage)?;
            upload_created = true;
            let mut body = request.into_body();
            let mut n = 0u64;
            use tokio::io::AsyncWriteExt;
            let mut tick=tokio::time::interval(std::time::Duration::from_millis(100));
            loop {
                tokio::select! {
                    frame=body.frame()=>{
                        let Some(frame)=frame else{break};let frame=frame.map_err(storage)?;
                        if let Ok(data)=frame.into_data(){n=n.checked_add(data.len() as u64).ok_or(Failure::invalid())?;if n>declared{return Err(Failure(StatusCode::PAYLOAD_TOO_LARGE,ErrorCode::ResourceLimit))}f.write_all(&data).await.map_err(storage)?;}
                    }
                    _=tick.tick()=>{
                        if matches!(report(&a,&id).await?.state,JobState::Cancelling|JobState::Cancelled){return Err(Failure(StatusCode::CONFLICT,ErrorCode::InvalidInput))}
                        sqlx::query("UPDATE export_jobs SET completed=$1,total=$2 WHERE id=$3 AND state='queued'").bind(n as i64).bind(reservation as i64).bind(&id).execute(a.0.catalog.pool()).await?;
                    }
                }
            }
            if n!=declared{return Err(Failure::invalid())}
            if matches!(report(&a,&id).await?.state,JobState::Cancelling|JobState::Cancelled){return Err(Failure(StatusCode::CONFLICT,ErrorCode::InvalidInput))}
            f.sync_all().await.map_err(storage)?;
            Ok(())
        }
        .await;
        if let Err(e) = result {
            sqlx::query("UPDATE export_jobs SET state=CASE WHEN state='cancelling' THEN 'cancelled' ELSE 'failed' END,error=CASE WHEN state='cancelling' THEN NULL ELSE 'upload' END WHERE id=$1")
                .bind(&id)
                .execute(a.0.catalog.pool())
                .await?;
            if upload_created {
                // Cleanup failure intentionally leaves ownership/reservation for
                // retry. Never clear quota after a best-effort unlink.
                remove_owned(&a, &id).await?;
            }
            // A create_new collision is not proof we own the pre-existing file;
            // retain the reservation and fail closed rather than unlink it.
            return Err(e);
        }
        let worker = a.clone();
        let job = id.clone();
        tokio::spawn(async move {
            run(
                worker,
                job,
                ExportRequest::ArchiveOriginals {
                    version: WireVersion::V1,
                },
                Some(path),
            )
            .await;
        });
        json_bounded(public_report(&a, &id).await?)
    }
    .await;
    r.unwrap_or_else(Failure::response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn unowned_collision_survives_restart_and_delete_with_quota_retained() {
        let dir = std::env::temp_dir().join(random_id().unwrap());
        crate::local::private_directory(&dir).unwrap();
        let catalog = Catalog::open_application_database(&dir.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &dir.join("originals"),
            &dir.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        let id = random_id().unwrap();
        // Independent pre-existing fixed-name upload and transfer-directory
        // sentinels: neither was created by the application ownership protocol.
        let sentinel = root(&store).join(format!("{id}.upload"));
        fs::write(&sentinel, b"unrelated upload").unwrap();
        let directory = crate::transient_owner::Owner::directory(&root(&store), &id);
        crate::local::private_directory(&directory).unwrap();
        fs::write(directory.join("sentinel"), b"unrelated directory").unwrap();
        sqlx::query(
            "INSERT INTO export_jobs(id,kind,state,reserved) VALUES($1,'restore','failed',4096)",
        )
        .bind(&id)
        .execute(catalog.pool())
        .await
        .unwrap();
        assert!(crate::transient_owner::Owner::prepare(&root(&store), &id).is_err());
        // Startup proceeds; only this job is blocked, with its quota retained.
        migrate(&catalog, &store).await.unwrap();
        assert_eq!(report(&api, &id).await.unwrap().state, JobState::Failed);
        // DELETE's central cleanup path must fail closed on the same proof.
        assert!(remove_owned(&api, &id).await.is_err());
        assert_eq!(fs::read(&sentinel).unwrap(), b"unrelated upload");
        assert_eq!(
            fs::read(directory.join("sentinel")).unwrap(),
            b"unrelated directory"
        );
        let reserved: i64 = sqlx::query_scalar("SELECT reserved FROM export_jobs WHERE id=$1")
            .bind(&id)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(reserved, 4096);
        drop(api);
        catalog.pool().close().await;
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn interrupted_directory_cleanup_after_marker_unlink_retains_quota() {
        let dir = std::env::temp_dir().join(random_id().unwrap());
        crate::local::private_directory(&dir).unwrap();
        let catalog = Catalog::open_application_database(&dir.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &dir.join("originals"),
            &dir.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        let id = create(&api, "restore", 4096).await.unwrap();
        let directory = crate::transient_owner::Owner::directory(&root(&store), &id);
        fs::write(directory.join("upload"), b"remaining owned upload").unwrap();
        // Independent crash cut during recursive removal: marker has already
        // vanished, but other contents and the durable ownership intent remain.
        fs::remove_file(directory.join("owner")).unwrap();
        // Retained indefinitely (intentional), but startup is not refused: the
        // job is blocked and visible instead.
        migrate(&catalog, &store).await.unwrap();
        let error: Option<String> = sqlx::query_scalar("SELECT error FROM export_jobs WHERE id=$1")
            .bind(&id)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(error.as_deref(), Some("cleanup-blocked"));
        assert!(remove_owned(&api, &id).await.is_err());
        assert_eq!(
            fs::read(directory.join("upload")).unwrap(),
            b"remaining owned upload"
        );
        assert!(
            crate::transient_owner::Owner::load(&root(&store), &id)
                .unwrap()
                .is_some()
        );
        let reserved: i64 = sqlx::query_scalar("SELECT reserved FROM export_jobs WHERE id=$1")
            .bind(&id)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(reserved, 4096);
        drop(api);
        catalog.pool().close().await;
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn uploaded_owned_file_recovers_before_restore_journal_exists() {
        let dir = std::env::temp_dir().join(random_id().unwrap());
        crate::local::private_directory(&dir).unwrap();
        let catalog = Catalog::open_application_database(&dir.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &dir.join("originals"),
            &dir.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        let id = create(&api, "restore", 4096).await.unwrap();
        let directory = crate::transient_owner::Owner::directory(&root(&store), &id);
        fs::write(directory.join("upload"), b"synthetic completed upload").unwrap();
        assert!(
            crate::restore_journal::Journal::load(&root(&store), &id)
                .unwrap()
                .is_none()
        );
        migrate(&catalog, &store).await.unwrap();
        assert!(!directory.exists());
        let reserved: i64 = sqlx::query_scalar("SELECT reserved FROM export_jobs WHERE id=$1")
            .bind(&id)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        assert_eq!(reserved, 0);
        migrate(&catalog, &store).await.unwrap();
        drop(api);
        catalog.pool().close().await;
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn service_shutdown_cancels_a_queued_background_archive_before_publish() {
        let dir = std::env::temp_dir().join(random_id().unwrap());
        crate::local::private_directory(&dir).unwrap();
        let c = Catalog::open_application_database(&dir.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            c.clone(),
            &dir.join("originals"),
            &dir.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let a = Api::open(c.clone(), store).await.unwrap();
        let hold = a.0.validation.clone().acquire_owned().await.unwrap();
        let id = create(&a, "archive", 1).await.unwrap();
        let worker = a.clone();
        let job = id.clone();
        let running = tokio::spawn(async move {
            run(
                worker,
                job,
                ExportRequest::ArchiveOriginals {
                    version: WireVersion::V1,
                },
                None,
            )
            .await;
        });
        let release = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            drop(hold);
        });
        a.shutdown().await.unwrap();
        running.await.unwrap();
        release.await.unwrap();
        assert_eq!(report(&a, &id).await.unwrap().state, JobState::Cancelled);
        assert!(!artifact(&a, &id).exists());
        assert_eq!(a.0.validation.available_permits(), 1);
        a.shutdown().await.unwrap();
        drop(a);
        c.pool().close().await;
        fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn queued_cancel_never_publishes_and_limits_admit_two_only() {
        let dir = std::env::temp_dir().join(random_id().unwrap());
        crate::local::private_directory(&dir).unwrap();
        let c = Catalog::open_application_database(&dir.join("app.db"))
            .await
            .unwrap();
        let store = Store::open(
            c.clone(),
            &dir.join("originals"),
            &dir.join("staging"),
            lytronaut_storage::Limits {
                free_floor: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let a = Api::open(c.clone(), store).await.unwrap();
        let hold = a.0.validation.clone().acquire_owned().await.unwrap();
        let id = create(&a, "archive", 1).await.unwrap();
        let second = create(&a, "archive", 1).await.unwrap();
        assert_eq!(
            create(&a, "archive", 1).await.unwrap_err().0,
            StatusCode::TOO_MANY_REQUESTS
        );
        let worker = a.clone();
        let job = id.clone();
        let running = tokio::spawn(async move {
            run(
                worker,
                job,
                ExportRequest::ArchiveOriginals {
                    version: WireVersion::V1,
                },
                None,
            )
            .await
        });
        sqlx::query("UPDATE export_jobs SET state='cancelling' WHERE id=$1")
            .bind(&id)
            .execute(c.pool())
            .await
            .unwrap();
        drop(hold);
        running.await.unwrap();
        assert_eq!(report(&a, &id).await.unwrap().state, JobState::Cancelled);
        assert!(!artifact(&a, &id).exists());
        sqlx::query("UPDATE export_jobs SET state='running' WHERE id=$1")
            .bind(&second)
            .execute(c.pool())
            .await
            .unwrap();
        fs::write(artifact(&a, &second), b"interrupted partial").unwrap();
        migrate(&c, &a.0.store).await.unwrap();
        assert_eq!(report(&a, &second).await.unwrap().state, JobState::Failed);
        assert!(!artifact(&a, &second).exists());
        let response = restore_upload(
            Extension(a.clone()),
            Request::builder()
                .header(header::CONTENT_LENGTH, "128")
                .body(Body::from(vec![0u8; 64]))
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let failed:i64=sqlx::query_scalar("SELECT count(*) FROM export_jobs WHERE kind='restore' AND state='failed' AND error='upload'").fetch_one(c.pool()).await.unwrap();
        assert_eq!(failed, 1);
        assert_eq!(a.0.validation.available_permits(), 1);
        drop(a);
        c.pool().close().await;
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
#[path = "restore_fault_tests.rs"]
mod restore_fault_tests;
