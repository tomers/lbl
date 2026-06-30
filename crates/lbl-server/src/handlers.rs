//! Request handlers.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use lbl::pipeline::{
    authoring_labels, encode_label, resolve_media, resolve_style, PipelineOptions, Source,
};
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

pub async fn list_catalog_printers(State(state): State<AppState>) -> ApiResult {
    Ok(Json(state.catalog.printers()).into_response())
}

pub async fn show_catalog_printer(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult {
    match state
        .catalog
        .lookup_printer(&key)
        .or_else(|| state.catalog.match_printer(&key))
    {
        Some(p) => Ok(Json(p).into_response()),
        None => Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("no printer entry '{key}'"),
        )),
    }
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
    state.profiles.remove(&lbl_core::printer::PrinterId(id))?;
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
            return Ok(Source::Text {
                text,
                raw: self.raw,
            });
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

pub async fn preview(State(state): State<AppState>, Json(req): Json<SourceReq>) -> ApiResult {
    let labels = authoring_labels(req.into_source()?).map_err(ApiError::from)?;
    let count = labels.len();
    // Resolve the configured physical sizes against the standard preview DPI and
    // supersample factor so the browser preview matches printed sizing.
    let style_cfg = state.loader.load().map(|c| c.style).unwrap_or_default();
    let style = resolve_style(&style_cfg, default_dpi(), 2);
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
                    style: style.clone(),
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
    serial: Option<String>,
    #[serde(default)]
    use_sidecar: bool,
    /// For the virtual printer: output image format (png|bmp|tiff|gif|pbm).
    #[serde(default)]
    media_type: Option<String>,
    /// Layout orientation (`portrait`|`landscape`). Falls back to the
    /// configured default (landscape) when omitted.
    #[serde(default)]
    orientation: Option<lbl_core::Orientation>,
    /// Extra clockwise quarter-turns, composed on top of the orientation.
    #[serde(default)]
    rotate_cw: u32,
    /// Extra counter-clockwise quarter-turns, composed on top of the orientation.
    #[serde(default)]
    rotate_ccw: u32,
    /// Also build the HTML pipeline debug report.
    #[serde(default)]
    debug: bool,
}

impl PrintReq {
    /// Resolve the net [`lbl_core::Rotation`] for this request: the explicit
    /// orientation (or the configured default) plus any extra quarter-turns.
    fn rotation(&self, state: &AppState) -> lbl_core::Rotation {
        let orientation = self.orientation.unwrap_or_else(|| {
            state
                .loader
                .load()
                .map(|c| c.render.orientation)
                .unwrap_or_default()
        });
        lbl_core::Rotation::for_print(orientation, self.rotate_cw, self.rotate_ccw)
    }
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
        "dymolw" | "dymo-lw" | "lw550" => Protocol::DymoLw,
        "escpos" | "esc/pos" => Protocol::EscPos,
        "zpl" => Protocol::Zpl,
        "tspl" => Protocol::Tspl,
        "niimbot" | "d110" | "d11" => Protocol::Niimbot,
        "virtual" | "file" => Protocol::Virtual,
        "console" | "term" => Protocol::Console,
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

    let style_cfg = state.loader.load().map(|c| c.style).unwrap_or_default();
    let style = resolve_style(&style_cfg, req.dpi, req.supersample);

    let protocol = parse_protocol(&req.protocol)?;
    let rotation = req.rotation(&state);
    let opts = PipelineOptions {
        protocol,
        media,
        supports_cut: req.supports_cut,
        cut: req.cut,
        copies: req.copies,
        dither: Algorithm::parse(&req.dither)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?,
        rotation,
        supersample: req.supersample,
        assets_base: AssetsBase::Cdn,
        style,
        media_type: None,
    };

    let labels = authoring_labels(source).map_err(ApiError::from)?;
    let use_sidecar = req.use_sidecar;
    let network = req.network.clone();
    let usb = req.usb.clone();
    let serial = req.serial.clone();

