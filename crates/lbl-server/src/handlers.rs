//! Request handlers.

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use lbl::pipeline::{
    authoring_labels, encode_label, render_label_raster, resolve_font_fit_scale,
    resolve_label_align, resolve_label_fit, resolve_label_fit_scale, resolve_label_valign,
    resolve_media, resolve_media_inset, resolve_style, resolve_style_vector, transpile_label_html,
    PipelineOptions, Source, TemplateFormat, VECTOR_CSS_DPI,
};
use lbl_catalog::{encode_capabilities_for, Catalog, ConnectionHint, PrinterEntry};
use lbl_core::media::Media;
use lbl_core::printer::{PrinterCapabilities, PrinterProfile, Protocol};
use lbl_core::Rotation;
use lbl_dither::Algorithm;
use lbl_driver_niimbot::{NiimbotDriver, NiimbotTask};
use lbl_encode::Registry;
use lbl_render::{ChromiumBackend, RenderBackend, SidecarBackend};
use lbl_transpile_html::{injected_fit_font_px, AssetsBase, LabelFitSetting};

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

pub async fn profile_printer_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult {
    let profiles = state.profiles.load()?;
    let profile = profiles
        .iter()
        .find(|p| p.id.0 == id)
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no profile '{id}'")))?;

    let connected = profile_is_connected(profile, state.host_discovery_enabled);
    if !connected {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            "printer is not connected".into(),
        ));
    }

    let profile = profile.clone();
    let status = tokio::task::spawn_blocking(move || query_profile_print_status(&profile))
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e))?;

    Ok(Json(status).into_response())
}

fn query_profile_print_status(profile: &PrinterProfile) -> Result<lbl_device::PrintStatus, String> {
    match &profile.transport {
        lbl_core::printer::Transport::Usb {
            vendor_id,
            product_id,
            serial,
        } => {
            let usb = lbl_device::UsbTransport::new(*vendor_id, *product_id, serial.clone());
            lbl_device::query_print_status(profile.model.protocol, &usb).map_err(|e| e.to_string())
        }
        _ => Err("profile transport does not support status queries".into()),
    }
}

fn detect_loaded_media_sku(profile: &PrinterProfile) -> Option<String> {
    match &profile.transport {
        lbl_core::printer::Transport::Usb {
            vendor_id,
            product_id,
            serial,
        } => {
            let usb = lbl_device::UsbTransport::new(*vendor_id, *product_id, serial.clone());
            lbl_device::query_loaded_media_sku(profile.model.protocol, &usb)
                .ok()
                .flatten()
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
    markdown: Option<String>,
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
        if let Some(markdown) = self.markdown {
            return Ok(Source::Markdown(markdown));
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
            "provide one of: text, markdown, html, template".into(),
        ))
    }
}

