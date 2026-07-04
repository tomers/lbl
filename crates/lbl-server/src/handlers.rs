//! Request handlers.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use lbl::pipeline::{
    authoring_labels, encode_label, render_viewport_px, resolve_label_align, resolve_label_fit,
    resolve_label_fit_scale, resolve_label_valign, resolve_media, resolve_media_inset,
    resolve_style, resolve_style_vector, PipelineOptions, Source, TemplateFormat, VECTOR_CSS_DPI,
};
use lbl_catalog::{Catalog, ConnectionHint, PrinterEntry};
use lbl_core::printer::{PrinterProfile, Protocol};
use lbl_core::Rotation;
use lbl_dither::Algorithm;
use lbl_encode::Registry;
use lbl_render::{ChromiumBackend, RenderBackend, SidecarBackend};
use lbl_transpile_html::{transpile, AssetsBase, LabelFitSetting, TranspileOptions};

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
    match state.catalog.require_printer(&key) {
        Ok(p) => Ok(Json(p).into_response()),
        Err(message) => Err(ApiError(StatusCode::BAD_REQUEST, message)),
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

pub async fn list_printers(State(state): State<AppState>) -> ApiResult {
    if !state.host_discovery_enabled {
        return Ok(Json(Vec::<lbl_device::DiscoveredPrinter>::new()).into_response());
    }
    let discovered = lbl_device::discover();
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

pub async fn profile_detected_media(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let profiles = state.profiles.load()?;
    let profile = profiles
        .iter()
        .find(|p| p.id.0 == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no profile '{id}'")))?;

    if !profile.model.capabilities.reports_media {
        return Ok(Json(json!({ "detected": null })).into_response());
    }

    let connected = profile_is_connected(profile, state.host_discovery_enabled);
    if !connected {
        return Ok(Json(json!({ "detected": null })).into_response());
    }

    let profile = profile.clone();
    let sku = tokio::task::spawn_blocking(move || detect_loaded_media_sku(&profile))
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let detected = sku.and_then(|sku| detected_media_from_catalog(&state.catalog, &sku));
    Ok(Json(json!({ "detected": detected })).into_response())
}

fn detect_loaded_media_sku(profile: &PrinterProfile) -> Option<String> {
    if profile.model.protocol != Protocol::DymoLw {
        return None;
    }
    match &profile.transport {
        lbl_core::printer::Transport::Usb {
            vendor_id,
            product_id,
            serial,
        } => {
            let usb = lbl_device::UsbTransport::new(*vendor_id, *product_id, serial.clone());
            lbl_device::query_dymo_lw_loaded_media(&usb).ok().flatten()
        }
        _ => None,
    }
}

fn detected_media_from_catalog(catalog: &Catalog, sku: &str) -> Option<serde_json::Value> {
    let name = catalog.lookup(sku).map(|e| e.name.clone());
    Some(json!({
        "sku": sku,
        "name": name,
    }))
}

fn profile_is_connected(profile: &PrinterProfile, host_discovery_enabled: bool) -> bool {
    if !host_discovery_enabled {
        return false;
    }
    let discovered = lbl_device::discover();
    match &profile.transport {
        lbl_core::printer::Transport::Usb {
            vendor_id,
            product_id,
            serial,
        } => discovered.iter().any(|d| {
            d.vendor_id == Some(*vendor_id)
                && d.product_id == Some(*product_id)
                && serial
                    .as_ref()
                    .zip(d.serial.as_ref())
                    .map(|(a, b)| a == b)
                    .unwrap_or(true)
        }),
        lbl_core::printer::Transport::Serial { path, .. } => discovered
            .iter()
            .any(|d| d.connection == "serial" && d.path.as_deref() == Some(path.as_str())),
        lbl_core::printer::Transport::Ble { name, .. } => discovered.iter().any(|d| {
            d.connection == "ble"
                && d.path
                    .as_deref()
                    .is_some_and(|p| p.to_ascii_lowercase().contains(&name.to_ascii_lowercase()))
        }),
        lbl_core::printer::Transport::Network { .. } => false,
        lbl_core::printer::Transport::Browser { .. } => false,
    }
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
    #[serde(default)]
    template_format: TemplateFormat,
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
                format: self.template_format,
            });
        }
        Err(ApiError(
            StatusCode::BAD_REQUEST,
            "provide one of: text, html, template".into(),
        ))
    }
}

