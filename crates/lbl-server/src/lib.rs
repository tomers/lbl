//! HTTP API for programmatic access to the `lbl` pipeline.
//!
//! Exposes JSON endpoints for the effective configuration, the media catalog,
//! printer discovery and profile management, label preview (server raster), and
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
        .route(
            "/api/catalog/printers",
            get(handlers::list_catalog_printers),
        )
        .route(
            "/api/catalog/printers/{key}",
            get(handlers::show_catalog_printer),
        )
        .route("/api/catalog/compatible", get(handlers::compatible_catalog))
        .route("/api/catalog/{key}", get(handlers::show_catalog))
        .route("/api/printers", get(handlers::list_printers))
        .route("/api/printers/profiles", get(handlers::list_profiles))
        .route("/api/printers/profiles", put(handlers::upsert_profile))
        .route(
            "/api/printers/profiles/{id}",
            delete(handlers::delete_profile),
        )
        .route(
            "/api/printers/profiles/{id}/media",
            get(handlers::profile_detected_media),
        )
        .route(
            "/api/printers/profiles/{id}/status",
            get(handlers::profile_printer_status),
        )
        .route("/api/preview", post(handlers::preview))
        .route("/api/preview/html", post(handlers::preview_html))
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
            host_discovery_enabled: true,
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
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let err = json["error"].as_str().unwrap_or("");
            if err.contains("Chromium") || err.contains("chrome") {
                eprintln!("skipping preview_returns_labels: {err}");
                return;
            }
        }
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 1);
        assert!(json["labels"][0]["image_base64"].as_str().unwrap().len() > 100);
    }

    #[tokio::test]
    async fn preview_uses_landscape_viewport_by_default() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"hi","media":"30252","dpi":300}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let err = json["error"].as_str().unwrap_or("");
            if err.contains("Chromium") || err.contains("chrome") {
                eprintln!("skipping preview_uses_landscape_viewport_by_default: {err}");
                return;
            }
        }
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let width = json["media"]["width_px"].as_f64().unwrap();
        let height = json["media"]["height_px"].as_f64().unwrap();
        assert!(
            width > height,
            "expected landscape preview raster, got {width}×{height}"
        );
    }

    #[tokio::test]
    async fn preview_portrait_orientation_swaps_viewport() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"text":"hi","media":"30252","dpi":300,"orientation":"portrait"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let err = json["error"].as_str().unwrap_or("");
            if err.contains("Chromium") || err.contains("chrome") {
                eprintln!("skipping preview_portrait_orientation_swaps_viewport: {err}");
                return;
            }
        }
        assert_eq!(status, StatusCode::OK);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let width = json["media"]["width_px"].as_f64().unwrap();
        let height = json["media"]["height_px"].as_f64().unwrap();
        assert!(
            width < height,
            "expected portrait preview raster, got {width}×{height}"
        );
    }

    #[tokio::test]
    async fn list_printers_empty_when_host_discovery_disabled() {
        let state = AppState {
            host_discovery_enabled: false,
            ..test_state()
        };
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/printers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert!(json.is_empty());
    }

    #[tokio::test]
    async fn upsert_browser_profile() {
        let state = test_state();
        let app = router(state);
        let profile = serde_json::json!({
            "id": "browser-test",
            "name": "My DYMO",
            "model": {
                "brand": "DYMO",
                "model": "LabelWriter 550",
                "protocol": "dymolw",
                "capabilities": {
                    "dpi": 300.0,
                    "max_width_mm": 57.0,
                    "supports_cut": false,
                    "reports_media": true
                }
            },
            "transport": {
                "type": "browser",
                "connection": "usb",
                "api": "webusb",
                "vendor_id": 2338,
                "product_id": 40
            },
            "default": false
        });
        let resp = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/printers/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(profile.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