    // The pipeline (browser render) is blocking; run it off the async runtime.
    let report = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let registry = Registry::with_builtin_drivers();
        let encoded: Vec<(String, Vec<u8>)> = if use_sidecar {
            encode_all(&SidecarBackend::node_default(), &registry, &labels, &opts)?
        } else {
            let backend = ChromiumBackend::launch()?;
            encode_all(&backend, &registry, &labels, &opts)?
        };
        dispatch(encoded, protocol, network, usb, serial)
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

// ---------------------------------------------------------------------------
// print to file (virtual printer) / debug report
// ---------------------------------------------------------------------------

/// "Print" to image files (the virtual printer) and/or build the HTML pipeline
/// debug report, returning the artifacts inline so the browser can download
/// them. No physical device is contacted.
pub async fn print_file(State(state): State<AppState>, Json(req): Json<PrintReq>) -> ApiResult {
    use base64::Engine as _;

    let source = req.source.clone().into_source()?;
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm,
        req.length_mm,
        req.dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;

    let protocol = parse_protocol(&req.protocol)?;
    let media_type = if protocol == Protocol::Virtual {
        Some(match &req.media_type {
            Some(name) => lbl_driver_file::MediaType::parse(name)
                .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?,
            None => lbl_driver_file::MediaType::Png,
        })
    } else {
        None
    };

    let style_cfg = state.loader.load().map(|c| c.style).unwrap_or_default();
    let style = resolve_style(&style_cfg, req.dpi, req.supersample);

    let rotation = req.rotation(&state);
    let opts = PipelineOptions {
        protocol,
        media,
        supports_cut: req.supports_cut,
        cut: req.cut,
        copies: req.copies,
        dither: Algorithm::parse(&req.dither)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?,
        rotation,
        supersample: req.supersample,
        assets_base: AssetsBase::Cdn,
        style,
        media_type,
    };

    let labels = authoring_labels(source).map_err(ApiError::from)?;
    let use_sidecar = req.use_sidecar;
    let want_debug = req.debug;
    let (extension, mime) = match media_type {
        Some(mt) => (mt.extension().to_string(), mt.mime().to_string()),
        None => ("bin".to_string(), "application/octet-stream".to_string()),
    };

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let mut registry = Registry::with_builtin_drivers();
        if let Some(mt) = media_type {
            registry.register(Box::new(lbl_driver_file::FileDriver::new(mt)));
        }

        let encode = |backend: &dyn DynBackend| -> anyhow::Result<serde_json::Value> {
            let mut out_labels = Vec::new();
            let mut traces = Vec::new();
            for label in &labels {
                let trace = backend.encode_traced(&registry, label.index, &label.html, &opts)?;
                let data_url = format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(&trace.encoded)
                );
                out_labels.push(json!({
                    "index": label.index,
                    "filename": format!("label-{:04}.{extension}", label.index),
                    "mime": mime,
                    "size": trace.encoded.len(),
                    "data_url": data_url,
                }));
                if want_debug {
                    traces.push(trace);
                }
            }
            let debug_html = if want_debug {
                Some(lbl::debug::render_report(&traces))
            } else {
                None
            };
            Ok(json!({
                "count": out_labels.len(),
                "protocol": opts_protocol_name(opts.protocol),
                "media_type": media_type.map(|mt| mt.name()),
                "labels": out_labels,
                "debug_html": debug_html,
            }))
        };

        if use_sidecar {
            encode(&SidecarBackend::node_default())
        } else {
            encode(&ChromiumBackend::launch()?)
        }
    })
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(result).into_response())
}

/// The CLI/display name of a protocol (matches the `--protocol` value).
fn opts_protocol_name(protocol: Protocol) -> &'static str {
    lbl::debug::protocol_cli_name(protocol)
}

/// A render backend object-safe enough to trace one label through the pipeline.
trait DynBackend {
    fn encode_traced(
        &self,
        registry: &Registry,
        index: usize,
        html: &str,
        opts: &PipelineOptions,
    ) -> anyhow::Result<lbl::debug::LabelTrace>;
}

impl<B: RenderBackend> DynBackend for B {
    fn encode_traced(
        &self,
        registry: &Registry,
        index: usize,
        html: &str,
        opts: &PipelineOptions,
    ) -> anyhow::Result<lbl::debug::LabelTrace> {
        lbl::pipeline::encode_label_traced(self, registry, index, html, opts)
    }
}

fn dispatch(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    network: Option<String>,
    usb: Option<String>,
    serial: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    use lbl::dispatch::{dispatch_encoded, parse_serial_target};
    let total = encoded.len();

    let report = if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow::anyhow!("network target must be host:port"))?;
        let mut t = lbl_device::NetworkTransport::new(host, port.parse()?);
        dispatch_encoded(encoded, protocol, &mut t)
    } else if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("usb target must be vid:pid"))?;
        let mut t = lbl_device::UsbTransport::new(
            u16::from_str_radix(vid, 16)?,
            u16::from_str_radix(pid, 16)?,
            None,
        );
        dispatch_encoded(encoded, protocol, &mut t)
    } else if let Some(target) = serial {
        let (path, baud) = parse_serial_target(&target);
        let mut t = lbl_device::SerialTransport::new(path, baud);
        dispatch_encoded(encoded, protocol, &mut t)
    } else {
        anyhow::bail!("no target; provide network, usb, or serial");
    };

    Ok(json!({
        "total": total,
        "completed": report.completed,
        "remaining": report.remaining,
        "disconnected": report.disconnected,
    }))
}
