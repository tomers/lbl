//! Request handlers.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use lbl::pipeline::{authoring_labels, encode_label, resolve_media, PipelineOptions, Source};
use lbl_core::printer::{PrinterProfile, Protocol};
use lbl_dither::Algorithm;
use lbl_encode::Registry;
use lbl_render::{ChromiumBackend, RenderBackend, SidecarBackend};
use lbl_transpile_html::{transpile, AssetsBase, TranspileOptions};

use crate::AppState;

/// A handler error that renders as a JSON `{ "error": ... }` with a status.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

type ApiResult = Result<axum::response::Response, ApiError>;

pub async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "name": "lbl-server" }))
}

pub async fn get_config(State(state): State<AppState>) -> ApiResult {
    let cfg = state.loader.load()?;
    Ok(Json(cfg).into_response())
}

pub async fn get_config_sources(State(state): State<AppState>) -> ApiResult {
    let sources = lbl_config::describe_sources(state.loader.figment());
    let map: serde_json::Map<String, serde_json::Value> = sources
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::String(v)))
        .collect();
    Ok(Json(map).into_response())
}

pub async fn list_catalog(State(state): State<AppState>) -> ApiResult {
    Ok(Json(state.catalog.entries()).into_response())
}

pub async fn show_catalog(State(state): State<AppState>, Path(key): Path<String>) -> ApiResult {
    match state.catalog.lookup(&key) {
        Some(e) => Ok(Json(e).into_response()),
        None => Err(ApiError(StatusCode::NOT_FOUND, format!("no entry '{key}'"))),
    }
}

#[derive(Deserialize)]
pub struct CompatibleQuery {
    printer: String,
}

pub async fn compatible_catalog(
    State(state): State<AppState>,
    Query(q): Query<CompatibleQuery>,
) -> ApiResult {
    let entries = state.catalog.compatible_with(&q.printer);
    Ok(Json(entries).into_response())
}

pub async fn list_printers(State(_state): State<AppState>) -> ApiResult {
    let discovered = lbl_device::discover_usb();
    Ok(Json(discovered).into_response())
}

pub async fn list_profiles(State(state): State<AppState>) -> ApiResult {
    let profiles = state.profiles.load()?;
    Ok(Json(profiles).into_response())
}

pub async fn upsert_profile(
    State(state): State<AppState>,
    Json(profile): Json<PrinterProfile>,
) -> ApiResult {
    state.profiles.upsert(profile)?;
    Ok((StatusCode::OK, Json(json!({ "ok": true }))).into_response())
}

pub async fn delete_profile(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult {
    state
        .profiles
        .remove(&lbl_core::printer::PrinterId(id))?;
    Ok((StatusCode::OK, Json(json!({ "ok": true }))).into_response())
}

// ---------------------------------------------------------------------------
// preview / print
// ---------------------------------------------------------------------------

#[derive(Deserialize, Clone)]
pub struct SourceReq {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    raw: bool,
    #[serde(default)]
    html: Option<String>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    data: Option<serde_json::Value>,
    #[serde(default)]
    each: Option<String>,
}

impl SourceReq {
    fn into_source(self) -> Result<Source, ApiError> {
        if let Some(text) = self.text {
            return Ok(Source::Text { text, raw: self.raw });
        }
        if let Some(html) = self.html {
            return Ok(Source::Html(html));
        }
        if let Some(template) = self.template {
            return Ok(Source::Template {
                template,
                data: self.data,
                each: self.each,
            });
        }
        Err(ApiError(
            StatusCode::BAD_REQUEST,
            "provide one of: text, html, template".into(),
        ))
    }
}

pub async fn preview(State(_state): State<AppState>, Json(req): Json<SourceReq>) -> ApiResult {
    let labels = authoring_labels(req.into_source()?).map_err(ApiError::from)?;
    let count = labels.len();
    let out: Vec<_> = labels
        .into_iter()
        .map(|l| {
            let html = transpile(
                &l.html,
                &TranspileOptions {
                    mode: lbl_core::job::OutputMode::Preview,
                    assets_base: AssetsBase::Cdn,
                    index: Some(l.index),
                    count: Some(count),
                },
            );
            json!({ "index": l.index, "html": html })
        })
        .collect();
    Ok(Json(json!({ "count": count, "labels": out })).into_response())
}

#[derive(Deserialize)]
pub struct PrintReq {
    #[serde(flatten)]
    source: SourceReq,
    media: Option<String>,
    width_mm: Option<f64>,
    length_mm: Option<f64>,
    #[serde(default = "default_dpi")]
    dpi: f64,
    protocol: String,
    #[serde(default)]
    cut: bool,
    #[serde(default)]
    supports_cut: bool,
    #[serde(default = "default_copies")]
    copies: u32,
    #[serde(default = "default_supersample")]
    supersample: u32,
    #[serde(default = "default_dither")]
    dither: String,
    network: Option<String>,
    usb: Option<String>,
    #[serde(default)]
    use_sidecar: bool,
}

fn default_dpi() -> f64 {
    300.0
}
fn default_copies() -> u32 {
    1
}
fn default_supersample() -> u32 {
    3
}
fn default_dither() -> String {
    "auto".to_string()
}

fn parse_protocol(s: &str) -> Result<Protocol, ApiError> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "dymo" => Protocol::Dymo,
        "escpos" | "esc/pos" => Protocol::EscPos,
        "zpl" => Protocol::Zpl,
        "tspl" => Protocol::Tspl,
        other => {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                format!("unknown protocol '{other}'"),
            ))
        }
    })
}