pub async fn preview(State(state): State<AppState>, Json(req): Json<PreviewReq>) -> ApiResult {
    let labels = authoring_labels(
        req.source.into_source()?,
        &lbl_template::BatchSelection::default(),
    )
    .map_err(ApiError::from)?;
    let count = labels.len();
    const PREVIEW_SUPERSAMPLE: u32 = 2;
    // Resolve the configured physical sizes against the standard preview DPI and
    // supersample factor so the browser preview matches printed sizing.
    let style_cfg = state.loader.load().map(|c| c.style).unwrap_or_default();
    let style = resolve_style(&style_cfg, req.dpi, PREVIEW_SUPERSAMPLE);
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm.or(Some(50.0)),
        req.length_mm,
        req.dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);
    let media_inset = resolve_media_inset(&style_cfg).to_px(req.dpi, PREVIEW_SUPERSAMPLE);
    let rotation = resolve_rotation(&state, req.orientation, req.rotate_cw, req.rotate_ccw);
    let viewport = render_viewport_px(&media, PREVIEW_SUPERSAMPLE, rotation);
    let media_info = json!({
        "width_mm": media.width_mm,
        "length_mm": match media.length {
            lbl_core::media::MediaLength::Fixed(mm) => serde_json::Value::from(mm),
            lbl_core::media::MediaLength::Continuous => serde_json::Value::Null,
        },
        "continuous": matches!(media.length, lbl_core::media::MediaLength::Continuous),
        "dpi": media.dpi.0,
        "width_px": viewport.width,
        "height_px": viewport.height,
    });
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
                    label_fit,
                    viewport: Some(viewport.clone()),
                    label_align,
                    label_valign,
                    label_fit_scale,
                    media_inset,
                    ..Default::default()
                },
            );
            json!({ "index": l.index, "html": html })
        })
        .collect();
    Ok(Json(json!({ "count": count, "labels": out, "media": media_info })).into_response())
}

#[derive(Deserialize)]
pub struct PreviewReq {
    #[serde(flatten)]
    source: SourceReq,
    media: Option<String>,
    width_mm: Option<f64>,
    length_mm: Option<f64>,
    #[serde(default = "default_dpi")]
    dpi: f64,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMode {
    #[default]
    Server,
    Client,
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
    /// Catalog printer key for browser transport hints (e.g. `LabelWriter 550`).
    #[serde(default)]
    printer: Option<String>,
    #[serde(default)]
    dispatch_mode: DispatchMode,
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
    bluetooth: Option<String>,
    #[serde(default)]
    use_sidecar: bool,
    /// For the virtual printer: output image format (png|bmp|tiff|gif|pbm).
    #[serde(default)]
    media_type: Option<String>,
    /// For the virtual printer: `raster` (default) or `vector` (PDF).
    #[serde(default)]
    export_mode: Option<String>,
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
        resolve_rotation(state, self.orientation, self.rotate_cw, self.rotate_ccw)
    }
}