pub async fn preview(State(state): State<AppState>, Json(req): Json<PreviewReq>) -> ApiResult {
    use base64::Engine as _;
    use lbl_driver_file::VirtualExportMode;
    use std::io::Cursor;

    let source = req.source.into_source()?;
    let labels = authoring_labels(source, &lbl_template::BatchSelection::default())
        .map_err(ApiError::from)?;
    let count = labels.len();
    let supersample = req.supersample;
    let dpi = resolve_request_dpi(&state.catalog, req.printer.as_deref(), None, req.dpi);
    // Preview targets the human reading frame, not the print head. The optional
    // printer key only selects native DPI; layout/rotation still follow the
    // chosen orientation without the head quarter-turn applied at encode time.
    let protocol = Protocol::Virtual;
    let style_cfg = load_style_cfg(&state, &req.style);
    let style = resolve_style(&style_cfg, dpi, supersample);
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm.or(Some(50.0)),
        req.length_mm,
        dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);
    let font_fit_scale = resolve_font_fit_scale(style_cfg.font_fit_scale);
    let media_inset = resolve_media_inset(&style_cfg).to_px(dpi, supersample);
    let rotation = resolve_rotation(
        &state,
        &media,
        req.orientation,
        req.rotate_cw,
        req.rotate_ccw,
    );
    let corner_radius_px = match media.length {
        lbl_core::media::MediaLength::Fixed(_) => style.corner_radius_px / supersample as f64,
        lbl_core::media::MediaLength::Continuous => 0.0,
    };
    let encode_caps = resolve_encode_caps(&state.catalog, req.printer.as_deref(), &media, false);
    let feed_along_width = rotation.swaps_axes();
    let opts = PipelineOptions {
        protocol,
        media: media.clone(),
        supports_cut: false,
        cut: false,
        copies: 1,
        dither: Algorithm::Auto,
        rotation,
        head_rotation: rotation,
        supersample,
        assets_base: AssetsBase::Cdn,
        style,
        media_type: None,
        virtual_export_mode: VirtualExportMode::Raster,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
        font_fit_scale,
        media_inset,
        encode_caps: encode_caps.clone(),
    };
    let px_per_mm = dpi * supersample as f64 / 25.4;

    let rendered =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<serde_json::Value>> {
            let backend = ChromiumBackend::launch()?;
            let mut out = Vec::with_capacity(count);
            for label in labels {
                let raster = render_label_raster(&backend, &label.html, &opts)?;
                let transpiled_html = raster.transpiled_html;
                let image = lbl::pipeline::pad_preview_head_tape(
                    raster.image,
                    &opts.media,
                    &encode_caps,
                    feed_along_width,
                );
                let padded =
                    lbl::pipeline::pad_preview_encode_feed(image, &encode_caps, feed_along_width);
                let image = padded.image;
                let mut png = Vec::new();
                image
                    .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
                    .context("encoding preview PNG")?;
                let computed_font_size_mm =
                    injected_fit_font_px(&transpiled_html).map(|px| px / px_per_mm);
                let mut label_json = json!({
                    "index": label.index,
                    "width_px": image.width(),
                    "height_px": image.height(),
                    "image_base64": base64::engine::general_purpose::STANDARD.encode(&png),
                });
                if padded.trail_feed_px > 0 || padded.lead_feed_px > 0 {
                    label_json["content_feed_end_px"] =
                        serde_json::Value::from(padded.content_feed_end_px);
                    label_json["feed_trail_px"] = serde_json::Value::from(padded.trail_feed_px);
                    if padded.feed_end_margin_px > 0 {
                        label_json["feed_end_margin_px"] =
                            serde_json::Value::from(padded.feed_end_margin_px);
                    }
                    if padded.lead_feed_px > 0 {
                        label_json["feed_lead_px"] = serde_json::Value::from(padded.lead_feed_px);
                    }
                }
                if let Some(mm) = computed_font_size_mm {
                    label_json["computed_font_size_mm"] = serde_json::Value::from(mm);
                }
                out.push(label_json);
            }
            Ok(out)
        })
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (width_px, height_px) = rendered
        .first()
        .map(|l| {
            (
                l["width_px"].as_u64().unwrap_or(0) as f64,
                l["height_px"].as_u64().unwrap_or(0) as f64,
            )
        })
        .unwrap_or((0.0, 0.0));
    let media_info = json!({
        "width_mm": media.width_mm,
        "length_mm": match media.length {
            lbl_core::media::MediaLength::Fixed(mm) => serde_json::Value::from(mm),
            lbl_core::media::MediaLength::Continuous => serde_json::Value::Null,
        },
        "continuous": matches!(media.length, lbl_core::media::MediaLength::Continuous),
        "dpi": media.dpi.0,
        "width_px": width_px,
        "height_px": height_px,
        "corner_radius_px": corner_radius_px,
    });

    Ok(Json(json!({ "count": count, "labels": rendered, "media": media_info })).into_response())
}