pub async fn print(State(state): State<AppState>, Json(req): Json<PrintReq>) -> ApiResult {
    let source = req.source.clone().into_source()?;
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm,
        req.length_mm,
        req.dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;

    let opts = PipelineOptions {
        protocol: parse_protocol(&req.protocol)?,
        media,
        supports_cut: req.supports_cut,
        cut: req.cut,
        copies: req.copies,
        dither: Algorithm::parse(&req.dither).map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?,
        supersample: req.supersample,
        assets_base: AssetsBase::Cdn,
    };

    let labels = authoring_labels(source).map_err(ApiError::from)?;
    let use_sidecar = req.use_sidecar;
    let network = req.network.clone();
    let usb = req.usb.clone();

    // The pipeline (browser render) is blocking; run it off the async runtime.
    let report = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let registry = Registry::with_builtin_drivers();
        let encoded: Vec<(String, Vec<u8>)> = if use_sidecar {
            encode_all(&SidecarBackend::node_default(), &registry, &labels, &opts)?
        } else {
            let backend = ChromiumBackend::launch()?;
            encode_all(&backend, &registry, &labels, &opts)?
        };
        dispatch(encoded, network, usb)
    })
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(report).into_response())
}

fn encode_all<B: RenderBackend>(
    backend: &B,
    registry: &Registry,
    labels: &[lbl::pipeline::AuthoringLabel],
    opts: &PipelineOptions,
) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for label in labels {
        let bytes = encode_label(backend, registry, &label.html, opts)?;
        out.push((format!("label-{:04}.bin", label.index), bytes));
    }
    Ok(out)
}

fn dispatch(
    encoded: Vec<(String, Vec<u8>)>,
    network: Option<String>,
    usb: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    use lbl_spool::Spooler;
    let total = encoded.len();
    let mut spool = Spooler::new();
    for (name, bytes) in encoded {
        spool.enqueue(name, bytes, None);
    }

    let report = if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("network target must be host:port"))?;
        let mut t = lbl_device::NetworkTransport::new(host, port.parse()?);
        spool.run(&mut t)
    } else if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid"))?;
        let mut t = lbl_device::UsbTransport::new(
            u16::from_str_radix(vid, 16)?,
            u16::from_str_radix(pid, 16)?,
            None,
        );
        spool.run(&mut t)
    } else {
        anyhow::bail!("no target; provide network or usb");
    };

    Ok(json!({
        "total": total,
        "completed": report.completed,
        "remaining": report.remaining,
        "disconnected": report.disconnected,
    }))
}
