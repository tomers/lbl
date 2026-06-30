//! HTTP API for programmatic access to the `lbl` pipeline.
//!
//! Exposes JSON endpoints for the effective configuration, the media catalog,
//! printer discovery and profile management, label preview (transpilation), and
//! printing (the full pipeline).

mod handlers;
mod state;

pub use state::AppState;

use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;

/// Build the application router with all routes mounted under `/api`.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/config", get(handlers::get_config))
        .route("/api/config/sources", get(handlers::get_config_sources))
        .route("/api/catalog", get(handlers::list_catalog))
        .route("/api/catalog/printers", get(handlers::list_catalog_printers))
        .route("/api/catalog/printers/:key", get(handlers::show_catalog_printer))
        .route("/api/catalog/compatible", get(handlers::compatible_catalog))
        .route("/api/catalog/:key", get(handlers::show_catalog))
        .route("/api/printers", get(handlers::list_printers))
        .route("/api/printers/profiles", get(handlers::list_profiles))
        .route("/api/printers/profiles", put(handlers::upsert_profile))
        .route(
            "/api/printers/profiles/:id",
            delete(handlers::delete_profile),
        )
        .route("/api/preview", post(handlers::preview))
        .route("/api/print", post(handlers::print))
        .route("/api/print/file", post(handlers::print_file))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use lbl_catalog::Catalog;
    use lbl_config::{Loader, ProfileStore};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState {
            catalog: Arc::new(Catalog::bundled().unwrap()),
            profiles: Arc::new(ProfileStore::new(
                std::env::temp_dir().join("lbl-server-test-printers.toml"),
            )),
            loader: Arc::new(Loader::new()),
        }
    }

    #[tokio::test]
    async fn health_ok() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn catalog_lists_entries() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/catalog")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn preview_returns_labels() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"hi {{qr:x}}"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 1);
        assert!(json["labels"][0]["html"]
            .as_str()
            .unwrap()
            .contains("lbl-qr"));
    }
}