pub async fn preview_html(State(state): State<AppState>, Json(req): Json<PreviewReq>) -> ApiResult {
    let source = req.source.into_source()?;
    let labels = authoring_labels(source, &lbl_template::BatchSelection::default())
        .map_err(ApiError::from)?;
    let count = labels.len();
    let dpi = resolve_request_dpi(&state.catalog, req.printer.as_deref(), None, req.dpi);
    let style_cfg = load_style_cfg(&state, &req.style);
    let style = resolve_style_vector(&style_cfg);
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm.or(Some(50.0)),
        req.length_mm,
        dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);
    let font_fit_scale = resolve_font_fit_scale(style_cfg.font_fit_scale);
    let media_inset = resolve_media_inset(&style_cfg).to_px(VECTOR_CSS_DPI, 1);
    let rotation = resolve_rotation(
        &state,
        &media,
        req.orientation,
        req.rotate_cw,
        req.rotate_ccw,
    );
    let opts = PipelineOptions {
        protocol: Protocol::Virtual,
        media: media.clone(),
        supports_cut: false,
        cut: false,
        copies: 1,
        dither: Algorithm::Auto,
        rotation,
        head_rotation: rotation,
        supersample: 1,
        assets_base: AssetsBase::Cdn,
        style,
        media_type: None,
        virtual_export_mode: lbl_driver_file::VirtualExportMode::Vector,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
        font_fit_scale,
        media_inset,
        encode_caps: resolve_encode_caps(&state.catalog, req.printer.as_deref(), &media, false),
    };
    let px_per_mm = VECTOR_CSS_DPI / 25.4;

    let mut rendered = Vec::with_capacity(count);
    for label in labels {
        let transpiled = transpile_label_html(&label.html, &opts)
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let computed_font_size_mm = injected_fit_font_px(&transpiled.html).map(|px| px / px_per_mm);
        let mut label_json = json!({
            "index": label.index,
            "html": transpiled.html,
            "width_px": transpiled.width_px,
            "height_px": transpiled.height_px,
        });
        if let Some(mm) = computed_font_size_mm {
            label_json["computed_font_size_mm"] = serde_json::Value::from(mm);
        }
        rendered.push(label_json);
    }

    let (width_px, height_px) = rendered
        .first()
        .map(|l| {
            (
                l["width_px"].as_f64().unwrap_or(0.0),
                l["height_px"].as_f64().unwrap_or(0.0),
            )
        })
        .unwrap_or((0.0, 0.0));
    let media_info = json!({
        "width_mm": media.width_mm,
        "length_mm": match media.length {
            lbl_core::media::MediaLength::Fixed(mm) => serde_json::Value::from(mm),
            lbl_core::media::MediaLength::Continuous => serde_json::Value::Null,
        },
        "continuous": matches!(media.length, lbl_core::media::MediaLength::Continuous),
        "dpi": VECTOR_CSS_DPI,
        "width_px": width_px,
        "height_px": height_px,
        "corner_radius_px": match media.length {
            lbl_core::media::MediaLength::Fixed(_) => opts.style.corner_radius_px,
            lbl_core::media::MediaLength::Continuous => 0.0,
        },
    });

    Ok(Json(json!({ "count": count, "labels": rendered, "media": media_info })).into_response())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StyleReqOverrides {
    padding_mm: Option<f64>,
    element_gap_mm: Option<f64>,
    border_width_mm: Option<f64>,
    font_size_mm: Option<f64>,
    font_fit_scale: Option<f64>,
    label_fit: Option<String>,
    label_align: Option<String>,
    label_valign: Option<String>,
}

fn load_style_cfg(state: &AppState, overrides: &StyleReqOverrides) -> lbl_config::StyleConfig {
    let mut style = state.loader.load().map(|c| c.style).unwrap_or_default();
    if let Some(v) = overrides.padding_mm {
        style.padding_mm = v;
    }
    if let Some(v) = overrides.element_gap_mm {
        style.element_gap_mm = v;
    }
    if let Some(v) = overrides.border_width_mm {
        style.border_width_mm = v;
    }
    if let Some(v) = overrides.font_size_mm {
        style.font_size_mm = v;
    }
    if let Some(v) = overrides.font_fit_scale {
        style.font_fit_scale = v;
    }
    if let Some(v) = &overrides.label_fit {
        style.label_fit = v.clone();
    }
    if let Some(v) = &overrides.label_align {
        style.label_align = v.clone();
    }
    if let Some(v) = &overrides.label_valign {
        style.label_valign = v.clone();
    }
    style
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
    /// Catalog printer key (e.g. `B1`) used to pick native DPI.
    #[serde(default)]
    printer: Option<String>,
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
    /// Render supersample factor (same default as print).
    #[serde(default = "default_supersample")]
    supersample: u32,
    #[serde(flatten, default)]
    style: StyleReqOverrides,
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
    #[serde(flatten, default)]
    style: StyleReqOverrides,
    /// Also build the HTML pipeline debug report.
    #[serde(default)]
    debug: bool,
}

impl PrintReq {
    /// Resolve the net [`lbl_core::Rotation`] for this request: the explicit
    /// orientation (or the configured default) plus any extra quarter-turns.
    fn rotation(&self, state: &AppState, media: &Media) -> lbl_core::Rotation {
        resolve_rotation(
            state,
            media,
            self.orientation,
            self.rotate_cw,
            self.rotate_ccw,
        )
    }
}

fn resolve_encode_caps(
    catalog: &Catalog,
    printer_key: Option<&str>,
    media: &Media,
    supports_cut: bool,
) -> PrinterCapabilities {
    let printer = printer_key.and_then(|k| catalog.lookup_printer(k));
    encode_capabilities_for(printer, media, supports_cut)
}

