//! Product routes added before the final Host/Origin guard.
use crate::{AppState, error};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use http_body_util::BodyExt;
use lytronaut_contracts::{ErrorCode, ImportRequest};
use lytronaut_storage::{StorageError, Store};
use serde::Serialize;
#[derive(Serialize)]
struct Started {
    version: u16,
    id: String,
}
fn failure(e: StorageError) -> Response {
    // A free-space or inode shortage is retryable and, for space, names the
    // required and available bytes; a declaration beyond the configured
    // upload budget stays a permanent 413 invalid_input.
    match e {
        StorageError::Space {
            required_bytes,
            available_bytes,
        } => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(lytronaut_contracts::ApiError {
                    version: lytronaut_contracts::WireVersion::V1,
                    code: ErrorCode::ResourceLimit,
                    retryable: true,
                    shortfall: Some(lytronaut_contracts::SpaceShortfall {
                        required_bytes,
                        available_bytes,
                    }),
                }),
            )
                .into_response();
        }
        StorageError::Inodes => {
            return error(StatusCode::TOO_MANY_REQUESTS, ErrorCode::ResourceLimit);
        }
        _ => {}
    }
    error(
        match e {
            StorageError::Quota => StatusCode::PAYLOAD_TOO_LARGE,
            StorageError::Invalid => StatusCode::BAD_REQUEST,
            StorageError::Busy => StatusCode::TOO_MANY_REQUESTS,
            StorageError::Conflict | StorageError::Cancelled => StatusCode::CONFLICT,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        },
        match e {
            StorageError::Invalid | StorageError::Quota => ErrorCode::InvalidInput,
            _ => ErrorCode::StorageUnavailable,
        },
    )
}
pub fn routes(store: Store) -> Router<AppState> {
    Router::new()
        .route("/imports", post(begin))
        .route("/imports/{id}", get(job))
        .route("/imports/{id}/file", put(file))
        .route("/imports/{id}/zip", put(zip))
        .route("/imports/{id}/finish", post(finish))
        .route("/imports/{id}/cancel", post(cancel))
        .route("/imports/{id}/retry", post(retry))
        .layer(Extension(store))
}
async fn begin(
    Extension(store): Extension<Store>,
    payload: Result<Json<ImportRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Ok(Json(request)) = payload else {
        return failure(StorageError::Invalid);
    };
    match store.begin(&request).await {
        Ok(id) => (StatusCode::CREATED, Json(Started { version: 1, id })).into_response(),
        Err(e) => failure(e),
    }
}
async fn job(
    State(state): State<AppState>,
    Extension(store): Extension<Store>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let Ok(Path(id)) = path else {
        return failure(StorageError::Invalid);
    };
    match crate::api::progress(&state.catalog, &store, &id).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => failure(e),
    }
}
async fn finish(
    State(state): State<AppState>,
    Extension(store): Extension<Store>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let Ok(Path(id)) = path else {
        return failure(StorageError::Invalid);
    };
    let finished = match store.job(&id).await {
        Ok(job) if job.state == lytronaut_contracts::JobState::Succeeded => Ok(()),
        _ => store.finish(&id).await,
    };
    match finished {
        Ok(()) => match crate::api::adapt(&state.catalog, &store, &id).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(_) => failure(StorageError::Unavailable),
        },
        Err(e) => failure(e),
    }
}
async fn cancel(
    Extension(store): Extension<Store>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let Ok(Path(id)) = path else {
        return failure(StorageError::Invalid);
    };
    match store.cancel(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
async fn retry(
    Extension(store): Extension<Store>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
) -> Response {
    let Ok(Path(id)) = path else {
        return failure(StorageError::Invalid);
    };
    match store.retry(&id).await {
        Ok(id) => (StatusCode::CREATED, Json(Started { version: 1, id })).into_response(),
        Err(e) => failure(e),
    }
}
fn header<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str, StorageError> {
    let mut all = headers.get_all(key).iter();
    let s = all
        .next()
        .ok_or(StorageError::Invalid)?
        .to_str()
        .map_err(|_| StorageError::Invalid)?;
    if all.next().is_some() {
        return Err(StorageError::Invalid);
    }
    Ok(s)
}
/// Canonical lowercase hexadecimal UTF-8 bytes, avoiding HTTP's ASCII header restriction.
fn logical_path(headers: &HeaderMap, store: &Store) -> Result<String, StorageError> {
    let value = header(headers, "x-lytronaut-path-hex")?;
    if value.is_empty()
        || value.len() > store.limits().path_bytes * 2
        || !value.len().is_multiple_of(2)
    {
        return Err(StorageError::Invalid);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            _ => Err(StorageError::Invalid),
        };
        bytes.push(nibble(pair[0])? * 16 + nibble(pair[1])?);
    }
    let p = String::from_utf8(bytes).map_err(|_| StorageError::Invalid)?;
    lytronaut_storage::intake::validate_path(&p, store.limits())?;
    Ok(p)
}
async fn stream(
    store: &Store,
    id: &str,
    headers: &HeaderMap,
    mut body: Body,
    limit: u64,
) -> Result<lytronaut_storage::intake::Spool, StorageError> {
    // Never decompress an HTTP Content-Encoding outside the counted intake boundary.
    if headers.contains_key("content-encoding") {
        return Err(StorageError::Invalid);
    }
    let length = if headers.contains_key("content-length") {
        Some(
            header(headers, "content-length")?
                .parse::<u64>()
                .map_err(|_| StorageError::Invalid)?,
        )
    } else {
        None
    };
    if length.is_some_and(|v| v > limit) {
        return Err(StorageError::Quota);
    }
    let mut spool = store.spool(id).await?;
    spool.restrict(limit);
    let mut actual = 0u64;
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                tokio::select! {
                    frame = body.frame() => return Ok(frame),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                        if store.cancellation_requested(id) { return Err(StorageError::Cancelled); }
                    }
                }
            }
        })
        .await
        .map_err(|_| StorageError::Unavailable)??;
        let Some(frame) = frame else { break };
        let frame = frame.map_err(|_| StorageError::Invalid)?;
        if let Ok(bytes) = frame.into_data() {
            actual = actual
                .checked_add(bytes.len() as u64)
                .ok_or(StorageError::Quota)?;
            spool.write(&bytes).await?;
        }
    }
    if length.is_some_and(|v| v != actual) {
        return Err(StorageError::Invalid);
    }
    spool.sync().await?;
    Ok(spool)
}
async fn file(
    Extension(store): Extension<Store>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Ok(Path(id)) = path else {
        return failure(StorageError::Invalid);
    };
    let result = async {
        if header(&headers, "x-lytronaut-entry-type")? != "file" {
            return Err(StorageError::Invalid);
        }
        let path = logical_path(&headers, &store)?;
        let spool = stream(&store, &id, &headers, body, store.limits().file_bytes).await?;
        store
            .append_directory_file(&id, &path, spool.reader()?)
            .await
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            if e != StorageError::Busy {
                let _ = store.fail_with(&id, e).await;
            }
            failure(e)
        }
    }
}
async fn zip(
    State(state): State<AppState>,
    Extension(store): Extension<Store>,
    path: Result<Path<String>, axum::extract::rejection::PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let Ok(Path(id)) = path else {
        return failure(StorageError::Invalid);
    };
    let result = async {
        let spool = stream(&store, &id, &headers, body, store.limits().encoded_bytes).await?;
        store.import_zip(&id, spool).await?;
        crate::api::adapt(&state.catalog, &store, &id)
            .await
            .map_err(|_| StorageError::Unavailable)
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            if e != StorageError::Busy {
                let _ = store.fail_with(&id, e).await;
            }
            failure(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn body(response: Response) -> (StatusCode, lytronaut_contracts::ApiError) {
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    /// A shortage is a retryable 429 resource_limit (with its numbers for
    /// space); a declaration over the upload budget is a permanent 413.
    #[tokio::test]
    async fn space_shortage_and_budget_overrun_are_distinct_refusals() {
        let (status, error) = body(failure(StorageError::Space {
            required_bytes: 9,
            available_bytes: 4,
        }))
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.code, ErrorCode::ResourceLimit);
        assert!(error.retryable);
        assert_eq!(
            error.shortfall,
            Some(lytronaut_contracts::SpaceShortfall {
                required_bytes: 9,
                available_bytes: 4
            })
        );
        let (status, error) = body(failure(StorageError::Inodes)).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            (error.code, error.retryable, error.shortfall),
            (ErrorCode::ResourceLimit, true, None)
        );
        let (status, error) = body(failure(StorageError::Quota)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            (error.code, error.retryable),
            (ErrorCode::InvalidInput, false)
        );
    }
}
