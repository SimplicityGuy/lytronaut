//! Native HTTP boundary. All application routes and fallbacks inherit authentication.
#![cfg(not(target_arch = "wasm32"))]
pub mod auth;
pub mod local;
use auth::{AuthStore, Session};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Request, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use lytronaut_contracts::{ApiError, CapabilityReport, ErrorCode, HealthReport, WireVersion};
use lytronaut_storage::Catalog;
use serde::{Deserialize, Serialize};

/// One exact externally visible origin. Forwarded headers are never trusted.
#[derive(Clone)]
pub struct SecurityConfig {
    origin: String,
    authority: String,
    secure: bool,
    pub session_ttl_seconds: i64,
}
impl SecurityConfig {
    pub fn new(origin: &str) -> Result<Self, auth::AuthError> {
        let uri: Uri = origin.parse().map_err(|_| auth::AuthError)?;
        let scheme = uri.scheme_str().ok_or(auth::AuthError)?;
        let authority = uri.authority().ok_or(auth::AuthError)?.as_str();
        if !matches!(scheme, "http" | "https")
            || authority.contains('@')
            || uri.path() != "/"
            || uri.query().is_some()
            || origin.ends_with('/')
        {
            return Err(auth::AuthError);
        }
        if scheme == "http" && !matches!(uri.host(), Some("localhost" | "127.0.0.1" | "[::1]")) {
            return Err(auth::AuthError);
        }
        Ok(Self {
            origin: origin.to_owned(),
            authority: authority.to_owned(),
            secure: scheme == "https",
            session_ttl_seconds: 8 * 60 * 60,
        })
    }
    fn cookie(&self, token: &str, delete: bool) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{}",
            self.cookie_name(),
            token,
            if delete { 0 } else { self.session_ttl_seconds },
            if self.secure { "; Secure" } else { "" }
        )
    }
    fn cookie_name(&self) -> &'static str {
        if self.secure {
            "__Host-lytronaut_session"
        } else {
            "lytronaut_session"
        }
    }
}
#[derive(Clone)]
pub struct AppState {
    pub catalog: Catalog,
    pub auth: AuthStore,
    pub security: SecurityConfig,
}
/// Convenience constructor with no product routes yet.
pub fn router(state: AppState) -> Router {
    router_with_routes(state, Router::new())
}
/// Add product routes BEFORE this final boundary. Never append routes to the returned router.
/// Only the exact public health/setup/login methods are exempt; even unknown paths require auth.
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
/// Exposes only four fixed client paths. Every application/product path remains protected.
pub fn router_with_client_assets(
    state: AppState,
    application: Router<AppState>,
    client: ClientAssets,
) -> Router {
    construct_router(state, application, Some(client))
}
#[derive(Clone)]
struct BoundaryState {
    app: AppState,
    client: bool,
}
fn construct_router(
    state: AppState,
    mut application: Router<AppState>,
    client: Option<ClientAssets>,
) -> Router {
    let public_client = client.is_some();
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
        .route("/auth/setup", post(setup))
        .route("/auth/login", post(login))
        .route("/auth/session", get(session))
        .route("/auth/logout", post(logout))
        .fallback(|| async { error(StatusCode::NOT_FOUND, ErrorCode::NotFound) })
        .method_not_allowed_fallback(|| async {
            error(StatusCode::METHOD_NOT_ALLOWED, ErrorCode::InvalidInput)
        })
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(
            BoundaryState {
                app: state.clone(),
                client: public_client,
            },
            boundary,
        ))
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
fn cookie_token(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for entry in value.to_str().ok()?.split(';') {
            if let Some((key, value)) = entry.trim().split_once('=')
                && key == name
            {
                if found.is_some() || !auth::token_valid(value) {
                    return None;
                }
                found = Some(value.to_owned());
            }
        }
    }
    found
}
async fn boundary(State(state): State<BoundaryState>, request: Request, next: Next) -> Response {
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
async fn authorize(boundary: BoundaryState, mut request: Request, next: Next) -> Response {
    let state = boundary.app;
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
    let bootstrap =
        *method == Method::POST && matches!(path.as_str(), "/auth/setup" | "/auth/login");
    let health = *method == Method::GET && path == "/healthz";
    if bootstrap {
        let intent = if path == "/auth/setup" {
            "setup"
        } else {
            "login"
        };
        if header_str(headers, "x-lytronaut-intent") != Some(intent) {
            return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized);
        }
    } else if !health
        && !(boundary.client
            && matches!(*method, Method::GET | Method::HEAD)
            && matches!(
                path.as_str(),
                "/" | "/client/lytronaut_client.js"
                    | "/client/lytronaut_client_bg.wasm"
                    | "/client/style.css"
            ))
    {
        let Some(token) = cookie_token(headers, state.security.cookie_name()) else {
            return error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized);
        };
        let session = match state.auth.session(&token).await {
            Ok(Some(session)) => session,
            Ok(None) => return error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized),
            Err(_) => {
                return error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    ErrorCode::StorageUnavailable,
                );
            }
        };
        if mutate
            && !header_str(headers, "x-lytronaut-csrf")
                .is_some_and(|value| auth::equal(value, &session.csrf))
        {
            return error(StatusCode::FORBIDDEN, ErrorCode::Unauthorized);
        }
        request.extensions_mut().insert(session);
        request.extensions_mut().insert(Authenticated);
    }
    if path.contains(['%', '\\']) || path.split('/').any(|segment| matches!(segment, "." | "..")) {
        return error(StatusCode::BAD_REQUEST, ErrorCode::InvalidInput);
    }
    next.run(request).await
}
/// Marker provided only after checking a durable unexpired session. Product handlers may extract it.
#[derive(Clone, Copy)]
pub struct Authenticated;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials {
    username: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setup {
    bootstrap: String,
    username: String,
    password: String,
}
#[derive(Serialize)]
struct SessionReport {
    version: WireVersion,
    csrf: String,
}
async fn admit(state: &AppState, kind: &str) -> Option<Response> {
    match state.auth.admit(kind).await {
        Ok(true) => None,
        Ok(false) => {
            let mut response = error(StatusCode::TOO_MANY_REQUESTS, ErrorCode::Unauthorized);
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
            Some(response)
        }
        Err(_) => Some(error(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        )),
    }
}
async fn setup(
    State(state): State<AppState>,
    payload: Result<Json<Setup>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Some(response) = admit(&state, "setup").await {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized);
    };
    match state
        .auth
        .setup(&payload.bootstrap, &payload.username, &payload.password)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        ),
    }
}
async fn login(
    State(state): State<AppState>,
    payload: Result<Json<Credentials>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if let Some(response) = admit(&state, "login").await {
        return response;
    }
    let Ok(Json(payload)) = payload else {
        return error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized);
    };
    if payload.username.len() > 64 || payload.password.len() > 1024 {
        return error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized);
    }
    if !(1..=24 * 60 * 60).contains(&state.security.session_ttl_seconds) {
        return error(StatusCode::SERVICE_UNAVAILABLE, ErrorCode::Internal);
    }
    match state
        .auth
        .login(
            &payload.username,
            &payload.password,
            state.security.session_ttl_seconds,
        )
        .await
    {
        Ok(Some(token)) => {
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                state.security.cookie(&token, false).parse().unwrap(),
            );
            response
        }
        Ok(None) => error(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized),
        Err(_) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        ),
    }
}
async fn session(Extension(session): Extension<Session>) -> Json<SessionReport> {
    Json(SessionReport {
        version: WireVersion::V1,
        csrf: session.csrf,
    })
}
async fn logout(State(state): State<AppState>, Extension(session): Extension<Session>) -> Response {
    if state.auth.logout(&session).await.is_err() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::StorageUnavailable,
        );
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        state.security.cookie("", true).parse().unwrap(),
    );
    response
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
