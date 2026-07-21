//! HTTP API for programmatic access to the `lbl` pipeline.
//!
//! Exposes JSON endpoints for the effective configuration, the media catalog,
//! printer discovery and profile management, label preview (server raster), and
//! printing (the full pipeline).

mod font_cache;
mod handlers;
mod render_pool;
mod state;

pub use render_pool::RenderPool;
pub use state::AppState;

use axum::extract::Request;
use axum::middleware::{from_fn, Next};
use axum::response::Response;
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tracing::Instrument;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize tracing the same way as openapp's API server:
/// `registry` + `RUST_LOG` EnvFilter (default `info`) + pretty `fmt` with optional ANSI.
pub fn init_tracing() {
    let enable_ansi = should_enable_ansi_colors();
    tracing_subscriber::registry()
        .with(EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_ansi(enable_ansi))
        .init();
}

fn should_enable_ansi_colors() -> bool {
    !env_bool("DISABLE_ANSI_COLORS", false)
}

fn env_bool(var_name: &str, default: bool) -> bool {
    match std::env::var(var_name) {
        Ok(value) => match value.to_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => true,
            "false" | "0" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

/// Parent span for each HTTP request (openapp: `http_request` + optional identity fields).
async fn http_request_span(request: Request, next: Next) -> Response {
    let span = tracing::info_span!(
        "http_request",
        method = %request.method(),
        path = %request.uri().path(),
    );
    next.run(request).instrument(span).await
}

/// Host-attached profile CRUD + status/media. Mounted only when
/// `LBL_HOST_DISCOVERY` is enabled (server print mode). Browser print mode
/// keeps profiles in the client and must not share a host-global store.
fn host_profile_routes() -> Router<AppState> {
    Router::new()
        .route("/api/devices/profiles", get(handlers::list_profiles))
        .route("/api/devices/profiles", put(handlers::upsert_profile))
        .route(
            "/api/devices/profiles/{id}",
            delete(handlers::delete_profile),
        )
        .route(
            "/api/devices/profiles/{id}/media",
            get(handlers::profile_detected_media),
        )
        .route(
            "/api/devices/profiles/{id}/status",
            get(handlers::profile_printer_status),
        )
        .route(
            "/api/devices/profiles/{id}/soft-reboot",
            post(handlers::profile_soft_reboot),
        )
}

/// Build the application router with all routes mounted under `/api`.
pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/api/health", get(handlers::health))
        .route("/api/config", get(handlers::get_config))
        .route("/api/config/sources", get(handlers::get_config_sources))
        .route("/api/catalog", get(handlers::list_catalog))
        .route("/api/fonts", get(handlers::list_fonts))
        .route("/api/catalog/devices", get(handlers::list_catalog_devices))
        .route(
            "/api/catalog/devices/{key}",
            get(handlers::show_catalog_printer),
        )
        .route("/api/catalog/compatible", get(handlers::compatible_catalog))
        .route("/api/catalog/{key}", get(handlers::show_catalog))
        .route("/api/devices", get(handlers::list_devices))
        .route("/api/preview", post(handlers::preview))
        .route("/api/preview/html", post(handlers::preview_html))
        .route("/api/print", post(handlers::print))
        .route("/api/print/file", post(handlers::print_file));

    if state.host_discovery_enabled {
        app = app.merge(host_profile_routes());
    }

    app.layer(from_fn(http_request_span))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::font_cache::FontFileCache;
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use lbl_catalog::Catalog;
    use lbl_config::{Loader, ProfileStore};
    use std::env;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState {
            catalog: Arc::new(Catalog::bundled().unwrap()),
            profiles: Arc::new(ProfileStore::new(
                env::temp_dir().join("lbl-server-test-printers.toml"),
            )),
            loader: Arc::new(Loader::new()),
            host_discovery_enabled: true,
            renderer: Arc::new(RenderPool::new(1)),
            font_assets_base_url: lbl_text::default_font_assets_base_url().to_string(),
            font_cache: Arc::new(FontFileCache::new(
                env::temp_dir().join("lbl-server-test-font-cache"),
            )),
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
    async fn fonts_lists_catalog_with_heebo() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/fonts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["version"], "v1");
        assert!(json["assetsBaseUrl"]
            .as_str()
            .unwrap()
            .starts_with("https://"));
        let fonts = json["fonts"].as_array().unwrap();
        assert!(fonts.len() > 10);
        let heebo = fonts.iter().find(|f| f["slug"] == "heebo").unwrap();
        assert_eq!(heebo["system"], false);
        assert!(heebo["scripts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "hebrew"));
        assert!(heebo["previewUrl"]
            .as_str()
            .unwrap()
            .contains("/previews/heebo.png"));
        assert!(!serde_json::to_string(&json)
            .unwrap()
            .contains("fonts.googleapis.com"));
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
    async fn preview_html_print_geometry_returns_html() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview/html")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"text":"hi","geometry":"print","dpi":300,"supersample":2,"media":"30252"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["media"]["geometry"], "print");
        assert!(json["labels"][0]["html"].as_str().unwrap().contains("lbl-"));
        let width = json["labels"][0]["width_px"]
            .as_f64()
            .or_else(|| json["labels"][0]["width_px"].as_u64().map(|n| n as f64))
            .unwrap_or(0.0);
        assert!(
            width > 0.0,
            "expected positive width_px, got {}",
            json["labels"][0]
        );
        assert!(
            !json["labels"][0]["html"]
                .as_str()
                .unwrap()
                .contains("lbl-stock"),
            "print geometry must stay printable-only (no stock frame)"
        );
    }

    #[tokio::test]
    async fn preview_html_template_syntax_error_is_bad_request() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview/html")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        // `{{qr:…}}` is not valid template syntax: request-supplied
                        // template errors must be client errors, not 500s.
                        r#"{"template":"{{qr:https://example.com}}","template_format":"text","data":[{"n":1}],"width_mm":50,"dpi":203}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let error = json["error"].as_str().unwrap();
        assert!(
            error.contains("template render error"),
            "error should surface the engine message, got: {error}"
        );
    }

    #[tokio::test]
    async fn preview_html_print_geometry_pins_continuous_d1_feed_width() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview/html")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"text":"Hello, world","geometry":"print","dpi":180,"supersample":1,"media":"45013","printer":"LabelManager 280"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["media"]["geometry"], "print");
        assert_eq!(json["media"]["continuous"], true);
        let html = json["labels"][0]["html"].as_str().unwrap();
        assert!(
            html.contains("--lbl-feed-px:"),
            "expected content head-fit feed pin, got {html}"
        );
        let width = json["labels"][0]["width_px"]
            .as_f64()
            .or_else(|| json["labels"][0]["width_px"].as_u64().map(|n| n as f64))
            .unwrap_or(0.0);
        assert!(
            width > 10.0 && width < 800.0,
            "continuous D1 print geometry must pin a tight feed width (not 0 / not iframe placeholder), got {width}"
        );
        assert!(
            !html.contains("lbl-stock"),
            "print geometry must stay printable-only (no stock frame)"
        );
    }

    #[tokio::test]
    async fn preview_html_vector_pads_physical_stock() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview/html")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"text":"hi","geometry":"vector","media":"15x30","printer":"D110"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["media"]["geometry"], "vector");
        let label = &json["labels"][0];
        assert!(
            label["html"].as_str().unwrap().contains("lbl-stock"),
            "vector preview should frame printable content in physical stock"
        );
        let pad_before = label["head_pad_before_px"].as_u64().unwrap_or(0);
        let pad_after = label["head_pad_after_px"].as_u64().unwrap_or(0);
        assert!(
            pad_before > 0 && pad_after > 0,
            "D110 on 15 mm stock should pad unprintable head gaps, got before={pad_before} after={pad_after}"
        );
        assert!(
            label.get("content_bounds_px").is_none(),
            "vector HTML preview leaves ink bounds to the browser"
        );
    }

    #[tokio::test]
    async fn preview_html_continuous_dymo_keeps_feed_axis_open() {
        let app = router(test_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/preview/html")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"text":"Hello, World!","geometry":"vector","media":"45013","printer":"LabelManager PnP","orientation":"landscape"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["media"]["continuous"], true);
        let width = json["media"]["width_px"].as_f64().unwrap_or(-1.0);
        assert!(
            width > 0.0,
            "continuous D1 should report content-estimated width_px, got {width}"
        );
        let height = json["media"]["height_px"].as_f64().unwrap_or(0.0);
        assert!(height > 0.0, "tape head axis should be sized, got {height}");
        let html = json["labels"][0]["html"].as_str().unwrap();
        assert!(
            html.contains("lbl-stock"),
            "LabelManager should still pad laminate head gaps"
        );
        assert!(
            !html.contains("width:0.00px"),
            "must not clip continuous content in a 0-wide stock print box"
        );
        assert!(
            html.contains("--lbl-feed-px:") && html.contains("white-space:nowrap"),
            "continuous head-fit text should pin feed estimate and nowrap"
        );
        let label = &json["labels"][0];
        let lead = label["feed_lead_px"].as_u64().unwrap_or(0);
        let end_margin = label["feed_end_margin_px"].as_u64().unwrap_or(0);
        assert!(
            lead > 0 && end_margin > 0,
            "LabelManager feed_trail_mm should show symmetric head-to-cutter gaps, got lead={lead} end={end_margin}"
        );
        assert!(
            label["content_feed_end_px"].as_u64().unwrap_or(0) > lead,
            "content end marker should sit after lead + estimated payload"
        );
        assert!(
            html.contains(&format!(
                "padding:{}px {}px {}px {}px",
                label["head_pad_before_px"].as_u64().unwrap_or(0),
                end_margin,
                label["head_pad_after_px"].as_u64().unwrap_or(0),
                lead
            )),
            "expected feed+head padding lead={lead} end={end_margin} in stock CSS, got: {html}"
        );
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
            if err.contains("Chromium")
                || err.contains("chrome")
                || err.to_ascii_lowercase().contains("rendering")
            {
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
    async fn list_devices_empty_when_host_discovery_disabled() {
        let state = AppState {
            host_discovery_enabled: false,
            ..test_state()
        };
        let app = router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/devices")
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
                    .uri("/api/devices/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(profile.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn host_profile_routes_absent_when_discovery_disabled() {
        let state = AppState {
            host_discovery_enabled: false,
            ..test_state()
        };
        let app = router(state);
        for (method, uri) in [
            ("GET", "/api/devices/profiles"),
            ("PUT", "/api/devices/profiles"),
            ("DELETE", "/api/devices/profiles/x"),
            ("GET", "/api/devices/profiles/x/media"),
            ("GET", "/api/devices/profiles/x/status"),
            ("POST", "/api/devices/profiles/x/soft-reboot"),
        ] {
            let mut builder = Request::builder().method(method).uri(uri);
            let body = if method == "PUT" {
                builder = builder.header("content-type", "application/json");
                Body::from("{}")
            } else {
                Body::empty()
            };
            let resp = app
                .clone()
                .oneshot(builder.body(body).unwrap())
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "{method} {uri} should be unmounted when host discovery is off"
            );
        }
    }
}