fn resolve_rotation(
    state: &AppState,
    media: &Media,
    orientation: Option<lbl_core::Orientation>,
    rotate_cw: u32,
    rotate_ccw: u32,
) -> Rotation {
    let orientation = resolve_orientation(state, orientation);
    Rotation::for_print_with_media(orientation, media, rotate_cw, rotate_ccw)
}

fn resolve_head_rotation(
    state: &AppState,
    media: &Media,
    orientation: Option<lbl_core::Orientation>,
    rotate_cw: u32,
    rotate_ccw: u32,
    protocol: Protocol,
) -> Rotation {
    let orientation = resolve_orientation(state, orientation);
    Rotation::for_head_with_media(orientation, media, rotate_cw, rotate_ccw, protocol)
}

fn resolve_orientation(
    state: &AppState,
    orientation: Option<lbl_core::Orientation>,
) -> lbl_core::Orientation {
    orientation.unwrap_or_else(|| {
        state
            .loader
            .load()
            .map(|c| c.render.orientation)
            .unwrap_or_default()
    })
}

/// Pick the render DPI: explicit user overrides win; otherwise use the printer's
/// native resolution from the catalog (same rule as the CLI).
fn resolve_request_dpi(
    catalog: &Catalog,
    printer_key: Option<&str>,
    protocol: Option<Protocol>,
    req_dpi: f64,
) -> f64 {
    let protocol = protocol
        .or_else(|| printer_key.and_then(|k| catalog.lookup_printer(k).map(|p| p.protocol)));
    match protocol {
        Some(p) => catalog.resolve_dpi(printer_key, p, req_dpi),
        None => catalog.resolve_dpi(printer_key, Protocol::Virtual, req_dpi),
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
        "b1" | "d11" | "d110" | "niimbot" => Protocol::Niimbot,
        "brother-ql" | "brother_ql" | "brotherql" | "ql820" => Protocol::BrotherQl,
        "console" | "term" => Protocol::Console,
        "dymo" => Protocol::Dymo,
        "dymo-lw" | "dymolw" | "lw550" => Protocol::DymoLw,
        "esc/pos" | "escpos" => Protocol::EscPos,
        "file" | "virtual" => Protocol::Virtual,
        "html" => Protocol::Html,
        "tspl" => Protocol::Tspl,
        "zpl" => Protocol::Zpl,
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
    let protocol = parse_protocol(&req.protocol)?;
    let dpi = resolve_request_dpi(
        &state.catalog,
        req.printer.as_deref(),
        Some(protocol),
        req.dpi,
    );
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm,
        req.length_mm,
        dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;

    let style_cfg = load_style_cfg(&state, &req.style);
    let style = resolve_style(&style_cfg, dpi, req.supersample);
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);
    let font_fit_scale = resolve_font_fit_scale(style_cfg.font_fit_scale);
    let media_inset = resolve_media_inset(&style_cfg).to_px(dpi, req.supersample);

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

    let rotation = req.rotation(&state, &media);
    let head_rotation = resolve_head_rotation(
        &state,
        &media,
        req.orientation,
        req.rotate_cw,
        req.rotate_ccw,
        protocol,
    );
    let encode_caps = resolve_encode_caps(
        &state.catalog,
        req.printer.as_deref(),
        &media,
        req.supports_cut,
    );
    let opts = PipelineOptions {
        protocol,
        media,
        supports_cut: req.supports_cut,
        cut: req.cut,
        copies: req.copies,
        dither: Algorithm::parse(&req.dither)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?,
        rotation,
        head_rotation,
        supersample: req.supersample,
        assets_base: AssetsBase::Cdn,
        style,
        media_type: None,
        virtual_export_mode: lbl_driver_file::VirtualExportMode::Raster,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
        font_fit_scale,
        media_inset,
        encode_caps,
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
        let registry = if protocol == Protocol::Niimbot {
            niimbot_registry(printer_key.as_deref())
        } else {
            Registry::with_builtin_drivers()
        };
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
    let protocol = parse_protocol(&req.protocol)?;
    let dpi = resolve_request_dpi(
        &state.catalog,
        req.printer.as_deref(),
        Some(protocol),
        req.dpi,
    );
    let media = resolve_media(
        &state.catalog,
        req.media.as_deref(),
        req.width_mm,
        req.length_mm,
        dpi,
    )
    .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?;

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

    let style_cfg = load_style_cfg(&state, &req.style);
    let (style, media_inset) = if virtual_export_mode == lbl_driver_file::VirtualExportMode::Vector
    {
        (
            resolve_style_vector(&style_cfg),
            resolve_media_inset(&style_cfg).to_px(VECTOR_CSS_DPI, 1),
        )
    } else {
        (
            resolve_style(&style_cfg, dpi, req.supersample),
            resolve_media_inset(&style_cfg).to_px(dpi, req.supersample),
        )
    };
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(style_cfg.label_fit_scale);
    let font_fit_scale = resolve_font_fit_scale(style_cfg.font_fit_scale);

    let rotation = req.rotation(&state, &media);
    let head_rotation = resolve_head_rotation(
        &state,
        &media,
        req.orientation,
        req.rotate_cw,
        req.rotate_ccw,
        protocol,
    );
    let encode_caps = resolve_encode_caps(
        &state.catalog,
        req.printer.as_deref(),
        &media,
        req.supports_cut,
    );
    let opts = PipelineOptions {
        protocol,
        media,
        supports_cut: req.supports_cut,
        cut: req.cut,
        copies: req.copies,
        dither: Algorithm::parse(&req.dither)
            .map_err(|e| ApiError(StatusCode::BAD_REQUEST, e.to_string()))?,
        rotation,
        head_rotation,
        supersample: req.supersample,
        assets_base: AssetsBase::Cdn,
        style,
        media_type,
        virtual_export_mode,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
        font_fit_scale,
        media_inset,
        encode_caps,
    };

    let labels = authoring_labels(source, &lbl_template::BatchSelection::default())
        .map_err(ApiError::from)?;
    let use_sidecar = req.use_sidecar;
    let want_debug = req.debug;
    let (extension, mime) = match media_type {
        Some(mt) => (mt.extension().to_string(), mt.mime().to_string()),
        None => ("bin".to_string(), "application/octet-stream".to_string()),
    };

    let printer_key = req.printer.clone();

    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        let mut registry = if protocol == Protocol::Niimbot {
            niimbot_registry(printer_key.as_deref())
        } else {
            Registry::with_builtin_drivers()
        };
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
        Protocol::Dymo => "dymo_d1",
        Protocol::DymoLw => "dymo_lw",
        Protocol::Niimbot => "niimbot_poll",
        _ => "fire_and_forget",
    }
}