fn resolve_rotation(
    state: &AppState,
    orientation: Option<lbl_core::Orientation>,
    rotate_cw: u32,
    rotate_ccw: u32,
) -> Rotation {
    let orientation = orientation.unwrap_or_else(|| {
        state
            .loader
            .load()
            .map(|c| c.render.orientation)
            .unwrap_or_default()
    });
    Rotation::for_print(orientation, rotate_cw, rotate_ccw)
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
        "html" => Protocol::Html,
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
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);
    let media_inset = resolve_media_inset(&style_cfg).to_px(req.dpi, req.supersample);

    let protocol = parse_protocol(&req.protocol)?;
    if req.dispatch_mode == DispatchMode::Client
        && matches!(
            protocol,
            Protocol::Virtual | Protocol::Console | Protocol::Html
        )
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "client dispatch_mode does not support virtual, console, or html protocols".into(),
        ));
    }

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
        virtual_export_mode: lbl_driver_file::VirtualExportMode::Raster,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
        media_inset,
    };

    let labels = authoring_labels(source, &lbl_template::BatchSelection::default())
        .map_err(ApiError::from)?;
    let use_sidecar = req.use_sidecar;
    let dispatch_mode = req.dispatch_mode;
    let network = req.network.clone();
    let usb = req.usb.clone();
    let serial = req.serial.clone();
    let bluetooth = req.bluetooth.clone();
    let catalog = state.catalog.clone();
    let printer_key = req.printer.clone();

    // The pipeline (browser render) is blocking; run it off the async runtime.
    let report = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let registry = Registry::with_builtin_drivers();
        let encoded: Vec<(String, Vec<u8>)> = if use_sidecar {
            encode_all(&SidecarBackend::node_default(), &registry, &labels, &opts)?
        } else {
            let backend = ChromiumBackend::launch()?;
            encode_all(&backend, &registry, &labels, &opts)?
        };
        if dispatch_mode == DispatchMode::Client {
            build_client_print_response(&catalog, protocol, printer_key.as_deref(), encoded)
        } else {
            dispatch(encoded, protocol, network, usb, serial, bluetooth)
        }
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
    let virtual_export_mode = if protocol == Protocol::Virtual {
        match &req.export_mode {
            Some(name) => lbl_driver_file::VirtualExportMode::parse(name)
                .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?,
            None => lbl_driver_file::VirtualExportMode::Raster,
        }
    } else {
        lbl_driver_file::VirtualExportMode::Raster
    };

    let media_type = if protocol == Protocol::Virtual {
        if virtual_export_mode == lbl_driver_file::VirtualExportMode::Vector {
            Some(lbl_driver_file::MediaType::Pdf)
        } else {
            Some(match &req.media_type {
                Some(name) => lbl_driver_file::MediaType::parse(name)
                    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?,
                None => lbl_driver_file::MediaType::Png,
            })
        }
    } else {
        None
    };

    let style_cfg = state.loader.load().map(|c| c.style).unwrap_or_default();
    let (style, media_inset) = if virtual_export_mode == lbl_driver_file::VirtualExportMode::Vector
    {
        (
            resolve_style_vector(&style_cfg),
            resolve_media_inset(&style_cfg).to_px(VECTOR_CSS_DPI, 1),
        )
    } else {
        (
            resolve_style(&style_cfg, req.dpi, req.supersample),
            resolve_media_inset(&style_cfg).to_px(req.dpi, req.supersample),
        )
    };
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);

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
        virtual_export_mode,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
        media_inset,
    };

    let labels = authoring_labels(source, &lbl_template::BatchSelection::default())
        .map_err(ApiError::from)?;
    let use_sidecar = req.use_sidecar;
    let want_debug = req.debug;
    let (extension, mime) = match media_type {
        Some(mt) => (mt.extension().to_string(), mt.mime().to_string()),
        None => ("bin".to_string(), "application/octet-stream".to_string()),
    };

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let mut registry = Registry::with_builtin_drivers();
        if let Some(mt) =
            media_type.filter(|_| virtual_export_mode == lbl_driver_file::VirtualExportMode::Raster)
        {
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
    bluetooth: Option<String>,
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
        let vendor_id = u16::from_str_radix(vid, 16)?;
        let product_id = u16::from_str_radix(pid, 16)?;
        let usb = lbl_device::UsbTransport::new(vendor_id, product_id, None);
        if protocol == Protocol::DymoLw {
            let mut t = lbl_device::DymoLwUsbTransport::new(usb);
            dispatch_encoded(encoded, protocol, &mut t)
        } else {
            let mut t = usb;
            dispatch_encoded(encoded, protocol, &mut t)
        }
    } else if let Some(target) = serial {
        let (path, baud) = parse_serial_target(&target);
        let mut t = lbl_device::SerialTransport::new(path, baud);
        dispatch_encoded(encoded, protocol, &mut t)
    } else if let Some(target) = bluetooth {
        dispatch_bluetooth(encoded, protocol, target)?
    } else {
        anyhow::bail!("no target; provide network, usb, serial, or bluetooth");
    };

    Ok(json!({
        "dispatch_mode": "server",
        "total": total,
        "completed": report.completed,
        "remaining": report.remaining,
        "disconnected": report.disconnected,
    }))
}

#[cfg(feature = "ble")]
fn dispatch_bluetooth(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    target: String,
) -> anyhow::Result<lbl_spool::SpoolReport> {
    let mut t = lbl_device::BleTransport::new(target);
    Ok(lbl::dispatch::dispatch_encoded(encoded, protocol, &mut t))
}

#[cfg(not(feature = "ble"))]
fn dispatch_bluetooth(
    _encoded: Vec<(String, Vec<u8>)>,
    _protocol: Protocol,
    _target: String,
) -> anyhow::Result<lbl_spool::SpoolReport> {
    anyhow::bail!(
        "Bluetooth LE support is not compiled in; rebuild lbl-server with `--features ble`"
    )
}

fn handshake_for_protocol(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::DymoLw => "dymo_lw",
        Protocol::Niimbot => "niimbot_poll",
        _ => "fire_and_forget",
    }
}

