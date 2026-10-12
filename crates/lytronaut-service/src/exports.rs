//! Durable deployment-protected export/isolated restore jobs. Shares viewer codec and transfer admission.
use super::*;
#[path = "exports/live_photo.rs"]
mod live_photo;
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
fn root(store: &Store) -> PathBuf {
    store.staging_root().with_extension("exports")
}
fn artifact(a: &Api, id: &str) -> PathBuf {
    root(&a.0.store).join(format!("{id}.artifact"))
}
fn storage(_: impl std::fmt::Debug) -> Failure {
    Failure(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::StorageUnavailable,
    )
}
pub(super) async fn migrate(c: &Catalog, s: &Store) -> Result<()> {
    crate::local::private_directory(&root(s)).map_err(storage)?;
    sqlx::query("CREATE TABLE IF NOT EXISTS export_jobs(id TEXT PRIMARY KEY,kind TEXT NOT NULL,state TEXT NOT NULL,completed INTEGER NOT NULL DEFAULT 0,total INTEGER NOT NULL DEFAULT 0,reserved INTEGER NOT NULL DEFAULT 0,retained INTEGER NOT NULL DEFAULT 0,error TEXT,recipe TEXT)").execute(c.pool()).await?;
    sqlx::query("UPDATE export_jobs SET state='failed',error='interrupted' WHERE state IN ('queued','running','cancelling')").execute(c.pool()).await?;
    // Artifacts of interrupted jobs are never offered. Owned files only, no source mutation.
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM export_jobs WHERE state IN ('failed','cancelled')")
            .fetch_all(c.pool())
            .await?;
    for id in ids {
        if OpaqueId::try_from(id.clone()).is_ok() {
            let _ = fs::remove_dir_all(root(s).join(format!("{id}.restored")));
            for suffix in ["artifact", "snapshot", "upload"] {
                let _ = fs::remove_file(root(s).join(format!("{id}.{suffix}")));
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
        if let ExportRequest::LivePhoto{asset,..}=&request {
            let (_,key)=a.lookup(asset.as_str(),"file",true).await?;
            if role(&key)!=OriginalRole::JpegFocusStack {return Err(Failure(StatusCode::UNPROCESSABLE_ENTITY,ErrorCode::UnsupportedFormat));}
            a.descriptor(asset.as_str(),false).await?;
        }
        let need=if matches!(request, ExportRequest::ArchiveOriginals{..}) {let n:i64=sqlx::query_scalar("SELECT COALESCE(sum(length),0) FROM (SELECT digest,length FROM intake_sources GROUP BY digest,length)").fetch_one(a.0.catalog.pool()).await?; (n as u64).checked_add(64*1024*1024).ok_or(Failure::invalid())?}else if matches!(request,ExportRequest::LivePhoto{..}){lytronaut_format::live_photo::BUNDLE_CAP as u64}else{OUTPUT_CAP as u64};
        let id = create(
            &a,
            if matches!(request, ExportRequest::ArchiveOriginals { .. }) {
                "archive"
            } else if matches!(request,ExportRequest::LivePhoto{..}) {
                "live_photo"
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
    let mut tx = a.0.catalog.pool().begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("DELETE FROM export_jobs WHERE id IN (SELECT id FROM export_jobs WHERE state IN ('cancelled','failed') AND retained=0 ORDER BY rowid LIMIT (SELECT max(count(*)-127,0) FROM export_jobs))").execute(&mut *tx).await?;
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
    let reserved:i64=sqlx::query_scalar("SELECT COALESCE(sum(retained+CASE WHEN state IN ('queued','running','cancelling') THEN reserved ELSE 0 END),0) FROM export_jobs").fetch_one(&mut *tx).await?;
    let active_reserved:i64=sqlx::query_scalar("SELECT COALESCE(sum(reserved),0) FROM export_jobs WHERE state IN ('queued','running','cancelling')").fetch_one(&mut *tx).await?;
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
    sqlx::query("INSERT INTO export_jobs(id,kind,state,reserved) VALUES(?,?,'queued',?)")
        .bind(&id)
        .bind(kind)
        .bind(need as i64)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(id)
}
async fn report(a: &Api, id: &str) -> Result<JobReport> {
    if OpaqueId::try_from(id.to_owned()).is_err() {
        return Err(Failure::invalid());
    }
    let r = sqlx::query("SELECT state,completed,total,error FROM export_jobs WHERE id=?")
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
                code: if r.try_get::<Option<String>, _>(3)?.as_deref() == Some("unsupported") {
                    ErrorCode::UnsupportedFormat
                } else {
                    ErrorCode::CorruptAsset
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
    let kind: String = sqlx::query_scalar("SELECT kind FROM export_jobs WHERE id=?")
        .bind(id)
        .fetch_one(a.0.catalog.pool())
        .await?;
    let kind = match kind.as_str() {
        "live_photo" => ExportKind::LivePhoto,
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
fn remove_owned(a: &Api, id: &str) -> Result<()> {
    match fs::remove_file(artifact(a, id)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(storage(e)),
    }
    match fs::remove_dir_all(root(&a.0.store).join(format!("{id}.restored"))) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(storage(e)),
    }
    Ok(())
}
async fn cancel(
    Extension(a): Extension<Api>,
    p: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let r=async {
        let id=path(p)?;report(&a,&id).await?;
        sqlx::query("UPDATE export_jobs SET state=CASE WHEN state IN ('queued','running') THEN 'cancelling' WHEN state='succeeded' THEN 'cancelled' ELSE state END WHERE id=?").bind(&id).execute(a.0.catalog.pool()).await?;
        if report(&a,&id).await?.state==JobState::Cancelled {
            remove_owned(&a,&id)?;
            sqlx::query("UPDATE export_jobs SET retained=0,reserved=0 WHERE id=?").bind(&id).execute(a.0.catalog.pool()).await?;
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
        let value: Option<String> = sqlx::query_scalar("SELECT recipe FROM export_jobs WHERE id=?")
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
        let f = fs::File::open(artifact(&a, &id)).map_err(storage)?;
        let length = f.metadata().map_err(storage)?.len();
        let value: Option<String> = sqlx::query_scalar("SELECT recipe FROM export_jobs WHERE id=?")
            .bind(&id)
            .fetch_one(a.0.catalog.pool())
            .await?;
        let kind: String = sqlx::query_scalar("SELECT kind FROM export_jobs WHERE id=?")
            .bind(&id)
            .fetch_one(a.0.catalog.pool())
            .await?;
        let (mime, ext) = if kind == "live_photo" {
            ("application/zip", "zip")
        } else if let Some(s) = value {
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
async fn run(a: Api, id: String, request: ExportRequest, restore: Option<PathBuf>) {
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
            let _=sqlx::query("UPDATE export_jobs SET completed=?,total=max(total,?) WHERE id=? AND state IN ('queued','running','cancelling')").bind(n).bind(n).bind(&poll_id).execute(poll_a.0.catalog.pool()).await;
        }
    });
    let result = async {
        sqlx::query("UPDATE export_jobs SET state='running' WHERE id=? AND state='queued'")
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
                    let expected = crate::bundle::trusted_schema(&worker.0.catalog)
                        .await
                        .map_err(storage)?;
                    let result = crate::bundle::restore(
                        &input,
                        &root(&worker.0.store).join(format!("{job}.restored")),
                        &expected,
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
                    ExportRequest::LivePhoto {
                        asset,
                        plane_index,
                        orientation,
                        ..
                    } => {
                        let output = live_photo::render(
                            &worker,
                            asset,
                            plane_index,
                            orientation,
                            &job,
                            &flag,
                            &progress,
                        )
                        .await?;
                        let mut f = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(artifact(&worker, &job))
                            .map_err(storage)?;
                        for chunk in output.chunks(1024 * 1024) {
                            if flag.load(Ordering::Acquire) {
                                return Err(Failure::invalid());
                            }
                            f.write_all(chunk).map_err(storage)?;
                            progress.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        }
                        f.sync_all().map_err(storage)?;
                        fs::File::open(root(&worker.0.store))
                            .map_err(storage)?
                            .sync_all()
                            .map_err(storage)?;
                        stored.store(output.len() as u64, Ordering::Relaxed);
                        Ok(None)
                    }
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
    if state != "succeeded" {
        let _ = fs::remove_file(artifact(&a, &id));
        let restored = root(&a.0.store).join(format!("{id}.restored"));
        let _ = fs::remove_dir_all(restored);
    }
    let n = bytes.load(Ordering::Relaxed) as i64;
    // Conditional final transition prevents a concurrent DELETE from resurrecting success.
    let saved=sqlx::query("UPDATE export_jobs SET state=CASE WHEN state='cancelling' THEN 'cancelled' ELSE ? END,error=CASE WHEN state='cancelling' THEN NULL ELSE ? END,recipe=?,completed=?,total=?,retained=?,reserved=0 WHERE id=? AND state IN ('queued','running','cancelling')").bind(state).bind(error).bind(recipe).bind(n).bind(n).bind(if state=="succeeded"{retained.load(Ordering::Relaxed) as i64}else{0}).bind(&id).execute(a.0.catalog.pool()).await;
    if saved.is_err() {
        eprintln!("Lytronaut export status could not be persisted; restart recovery is required");
        let _ = fs::remove_file(artifact(&a, &id));
    }
    if report(&a, &id)
        .await
        .is_ok_and(|r| r.state == JobState::Cancelled)
        && remove_owned(&a, &id).is_ok()
    {
        let _ = sqlx::query("UPDATE export_jobs SET retained=0,reserved=0 WHERE id=?")
            .bind(&id)
            .execute(a.0.catalog.pool())
            .await;
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
            .filter(|n| *n > 0 && *n <= a.0.store.limits().encoded_bytes + 32 * 1024 * 1024)
            .ok_or(Failure::invalid())?;
        let id = create(&a, "restore", declared.saturating_mul(2)).await?;
        let path = root(&a.0.store).join(format!("{id}.upload"));
        let result = async {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .await
                .map_err(storage)?;
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
                        sqlx::query("UPDATE export_jobs SET completed=?,total=? WHERE id=? AND state='queued'").bind(n as i64).bind(declared.saturating_mul(2) as i64).bind(&id).execute(a.0.catalog.pool()).await?;
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
            let _ = fs::remove_file(&path);
            sqlx::query("UPDATE export_jobs SET state=CASE WHEN state='cancelling' THEN 'cancelled' ELSE 'failed' END,error=CASE WHEN state='cancelling' THEN NULL ELSE 'upload' END,reserved=0,retained=0 WHERE id=?")
                .bind(&id)
                .execute(a.0.catalog.pool())
                .await?;
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
        sqlx::query("UPDATE export_jobs SET state='cancelling' WHERE id=?")
            .bind(&id)
            .execute(c.pool())
            .await
            .unwrap();
        drop(hold);
        running.await.unwrap();
        assert_eq!(report(&a, &id).await.unwrap().state, JobState::Cancelled);
        assert!(!artifact(&a, &id).exists());
        sqlx::query("UPDATE export_jobs SET state='running' WHERE id=?")
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
