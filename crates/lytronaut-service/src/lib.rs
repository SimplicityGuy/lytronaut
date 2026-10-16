//! Native HTTP boundary. Access control belongs to deployment.
#![cfg(not(target_arch = "wasm32"))]
pub mod api;

pub mod bundle;
pub mod database;
pub mod diagnostics;
pub mod local;
pub mod restore_journal;
pub mod uploads;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use lytronaut_contracts::{ApiError, CapabilityReport, ErrorCode, HealthReport, WireVersion};
use lytronaut_storage::Catalog;

/// One exact externally visible origin. Forwarded headers are never trusted.
#[derive(Clone)]
pub struct SecurityConfig {
    origin: String,
    authority: String,
}
impl SecurityConfig {
    pub fn new(origin: &str) -> Result<Self, ServiceError> {
        let uri: Uri = origin.parse().map_err(|_| ServiceError)?;
        let scheme = uri.scheme_str().ok_or(ServiceError)?;
        let authority = uri.authority().ok_or(ServiceError)?.as_str();
        if !matches!(scheme, "http" | "https")
            || authority.contains('@')
            || uri.path() != "/"
            || uri.query().is_some()
            || origin.ends_with('/')
        {
            return Err(ServiceError);
        }
        if scheme == "http" && !matches!(uri.host(), Some("localhost" | "127.0.0.1" | "[::1]")) {
            return Err(ServiceError);
        }
        Ok(Self {
            origin: origin.to_owned(),
            authority: authority.to_owned(),
        })
    }
}
#[derive(Clone)]
pub struct AppState {
    pub catalog: Catalog,
    pub security: SecurityConfig,
}
/// Convenience constructor with no product routes yet.
pub fn router(state: AppState) -> Router {
    router_with_routes(state, Router::new())
}
/// Add product routes BEFORE this final boundary. Never append routes to the returned router.
/// All paths inherit the configured Host/Origin boundary.
pub fn router_with_routes(state: AppState, application: Router<AppState>) -> Router {
    construct_router(state, application, None)
}
/// Trusted generated application bytes only, never imported library or uploaded bytes.
#[derive(Clone)]
pub struct ClientAssets {
    pub html: axum::body::Bytes,
    pub javascript: axum::body::Bytes,
    pub wasm: axum::body::Bytes,
    pub css: axum::body::Bytes,
}
/// Exposes only four fixed client paths through the same Host/Origin boundary.
pub fn router_with_client_assets(
    state: AppState,
    application: Router<AppState>,
    client: ClientAssets,
) -> Router {
    construct_router(state, application, Some(client))
}
fn construct_router(
    state: AppState,
    mut application: Router<AppState>,
    client: Option<ClientAssets>,
) -> Router {
    if let Some(client) = client {
        for (path, bytes, content_type) in [
            ("/", client.html, "text/html; charset=utf-8"),
            (
                "/client/lytronaut_client.js",
                client.javascript,
                "text/javascript; charset=utf-8",
            ),
            (
                "/client/lytronaut_client_bg.wasm",
                client.wasm,
                "application/wasm",
            ),
            ("/client/style.css", client.css, "text/css; charset=utf-8"),
        ] {
            application = application.route(
                path,
                get(move || {
                    let bytes = bytes.clone();
                    async move { ([(header::CONTENT_TYPE, content_type)], bytes) }
                }),
            );
        }
    }
    application
        .route("/healthz", get(health))
        .fallback(|| async { error(StatusCode::NOT_FOUND, ErrorCode::NotFound) })
        .method_not_allowed_fallback(|| async {
            error(StatusCode::METHOD_NOT_ALLOWED, ErrorCode::InvalidInput)
        })
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(state.clone(), boundary))
        .with_state(state)
}
fn error(status: StatusCode, code: ErrorCode) -> Response {
    (
        status,
        Json(ApiError {
            version: WireVersion::V1,
            code,
            retryable: status == StatusCode::SERVICE_UNAVAILABLE
                || status == StatusCode::TOO_MANY_REQUESTS,
            shortfall: None,
        }),
    )
        .into_response()
}
fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut all = headers.get_all(name).iter();
    let value = all.next()?.to_str().ok()?;
    if all.next().is_some() {
        return None;
    }
    Some(value)
}
async fn boundary(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let mut response = authorize(state, request, next).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response
}
async fn authorize(state: AppState, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let method = request.method();
    let path = request.uri().path().to_owned();
    let mutate = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    // Authority prevents DNS rebinding. Proxy must preserve the configured external Host.
    if header_str(headers, "host") != Some(state.security.authority.as_str())
        || header_str(headers, "origin").is_some_and(|origin| origin != state.security.origin)
        || headers.contains_key("origin") && header_str(headers, "origin").is_none()
        || header_str(headers, "sec-fetch-site")
            .is_some_and(|site| !matches!(site, "same-origin" | "none"))
        || mutate && header_str(headers, "origin") != Some(state.security.origin.as_str())
    {
        return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized);
    }
    // A custom header makes browser mutations non-simple and forbids form-based CSRF.
    // This is request intent, not an authentication credential; CORS is never enabled.
    if mutate && header_str(headers, "x-lytronaut-intent") != Some("mutation") {
        return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized);
    }
    if request
        .uri()
        .query()
        .is_some_and(|query| query.len() > 1024)
    {
        return error(StatusCode::BAD_REQUEST, ErrorCode::InvalidInput);
    }
    if path.contains(['%', '\\']) || path.split('/').any(|segment| matches!(segment, "." | "..")) {
        return error(StatusCode::BAD_REQUEST, ErrorCode::InvalidInput);
    }
    next.run(request).await
}
async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthReport>) {
    let ready = state.catalog.healthy().await.is_ok();
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(HealthReport {
            version: WireVersion::V1,
            ready,
            capabilities: CapabilityReport::foundation(),
        }),
    )
}

/// Diagnostics deliberately exclude filesystem and configuration details.
#[derive(Debug, Clone, Copy)]
pub struct ServiceError;
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("service unavailable")
    }
}
impl std::error::Error for ServiceError {}

mod transient_owner;