fn resolve_niimbot_task_name(printer_key: Option<&str>) -> Option<&'static str> {
    printer_key
        .and_then(NiimbotDriver::task_for_printer_key)
        .map(NiimbotDriver::task_name)
}

fn niimbot_registry(printer_key: Option<&str>) -> Registry {
    let mut registry = Registry::with_builtin_drivers();
    if let Some(task) = printer_key.and_then(NiimbotDriver::task_for_printer_key) {
        match task {
            NiimbotTask::V4 => registry.register(Box::new(NiimbotDriver::v4())),
            NiimbotTask::B1 => registry.register(Box::new(NiimbotDriver::b1())),
            NiimbotTask::Standard => {}
        }
    }
    registry
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
    if !ble_names.is_empty() {
        return json!({ "api": "web_bluetooth", "names": ble_names });
    }
    if has_serial {
        return json!({ "api": "web_serial" });
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

    let handshake = handshake_for_protocol(protocol);
    let niimbot_task = resolve_niimbot_task_name(printer_key);

    Ok(json!({
        "dispatch_mode": "client",
        "protocol": opts_protocol_name(protocol),
        "handshake": handshake,
        "transport": browser_transport_hints(catalog, protocol, printer_key),
        "niimbot_task": niimbot_task,
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
    fn niimbot_b1_ble_hints() {
        let catalog = Catalog::bundled().unwrap();
        let hints = browser_transport_hints(&catalog, Protocol::Niimbot, Some("B1"));
        assert_eq!(hints["api"], "web_bluetooth");
        assert!(hints["names"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str() == Some("B1")));
    }

    #[test]
    fn niimbot_b1_client_response_includes_task() {
        let catalog = Catalog::bundled().unwrap();
        let resp = build_client_print_response(
            &catalog,
            Protocol::Niimbot,
            Some("B1"),
            vec![("label-0000.bin".into(), vec![0x55, 0x55])],
        )
        .unwrap();
        assert_eq!(resp["niimbot_task"], "b1");
        assert_eq!(resp["transport"]["api"], "web_bluetooth");
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
        assert_eq!(resp["handshake"], "dymo_d1");
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
