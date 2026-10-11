//! Native HTTP boundary. Only redacted health is exposed until authentication is implemented.
#![cfg(not(target_arch = "wasm32"))]
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use lytronaut_contracts::{CapabilityReport, HealthReport, WireVersion};
use lytronaut_storage::Catalog;

#[derive(Clone)]
pub struct AppState {
    pub catalog: Catalog,
}
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .with_state(state)
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
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    #[tokio::test]
    async fn health_is_honest_and_unknown_routes_are_closed() {
        let state = AppState {
            catalog: Catalog::open_application_database(std::path::Path::new(":memory:"))
                .await
                .unwrap(),
        };
        let app = router(state);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let report: HealthReport = serde_json::from_slice(&bytes).unwrap();
        assert!(report.ready);
        assert_eq!(report.capabilities, CapabilityReport::foundation());
        for uri in ["/api/assets", "/api/imports", "/api/jobs"] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }
}