fn resolve_catalog_printer<'a>(
    catalog: &'a Catalog,
    printer_key: Option<&str>,
    protocol: Protocol,
) -> Option<&'a PrinterEntry> {
    if let Some(key) = printer_key {
        return catalog.lookup_printer(key);
    }
    catalog.printers().iter().find(|p| p.protocol == protocol)
}

/// Build browser transport hints from catalog connection entries.
pub fn browser_transport_hints(
    catalog: &Catalog,
    protocol: Protocol,
    printer_key: Option<&str>,
) -> serde_json::Value {
    let Some(printer) = resolve_catalog_printer(catalog, printer_key, protocol) else {
        return json!({ "api": default_browser_api(protocol) });
    };

    let mut webusb_filters = Vec::new();
    let mut ble_names = Vec::new();
    let mut has_serial = false;

    for conn in &printer.connections {
        match conn {
            ConnectionHint::Usb {
                vendor_id,
                product_id,
            } => {
                let mut filter = json!({ "vendorId": vendor_id });
                if let Some(pid) = product_id {
                    filter["productId"] = json!(pid);
                }
                webusb_filters.push(filter);
            }
            ConnectionHint::Serial { .. } => has_serial = true,
            ConnectionHint::Ble { name } => ble_names.push(name.clone()),
            ConnectionHint::Network { .. } => {}
        }
    }

    if !webusb_filters.is_empty() {
        return json!({ "api": "webusb", "filters": webusb_filters });
    }
    if has_serial {
        return json!({ "api": "web_serial" });
    }
    if !ble_names.is_empty() {
        return json!({ "api": "web_bluetooth", "names": ble_names });
    }

    json!({ "api": default_browser_api(protocol) })
}

fn default_browser_api(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::Niimbot => "web_serial",
        Protocol::Dymo | Protocol::DymoLw => "webusb",
        _ => "webusb",
    }
}

fn build_client_print_response(
    catalog: &Catalog,
    protocol: Protocol,
    printer_key: Option<&str>,
    encoded: Vec<(String, Vec<u8>)>,
) -> anyhow::Result<serde_json::Value> {
    use base64::Engine as _;

    let labels: Vec<serde_json::Value> = encoded
        .iter()
        .enumerate()
        .map(|(i, (name, bytes))| {
            json!({
                "index": i,
                "filename": name,
                "size": bytes.len(),
                "data_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
            })
        })
        .collect();

    Ok(json!({
        "dispatch_mode": "client",
        "protocol": opts_protocol_name(protocol),
        "handshake": handshake_for_protocol(protocol),
        "transport": browser_transport_hints(catalog, protocol, printer_key),
        "labels": labels,
    }))
}

#[cfg(test)]
mod browser_hints_tests {
    use super::*;
    use lbl_catalog::Catalog;

    #[test]
    fn dymo_lw_hints_webusb_with_filters() {
        let catalog = Catalog::bundled().unwrap();
        let hints = browser_transport_hints(&catalog, Protocol::DymoLw, Some("LabelWriter 550"));
        assert_eq!(hints["api"], "webusb");
        assert!(hints["filters"].as_array().unwrap().iter().any(|f| {
            f["vendorId"].as_u64() == Some(0x0922) && f["productId"].as_u64() == Some(0x0028)
        }));
    }

    #[test]
    fn niimbot_ble_hints() {
        let catalog = Catalog::bundled().unwrap();
        let hints = browser_transport_hints(&catalog, Protocol::Niimbot, Some("D110"));
        assert_eq!(hints["api"], "web_bluetooth");
        assert!(hints["names"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str() == Some("D110")));
    }

    #[test]
    fn client_response_shape() {
        let catalog = Catalog::bundled().unwrap();
        let resp = build_client_print_response(
            &catalog,
            Protocol::Dymo,
            Some("LabelWriter 450"),
            vec![("label-0000.bin".into(), vec![0x01, 0x02])],
        )
        .unwrap();
        assert_eq!(resp["dispatch_mode"], "client");
        assert_eq!(resp["handshake"], "fire_and_forget");
        assert_eq!(resp["labels"].as_array().unwrap().len(), 1);
        assert!(resp["labels"][0]["data_base64"].is_string());
    }

    #[test]
    fn detected_media_resolves_catalog_name() {
        let catalog = Catalog::bundled().unwrap();
        let detected = detected_media_from_catalog(&catalog, "11352").unwrap();
        assert_eq!(detected["sku"], "11352");
        assert!(detected["name"].as_str().unwrap().contains("11352"));
    }
}
