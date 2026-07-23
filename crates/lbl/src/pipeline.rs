//! Pipeline chaining used by the orchestrator's high-level flows.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use image::RgbaImage;
use lbl_catalog::{Catalog, ConnectionHint, DeviceEntry};
use lbl_core::job::{CutMode, JobSpec, OutputMode};
use lbl_core::media::Media;
use lbl_core::printer::{DeviceCapabilities, Protocol};
use lbl_core::units::{Dpi, Millimeters, CSS_LAYOUT_REFERENCE_DPI};
use lbl_core::MonoBitmap;
use lbl_core::Rotation;
use lbl_dither::{dither, split_black_red, Algorithm};
use lbl_driver_api::EncodeContext;
use lbl_driver_file::{MediaType, VirtualExportMode};
use lbl_encode::Registry;
use lbl_pattern::sample_pattern_for_media;
use lbl_render::{
    apply_mirror_horizontal, apply_rotation, render_two_pass, PdfExportRequest, RenderBackend,
    RenderRequest,
};

type PrintTransportTargets = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
pub use lbl_template::BatchSelection;
use lbl_template::{select_batch_indices, Engine, RenderOptions};
use lbl_transpile_html::{
    transpile, AssetsBase, CascadingInsetMm, FontDelivery, LabelAlign, LabelFit, LabelFitSetting,
    LabelStyle, LabelValign, MediaInset, MediaInsetPx, PageSizeMm, QrErrorCorrection,
    TranspileOptions, ViewportPx,
};

/// CSS reference resolution for vector PDF export. Alias for
/// [`CSS_LAYOUT_REFERENCE_DPI`]; the configured printer DPI does not affect
/// vector output.
pub const VECTOR_CSS_DPI: f64 = CSS_LAYOUT_REFERENCE_DPI;

/// A single authoring-HTML label with its batch index.
#[derive(Debug, Clone)]
pub struct AuthoringLabel {
    /// Zero-based index within the batch.
    pub index: usize,
    /// Authoring HTML (pre-transpilation).
    pub html: String,
}

/// How a rendered [`Source::Template`] body is turned into authoring HTML.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TemplateFormat {
    /// Plain text: run through `lbl-text` after rendering (default).
    #[default]
    Text,
    /// Markdown: run through `lbl-markdown` after rendering.
    Markdown,
    /// The template body is already authoring HTML.
    Html,
}

/// Infer a template body format from a template path's extension.
///
/// Returns `None` for stdin (`-`), inline templates without an extension, and
/// unknown extensions.
pub fn infer_template_format_from_path(template: &str) -> Option<TemplateFormat> {
    if template == "-" {
        return None;
    }
    let ext = std::path::Path::new(template).extension()?.to_str()?;
    match ext.to_ascii_lowercase().as_str() {
        "htm" | "html" | "lbl" => Some(TemplateFormat::Html),
        "markdown" | "md" => Some(TemplateFormat::Markdown),
        "text" | "txt" => Some(TemplateFormat::Text),
        _ => None,
    }
}

/// Resolve the template body format, preferring an explicit override.
pub fn resolve_template_format(template: &str, explicit: Option<TemplateFormat>) -> TemplateFormat {
    explicit.unwrap_or_else(|| {
        infer_template_format_from_path(template).unwrap_or(TemplateFormat::Text)
    })
}

/// The input to a flow.
#[derive(Debug, Clone)]
pub enum Source {
    /// Plain text run through `lbl-text` (with inline directives unless `raw`).
    Text {
        /// The text.
        text: String,
        /// Disable inline mini-syntax.
        raw: bool,
    },
    /// Markdown run through `lbl-markdown` (inline directives still apply).
    Markdown(String),
    /// Already-authoring HTML content.
    Html(String),
    /// A template rendered against optional data, optionally batched.
    Template {
        /// The template source (may contain frontmatter).
        template: String,
        /// External data (already parsed to JSON), if any.
        data: Option<serde_json::Value>,
        /// JSON-pointer to a batch array, if any.
        each: Option<String>,
        /// How to interpret each rendered label body.
        format: TemplateFormat,
    },
}

/// Turn a [`Source`] into one or more authoring-HTML labels.
///
/// Date/time `<stamp>` elements are resolved once for the job using the host
/// local clock so every label in a batch shares the same instant.
pub fn authoring_labels(source: Source, selection: &BatchSelection) -> Result<Vec<AuthoringLabel>> {
    let now = chrono::Local::now();
    let labels = authoring_labels_unresolved(source, selection)?;
    labels
        .into_iter()
        .map(|mut label| {
            label.html = lbl_text::resolve_stamps_at(&label.html, now)
                .map_err(|e| anyhow!("resolving date/time stamps: {e}"))?;
            Ok(label)
        })
        .collect()
}

fn authoring_labels_unresolved(
    source: Source,
    selection: &BatchSelection,
) -> Result<Vec<AuthoringLabel>> {
    match source {
        Source::Text { text, raw } => {
            let record = serde_json::json!({ "text": text, "raw": raw });
            select_batch_indices(std::slice::from_ref(&record), selection)?;
            let doc = lbl_text::Document::parse(&text, raw);
            Ok(vec![AuthoringLabel {
                index: 0,
                html: doc.to_authoring_document(),
            }])
        }
        Source::Markdown(markdown) => {
            let record = serde_json::json!({ "markdown": markdown });
            select_batch_indices(std::slice::from_ref(&record), selection)?;
            let doc = lbl_markdown::MarkdownDocument::parse(&markdown);
            Ok(vec![AuthoringLabel {
                index: 0,
                html: doc.to_authoring_document(),
            }])
        }
        Source::Html(html) => {
            let record = serde_json::json!({ "html": html });
            select_batch_indices(std::slice::from_ref(&record), selection)?;
            Ok(vec![AuthoringLabel { index: 0, html }])
        }
        Source::Template {
            template,
            data,
            each,
            format,
        } => {
            let labels = Engine::new()
                .render(
                    &template,
                    data,
                    &RenderOptions {
                        each,
                        selection: selection.clone(),
                    },
                )
                .context("rendering template")?;
            Ok(labels
                .into_iter()
                .map(|l| AuthoringLabel {
                    index: l.index,
                    html: template_render_to_authoring(&l.html, format),
                })
                .collect())
        }
    }
}

fn template_render_to_authoring(rendered: &str, format: TemplateFormat) -> String {
    match format {
        TemplateFormat::Text => lbl_text::Document::parse(rendered, false).to_authoring_document(),
        TemplateFormat::Markdown => {
            lbl_markdown::MarkdownDocument::parse(rendered).to_authoring_document()
        }
        TemplateFormat::Html => rendered.to_string(),
    }
}

/// Resolve a [`Media`] from an optional catalog SKU and/or explicit dimensions.
///
/// Precedence: a catalog `sku` (resolved at `dpi`) wins; otherwise explicit
/// `width_mm` (+ optional `length_mm`) is used.
pub fn resolve_media(
    catalog: &Catalog,
    sku: Option<&str>,
    width_mm: Option<f64>,
    length_mm: Option<f64>,
    dpi: f64,
) -> Result<Media> {
    let dpi = Dpi(dpi);
    if let Some(sku) = sku {
        let entry = catalog
            .lookup(sku)
            .ok_or_else(|| anyhow!("unknown media SKU '{sku}'"))?;
        return Ok(entry.media.to_media(dpi));
    }
    let width =
        width_mm.ok_or_else(|| anyhow!("media required: pass --media SKU or --width-mm"))?;
    Ok(match length_mm {
        Some(len) => Media::fixed(width, len, dpi),
        None => Media::continuous(width, dpi),
    })
}

/// Resolve transport targets for printing, filling in catalog defaults when the
/// caller did not pass explicit `--network`, `--usb`, `--serial`, or
/// `--bluetooth` flags.
pub fn resolve_print_transport(
    printer: Option<&DeviceEntry>,
    network: Option<String>,
    usb: Option<String>,
    serial: Option<String>,
    bluetooth: Option<String>,
) -> Result<PrintTransportTargets> {
    let mut network = network;
    let mut usb = usb;
    let mut serial = serial;
    let mut bluetooth = bluetooth;

    if network.is_none() && usb.is_none() && serial.is_none() && bluetooth.is_none() {
        let Some(printer) = printer else {
            return Ok((network, usb, serial, bluetooth));
        };
        let defaults = printer.default_transport();
        network = defaults.network;
        usb = defaults.usb;
        serial = defaults.serial;
        bluetooth = defaults.bluetooth;

        if serial.is_none()
            && printer
                .connections
                .iter()
                .any(|c| matches!(c, ConnectionHint::Serial { path: None }))
        {
            serial = discover_serial_port(printer);
        }

        if network.is_none() && usb.is_none() && serial.is_none() && bluetooth.is_none() {
            bail!(
                "no transport for printer '{}'; pass --network, --usb, --serial, or --bluetooth",
                printer.canonical_key()
            );
        }
    }

    Ok((network, usb, serial, bluetooth))
}

/// USB target for querying print-engine status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusTarget {
    /// Protocol spoken by the printer.
    pub protocol: Protocol,
    /// `vid:pid` in hex for [`lbl_device::UsbTransport`].
    pub usb: String,
    /// Optional serial to disambiguate when several identical models are connected.
    pub serial: Option<String>,
}

/// Resolve which USB printer to query for status, using the same sources as
/// [`resolve_print_transport`] (`--usb`, `[print] usb`, `--printer` catalog
/// defaults), then saved profiles (`--profile`, `[general] default_printer`),
/// then auto-discovery when exactly one status-capable printer is connected.
pub fn resolve_status_target(
    catalog: &Catalog,
    config: &lbl_config::Config,
    printer_key: Option<&str>,
    profile_id: Option<&str>,
    usb_override: Option<String>,
) -> Result<StatusTarget> {
    if let Some(usb) = usb_override {
        let protocol = infer_usb_protocol(&usb)?;
        return Ok(StatusTarget {
            protocol,
            usb,
            serial: None,
        });
    }

    let printer_entry = match printer_key {
        Some(key) => Some(catalog.require_device(key).map_err(|e| anyhow!(e))?),
        None => None,
    };

    if let Some(printer) = printer_entry {
        if !lbl_device::status_supported(printer.protocol) {
            bail!(
                "printer '{}' does not support status queries",
                printer.canonical_key()
            );
        }
    }

    let (_, usb, _, _) =
        resolve_print_transport(printer_entry, None, config.print.usb.clone(), None, None)?;

    if let Some(usb) = usb {
        let protocol = match printer_entry {
            Some(p) => p.protocol,
            None => infer_usb_protocol(&usb)?,
        };
        if !lbl_device::status_supported(protocol) {
            bail!("printer at USB {usb} does not support status queries");
        }
        return Ok(StatusTarget {
            protocol,
            usb,
            serial: None,
        });
    }

    if let Some(id) = profile_id.or(config.general.default_printer.as_deref()) {
        return status_usb_from_profile(id);
    }

    discover_status_usb_target(printer_entry.map(|p| p.protocol))
}

fn infer_usb_protocol(usb: &str) -> Result<Protocol> {
    let (vid, pid) = parse_usb_vid_pid(usb)?;
    lbl_device::discover_usb()
        .into_iter()
        .find(|d| d.vendor_id == Some(vid) && d.product_id == Some(pid))
        .and_then(|d| d.protocol)
        .filter(|&p| lbl_device::status_supported(p))
        .ok_or_else(|| {
            anyhow!(
                "cannot determine printer protocol for USB {usb}; \
                 pass --printer or --profile"
            )
        })
}

fn parse_usb_vid_pid(usb: &str) -> Result<(u16, u16)> {
    let (vid, pid) = usb
        .split_once(':')
        .ok_or_else(|| anyhow!("usb target must be vid:pid (hex)"))?;
    Ok((u16::from_str_radix(vid, 16)?, u16::from_str_radix(pid, 16)?))
}

fn status_usb_from_profile(profile_id: &str) -> Result<StatusTarget> {
    use lbl_config::ProfileStore;
    use lbl_core::printer::Transport;

    let loader = lbl_config::Loader::new();
    let store = ProfileStore::new(loader.paths().profiles.clone());
    let profiles = store.load()?;
    let profile = profiles
        .iter()
        .find(|p| p.id.0 == profile_id)
        .ok_or_else(|| anyhow!("no profile '{profile_id}'"))?;
    if !lbl_device::status_supported(profile.model.protocol) {
        bail!("profile '{profile_id}' does not support status queries");
    }
    match &profile.transport {
        Transport::Usb {
            vendor_id,
            product_id,
            serial,
        } => Ok(StatusTarget {
            protocol: profile.model.protocol,
            usb: format!("{vendor_id:04x}:{product_id:04x}"),
            serial: serial.clone(),
        }),
        _ => bail!("profile '{profile_id}' has no USB transport"),
    }
}

fn discover_status_usb_target(expected: Option<Protocol>) -> Result<StatusTarget> {
    let candidates: Vec<_> = lbl_device::discover_usb()
        .into_iter()
        .filter(|d| {
            d.protocol
                .is_some_and(|p| lbl_device::status_supported(p) && expected.is_none_or(|e| e == p))
        })
        .collect();
    match candidates.len() {
        0 => bail!(
            "no printer supporting status queries found; connect one or pass \
             --printer, --profile, or --usb (same targets as lbl print)"
        ),
        1 => {
            let d = &candidates[0];
            Ok(StatusTarget {
                protocol: d.protocol.expect("filtered for protocol"),
                usb: format!(
                    "{:04x}:{:04x}",
                    d.vendor_id
                        .ok_or_else(|| anyhow!("discovered printer missing vendor id"))?,
                    d.product_id
                        .ok_or_else(|| anyhow!("discovered printer missing product id"))?
                ),
                serial: d.serial.clone(),
            })
        }
        n => {
            let lines: Vec<String> = candidates
                .iter()
                .map(|d| {
                    format!(
                        "  {:04x}:{:04x}{}",
                        d.vendor_id.unwrap_or(0),
                        d.product_id.unwrap_or(0),
                        d.serial
                            .as_ref()
                            .map(|s| format!(" (serial {s})"))
                            .unwrap_or_default()
                    )
                })
                .collect();
            bail!(
                "{n} printers supporting status queries found; pass --usb or --profile:\n{}",
                lines.join("\n")
            )
        }
    }
}

fn discover_serial_port(printer: &DeviceEntry) -> Option<String> {
    lbl_device::discover_serial()
        .into_iter()
        .find(|d| {
            d.path.is_some()
                && d.protocol == Some(Protocol::Niimbot)
                && d.model
                    .as_ref()
                    .is_none_or(|model| printer.matches_model(model))
        })
        .and_then(|d| d.path)
}

/// Resolve a configured (millimetre) [`lbl_config::StyleConfig`] into the
/// pixel-based [`LabelStyle`] used by transpilation, given the render `dpi` and
/// `supersample` factor.
pub fn resolve_style(style: &lbl_config::StyleConfig, dpi: f64, supersample: u32) -> LabelStyle {
    let mut label = LabelStyle::from_mm(
        style.font_size_mm,
        style.qr_size_mm,
        style.barcode_height_mm,
        style.barcode_module_width_mm,
        resolve_label_padding(style),
        style.element_gap_mm,
        style.border_width_mm,
        style.corner_radius_mm,
        dpi,
        supersample,
    );
    label.qr_error_correction =
        QrErrorCorrection::parse(&style.qr_error_correction).unwrap_or_default();
    label.qr_margin = style.qr_margin;
    label.qr_dark = style.qr_dark.clone();
    label.qr_light = style.qr_light.clone();
    label
}

/// Resolve style for vector PDF export (96 CSS dpi, no supersampling).
pub fn resolve_style_vector(style: &lbl_config::StyleConfig) -> LabelStyle {
    resolve_style(style, VECTOR_CSS_DPI, 1)
}

/// Options for encoding a label all the way to protocol bytes.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    /// Target protocol.
    pub protocol: Protocol,
    /// Resolved media.
    pub media: Media,
    /// Whether the printer can cut.
    pub supports_cut: bool,
    /// When to cut during the job.
    pub cut_mode: CutMode,
    /// Copies.
    pub copies: u32,
    /// 0-based index within a multi-label batch encode.
    pub batch_index: u32,
    /// Total labels in the batch encode (`1` = standalone).
    pub batch_total: u32,
    /// Optional print density / heat (driver-specific).
    pub density: Option<u8>,
    /// Requested feed lead padding (mm). `None` → resolve uses \(D_x\) when known.
    pub feed_lead_mm: Option<f64>,
    /// Requested feed end padding (mm). `None` → 0.
    pub feed_end_mm: Option<f64>,
    /// Opt-in pre-cut. `None` → catalog `precut_default`.
    pub precut: Option<bool>,
    /// Protocol-specific options (each driver reads only its own bag).
    pub driver: lbl_core::DriverOptions,
    /// Dithering algorithm.
    pub dither: Algorithm,
    /// Net rotation for layout viewport sizing (reading frame).
    pub rotation: Rotation,
    /// Quarter-turn applied to the raster before encode. Matches [`rotation`] for
    /// row-oriented heads; feed-oriented DYMO drivers invert portrait/landscape.
    pub head_rotation: Rotation,
    /// Flip the reading-frame raster left↔right before head rotation (Mirror print).
    /// Independent of catalog [`DeviceCapabilities::feed_reverse`].
    pub mirror: bool,
    /// Supersample factor.
    pub supersample: u32,
    /// Where transpilation loads JS libraries from.
    pub assets_base: AssetsBase,
    /// How catalog web fonts are injected into transpiled HTML.
    pub font_delivery: FontDelivery,
    /// Font / QR / barcode sizing (already resolved to pixels for this run's
    /// DPI and supersample factor; see [`resolve_style`]).
    pub style: LabelStyle,
    /// For the virtual (`Protocol::Virtual`) printer, the output file format
    /// ("media type"). Ignored by hardware protocols and vector export mode.
    pub media_type: Option<MediaType>,
    /// Virtual-printer export mode: raster image vs vector PDF.
    pub virtual_export_mode: VirtualExportMode,
    /// How the label root fills the render viewport (resolved from config/CLI).
    pub label_fit: LabelFit,
    /// Cross-axis alignment when the viewport width is known (resolved from config/CLI).
    pub label_align: LabelAlign,
    /// Main-axis alignment in fill mode (resolved from config/CLI).
    pub label_valign: LabelValign,
    /// Fit-box scale in fill mode (resolved from config/CLI).
    pub label_fit_scale: f64,
    /// Auto-fit text scale in fill mode (resolved from config/CLI).
    pub font_fit_scale: f64,
    /// Inset from the physical media edge (resolved from config/CLI).
    pub media_inset: MediaInsetPx,
    /// Printer encode capabilities (feed padding, DPI, cut support).
    pub encode_caps: DeviceCapabilities,
}

/// Options for [`encode_labels`].
#[derive(Debug, Clone, Copy, Default)]
pub struct EncodeLabelsOptions {
    /// File extension for encoded output names (e.g. `png`, `bin`).
    pub extension: &'static str,
    /// Keep per-stage traces for debug / confirm / preview flows.
    pub want_trace: bool,
    /// Emit preprocessing warnings to stderr when a job is heavy.
    pub warn_preprocess: bool,
    /// Sidecar backend is in use (slower than in-process Chromium).
    pub sidecar_backend: bool,
}

/// Result of [`encode_labels`].
pub struct EncodeLabelsResult {
    pub encoded: Vec<(String, Vec<u8>)>,
    pub traces: Vec<crate::debug::LabelTrace>,
    /// Feed extent per label (for throughput stats even when traces are omitted).
    pub feed_dots: Vec<crate::print_stats::LabelFeedDots>,
    pub preprocess_duration: Duration,
}

/// Encode every label in `labels`, optionally warning when preprocessing is
/// expected to be slow.
pub fn encode_labels<B: RenderBackend>(
    backend: &B,
    registry: &Registry,
    labels: &[AuthoringLabel],
    opts: &PipelineOptions,
    encode_opts: EncodeLabelsOptions,
) -> Result<EncodeLabelsResult> {
    use crate::preprocess::{estimate_job, job_input, BATCH_WARN_INTERVAL};
    use crate::print_stats::{feed_dots_for_trace, LabelFeedDots};
    use std::time::Duration;

    let mut encoded = Vec::new();
    let mut traces = Vec::new();
    let mut feed_dots = Vec::new();
    if labels.is_empty() {
        return Ok(EncodeLabelsResult {
            encoded,
            traces,
            feed_dots,
            preprocess_duration: Duration::ZERO,
        });
    }

    let preprocess_input = job_input(
        labels.len(),
        &opts.media,
        opts.rotation,
        opts.supersample,
        encode_opts.sidecar_backend,
    );
    let preprocess_estimate = estimate_job(&preprocess_input);

    if encode_opts.warn_preprocess && preprocess_estimate.exceeds_threshold {
        crate::terminal::warn_preprocess_before(&preprocess_input, &preprocess_estimate)?;
    }

    let batch = labels.len() > 1;
    let mut preprocess_elapsed = Duration::ZERO;
    let mut next_batch_warn = BATCH_WARN_INTERVAL;
    let last_index = labels.len() - 1;

    for (position, label) in labels.iter().enumerate() {
        let mut label_opts = opts.clone();
        label_opts.batch_index = position as u32;
        label_opts.batch_total = labels.len() as u32;
        // Across a multi-label batch, "cut at end" applies only to the last label.
        if opts.cut_mode == CutMode::End && position != last_index {
            label_opts.cut_mode = CutMode::None;
        }
        let started = Instant::now();
        let trace = encode_label_traced(backend, registry, label.index, &label.html, &label_opts)
            .with_context(|| format!("encoding label {}", label.index))?;
        preprocess_elapsed += started.elapsed();

        if encode_opts.warn_preprocess && batch && preprocess_elapsed >= next_batch_warn {
            crate::terminal::warn_preprocess_batch_progress(
                preprocess_elapsed,
                encoded.len() + 1,
                labels.len(),
                &preprocess_input,
                &preprocess_estimate,
            )?;
            next_batch_warn += BATCH_WARN_INTERVAL;
        }

        feed_dots.push(LabelFeedDots(feed_dots_for_trace(&trace, opts.protocol)));
        encoded.push((
            format!("label-{:04}.{}", label.index, encode_opts.extension),
            trace.encoded.clone(),
        ));
        if encode_opts.want_trace {
            traces.push(trace);
        }
    }
    Ok(EncodeLabelsResult {
        encoded,
        traces,
        feed_dots,
        preprocess_duration: preprocess_elapsed,
    })
}

/// Resolve a [`LabelFitSetting`] against the target media.
pub fn resolve_label_fit(setting: LabelFitSetting, media: &Media) -> LabelFit {
    setting.resolve(media.length_dots().is_some())
}

/// Resolve a configured cross-axis alignment string.
pub fn resolve_label_align(s: &str) -> LabelAlign {
    LabelAlign::parse(s).unwrap_or_default()
}

/// Resolve a configured main-axis alignment string.
pub fn resolve_label_valign(s: &str) -> LabelValign {
    LabelValign::parse(s).unwrap_or_default()
}

/// Resolve a configured fit-box scale (clamped to `(0.01, 1.0]`).
pub fn resolve_label_fit_scale(scale: f64) -> f64 {
    scale.clamp(0.01, 1.0)
}

/// Resolve a configured auto-fit text scale (clamped to `(0.01, 5.0]`).
pub fn resolve_font_fit_scale(scale: f64) -> f64 {
    scale.clamp(0.01, 5.0)
}

/// Build a [`CascadingInsetMm`] from the style configuration's padding fields.
pub fn resolve_label_padding(style: &lbl_config::StyleConfig) -> CascadingInsetMm {
    CascadingInsetMm {
        all: style.padding_mm,
        horizontal: style.padding_horizontal_mm,
        vertical: style.padding_vertical_mm,
        top: style.padding_top_mm,
        right: style.padding_right_mm,
        bottom: style.padding_bottom_mm,
        left: style.padding_left_mm,
    }
}

/// Resolved TRBL millimetres from the style padding cascade.
pub fn resolve_label_padding_sides(style: &lbl_config::StyleConfig) -> lbl_core::PaddingSidesMm {
    let s = resolve_label_padding(style).resolve();
    lbl_core::PaddingSidesMm {
        top: s.top,
        right: s.right,
        bottom: s.bottom,
        left: s.left,
    }
}

/// Write explicit per-side padding onto a style config (clears axis/uniform cascade).
pub fn apply_padding_sides_to_style(
    style: &mut lbl_config::StyleConfig,
    sides: lbl_core::PaddingSidesMm,
) {
    style.padding_mm = 0.0;
    style.padding_horizontal_mm = None;
    style.padding_vertical_mm = None;
    style.padding_top_mm = Some(sides.top);
    style.padding_right_mm = Some(sides.right);
    style.padding_bottom_mm = Some(sides.bottom);
    style.padding_left_mm = Some(sides.left);
}

/// Derive tape lead/end from feed-axis label padding; rewrite style padding in place.
///
/// When `override_lead` / `override_end` are set (explicit job fields), that side
/// is not derived from padding.
pub fn apply_virtual_feed_gaps(
    style: &mut lbl_config::StyleConfig,
    caps: &DeviceCapabilities,
    cut_mode: CutMode,
    precut: Option<bool>,
    feed_along_width: bool,
    override_lead: Option<f64>,
    override_end: Option<f64>,
) -> lbl_core::VirtualFeedGaps {
    let gaps = lbl_core::resolve_virtual_feed_gaps(
        caps,
        cut_mode,
        precut,
        resolve_label_padding_sides(style),
        feed_along_width,
        override_lead,
        override_end,
    );
    apply_padding_sides_to_style(style, gaps.padding);
    gaps
}

/// Build a [`MediaInset`] from the style configuration.
pub fn resolve_media_inset(style: &lbl_config::StyleConfig) -> MediaInset {
    MediaInset {
        all_mm: style.media_inset_mm,
        horizontal_mm: style.media_inset_horizontal_mm,
        vertical_mm: style.media_inset_vertical_mm,
        start_mm: style.media_inset_start_mm,
        end_mm: style.media_inset_end_mm,
        cross_start_mm: style.media_inset_cross_start_mm,
        cross_end_mm: style.media_inset_cross_end_mm,
    }
}

/// Inkable width across the head after applying printer clamps.
///
/// Order: physical stock → printer `max_width_mm` → optional laminate
/// `head_printable_height_mm`. Preview pads any leftover side gaps so the
/// gallery shows physical stock, not only the printable raster.
pub fn effective_printable_width_mm(media: &Media, caps: &DeviceCapabilities) -> f64 {
    let mut width = media.width_mm.min(caps.max_width_mm);
    if let Some(printable) = caps.head_printable_height_mm {
        width = width.min(printable);
    }
    width.max(0.0)
}

/// Head-axis dots used for layout and rasterize after printable-band clamps.
pub fn effective_render_head_dots(media: &Media, caps: &DeviceCapabilities) -> u32 {
    Millimeters(effective_printable_width_mm(media, caps))
        .to_dots(media.dpi)
        .0
}

/// CSS-pixel viewport matching the rasterizer's [`RenderRequest`] dimensions.
pub fn render_viewport_px(
    media: &Media,
    supersample: u32,
    rotation: Rotation,
    encode_caps: Option<&DeviceCapabilities>,
) -> ViewportPx {
    let factor = supersample.max(1) as f64;
    let head_dots = encode_caps
        .map(|caps| effective_render_head_dots(media, caps))
        .unwrap_or(media.width_dots().0);
    let feed_dots = media.length_dots().map(|d| d.0);
    let (width_dots, height_dots) = if rotation.swaps_axes() {
        (feed_dots, Some(head_dots))
    } else {
        (Some(head_dots), feed_dots)
    };
    ViewportPx {
        width: width_dots.map(|w| w as f64 * factor),
        height: height_dots.map(|h| h as f64 * factor),
    }
}

/// Result of centering a printable-band raster inside the physical stock width.
#[derive(Debug, Clone)]
pub struct HeadTapePad {
    /// Preview image sized to the physical stock on the head axis.
    pub image: RgbaImage,
    /// Blank pixels before the printable band on the head axis.
    pub pad_before_px: u32,
    /// Blank pixels after the printable band on the head axis.
    pub pad_after_px: u32,
}

/// Axis-aligned bounds of inked pixels in a preview raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentBoundsPx {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Tight bounding box from the first to last inked pixel (inclusive).
///
/// White layout gaps *between* inked elements (e.g. space between text and a
/// QR code) are inside this box by design: it is the span from the leftmost
/// to the rightmost ink, matching a ruler placed across the printed content.
pub fn inked_content_bounds(image: &RgbaImage) -> Option<ContentBoundsPx> {
    let (w, h) = image.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for (x, y, pixel) in image.enumerate_pixels() {
        if !is_inked_preview_pixel(pixel.0) {
            continue;
        }
        found = true;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    if !found {
        return None;
    }
    Some(ContentBoundsPx {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

fn is_inked_preview_pixel([r, g, b, a]: [u8; 4]) -> bool {
    if a < 128 {
        return false;
    }
    let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
    luma <= 128
}

/// Pad a preview raster to the full tape width on the head axis when content
/// was rendered into a narrower printable band.
pub fn pad_preview_head_tape(
    image: RgbaImage,
    media: &Media,
    caps: &DeviceCapabilities,
    head_along_height: bool,
) -> HeadTapePad {
    use image::{imageops, Rgba};

    let tape_dots = media.width_dots().0;
    let printable_dots = effective_render_head_dots(media, caps);
    if printable_dots >= tape_dots {
        return HeadTapePad {
            image,
            pad_before_px: 0,
            pad_after_px: 0,
        };
    }
    let pad_before = (tape_dots - printable_dots) / 2;
    let pad_after = tape_dots - printable_dots - pad_before;
    let label_white = Rgba([255, 255, 255, 255]);

    let image = if head_along_height {
        let (w, h) = image.dimensions();
        if h != printable_dots {
            return HeadTapePad {
                image,
                pad_before_px: 0,
                pad_after_px: 0,
            };
        }
        let mut out = RgbaImage::from_pixel(w, tape_dots, label_white);
        imageops::overlay(&mut out, &image, 0, pad_before as i64);
        out
    } else {
        let (w, h) = image.dimensions();
        if w != printable_dots {
            return HeadTapePad {
                image,
                pad_before_px: 0,
                pad_after_px: 0,
            };
        }
        let mut out = RgbaImage::from_pixel(tape_dots, h, label_white);
        imageops::overlay(&mut out, &image, pad_before as i64, 0);
        out
    };
    HeadTapePad {
        image,
        pad_before_px: pad_before,
        pad_after_px: pad_after,
    }
}

/// CSS-pixel viewport for vector PDF/HTML (reference DPI, no supersampling).
///
/// When `encode_caps` is set, the head axis uses the printable band (same clamp
/// as [`render_viewport_px`]) so Studio HTML preview can pad out to physical
/// stock the same way the raster preview path does.
pub fn render_viewport_vector(
    media: &Media,
    rotation: Rotation,
    encode_caps: Option<&DeviceCapabilities>,
) -> ViewportPx {
    let px_per_mm = CSS_LAYOUT_REFERENCE_DPI / 25.4;
    let head_mm = encode_caps
        .map(|caps| effective_printable_width_mm(media, caps))
        .unwrap_or(media.width_mm);
    let feed_mm = match media.length {
        lbl_core::media::MediaLength::Fixed(len) => Some(len),
        lbl_core::media::MediaLength::Continuous => None,
    };
    let (width_mm, height_mm) = if rotation.swaps_axes() {
        (feed_mm, Some(head_mm))
    } else {
        (Some(head_mm), feed_mm)
    };
    ViewportPx {
        width: width_mm.map(|w| w * px_per_mm),
        height: height_mm.map(|h| h * px_per_mm),
    }
}

/// Stock framing for HTML preview: printable content centered in physical tape.
///
/// Mirrors [`pad_preview_head_tape`] + [`pad_preview_encode_feed`] without a
/// raster — sizes are CSS pixels at `layout_dpi`.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewStockFrame {
    pub content_width_px: f64,
    pub content_height_px: f64,
    pub width_px: f64,
    pub height_px: f64,
    pub head_pad_before_px: u32,
    pub head_pad_after_px: u32,
    pub head_along_height: bool,
    pub lead_feed_px: u32,
    pub feed_end_margin_px: u32,
    pub trail_feed_px: u32,
    pub content_feed_end_px: u32,
    pub precut: bool,
    /// User-facing virtual start gap (lead + feed-start content inset), layout px.
    pub virtual_feed_start_px: u32,
    /// User-facing virtual end gap (end margin + feed-end content inset), layout px.
    pub virtual_feed_end_px: u32,
}

fn mm_to_layout_px(mm: f64, layout_dpi: f64) -> f64 {
    (mm / 25.4) * layout_dpi
}

/// Optional job feed/cut fields for preview stock / raster padding.
///
/// Defaults match caps-only preview (`CutMode::None`, unset padding) so interactive
/// gallery still works without print settings. When Studio passes print prefs,
/// lead/end/precut follow the same [`resolve_feed_plan`] rules as encode.
#[derive(Debug, Clone, Default)]
pub struct PreviewFeedOverrides {
    pub cut_mode: CutMode,
    pub feed_lead_mm: Option<f64>,
    pub feed_end_mm: Option<f64>,
    pub precut: Option<bool>,
    /// Virtual start gap \(G\) before first ink (mm); when set, exposed as marker px.
    pub virtual_feed_start_mm: Option<f64>,
    /// Virtual end gap after last ink (mm).
    pub virtual_feed_end_mm: Option<f64>,
}

/// Resolve a feed plan for preview; on policy errors keep lead/end without pre-cut.
pub fn preview_feed_plan(
    caps: &DeviceCapabilities,
    feed: &PreviewFeedOverrides,
) -> lbl_core::FeedPlan {
    match preview_resolve_feed_plan(
        caps,
        feed.cut_mode,
        feed.feed_lead_mm,
        feed.feed_end_mm,
        feed.precut,
    ) {
        Ok(plan) => plan,
        Err(_) => preview_resolve_feed_plan(
            caps,
            CutMode::None,
            feed.feed_lead_mm,
            feed.feed_end_mm,
            Some(false),
        )
        .unwrap_or_default(),
    }
}

/// Compute head + feed padding that expands printable HTML to physical stock.
///
/// A non-positive content size on the feed axis means continuous / content-sized
/// media: that axis stays `0` in the returned frame (Studio sizes it from the
/// viewport / `--lbl-feed-px`). Head-to-cutter lead/end margins from
/// [`DeviceCapabilities`] are still applied as stock padding so the preview
/// matches real cut gaps (same DX rule as [`pad_preview_encode_feed`]).
pub fn preview_stock_frame(
    content_width_px: f64,
    content_height_px: f64,
    media: &Media,
    caps: &DeviceCapabilities,
    head_along_height: bool,
    layout_dpi: f64,
    feed: &PreviewFeedOverrides,
) -> PreviewStockFrame {
    let tape_head_px = mm_to_layout_px(media.width_mm, layout_dpi).round().max(0.0);
    let content_head = if head_along_height {
        content_height_px
    } else {
        content_width_px
    };
    let (head_before, head_after) = if tape_head_px > content_head + 0.5 {
        let gap = (tape_head_px - content_head).round().max(0.0) as u32;
        let before = gap / 2;
        (before, gap - before)
    } else {
        (0, 0)
    };

    let feed_along_width = head_along_height;
    let content_feed = if feed_along_width {
        content_width_px.round().max(0.0) as u32
    } else {
        content_height_px.round().max(0.0) as u32
    };
    let plan = preview_feed_plan(caps, feed);
    let (lead_feed, end_margin, dx) = preview_feed_margins_from_plan(&plan, layout_dpi);
    let trail_feed = if plan.precut || dx > 0 { dx } else { 0 };
    let precut = plan.precut;
    let virtual_start = feed
        .virtual_feed_start_mm
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|mm| feed_mm_px(mm, layout_dpi))
        .unwrap_or(lead_feed);
    let virtual_end = feed
        .virtual_feed_end_mm
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|mm| feed_mm_px(mm, layout_dpi))
        .unwrap_or(end_margin);
    // Unknown continuous length: leave feed axis open (0). Known length: bake
    // lead+end into the stock box. Margins themselves always come from caps.
    let content_feed_end = if content_feed > 0 {
        lead_feed + content_feed
    } else {
        0
    };

    let (width_px, height_px) = if head_along_height {
        let feed = if content_feed == 0 {
            0.0
        } else {
            content_width_px + f64::from(lead_feed + end_margin)
        };
        (
            feed,
            content_height_px + f64::from(head_before + head_after),
        )
    } else {
        let feed = if content_feed == 0 {
            0.0
        } else {
            content_height_px + f64::from(lead_feed + end_margin)
        };
        (content_width_px + f64::from(head_before + head_after), feed)
    };

    PreviewStockFrame {
        content_width_px,
        content_height_px,
        width_px,
        height_px,
        head_pad_before_px: head_before,
        head_pad_after_px: head_after,
        head_along_height,
        lead_feed_px: lead_feed,
        feed_end_margin_px: end_margin,
        trail_feed_px: trail_feed,
        content_feed_end_px: content_feed_end,
        precut,
        virtual_feed_start_px: virtual_start,
        virtual_feed_end_px: virtual_end,
    }
}

/// Wrap transpiled label HTML in a physical-stock frame (white head/feed pads).
///
/// When both axes have known content sizes, the outer `.lbl-stock` matches
/// [`PreviewStockFrame::width_px`] × [`PreviewStockFrame::height_px`] with the
/// printable band absolutely positioned inside. When a continuous feed axis is
/// still `0`, padding + `100%` keeps that axis open so Studio can size it.
pub fn frame_html_preview_stock(html: &str, frame: &PreviewStockFrame) -> String {
    if frame.head_pad_before_px == 0
        && frame.head_pad_after_px == 0
        && frame.lead_feed_px == 0
        && frame.feed_end_margin_px == 0
    {
        return html.to_string();
    }

    let (pad_left, pad_top) = if frame.head_along_height {
        (frame.lead_feed_px, frame.head_pad_before_px)
    } else {
        (frame.head_pad_before_px, frame.lead_feed_px)
    };
    let (pad_right, pad_bottom) = if frame.head_along_height {
        (frame.feed_end_margin_px, frame.head_pad_after_px)
    } else {
        (frame.head_pad_after_px, frame.feed_end_margin_px)
    };

    let open_width = frame.content_width_px <= 0.0;
    let open_height = frame.content_height_px <= 0.0;
    let stock_css = if open_width || open_height {
        // Continuous feed axis: size to content (max-content), not 100% of a
        // placeholder iframe width — otherwise head-fitted text re-wraps and
        // overflows the tape height.
        let width_rule = if open_width {
            "width:max-content;max-width:none".to_string()
        } else {
            format!("width:{:.2}px", frame.width_px)
        };
        let height_rule = if open_height {
            "height:max-content;max-height:none".to_string()
        } else {
            format!("height:{:.2}px", frame.height_px)
        };
        format!(
            ".lbl-stock{{position:relative;{width_rule};{height_rule};box-sizing:border-box;\
             padding:{pad_top}px {pad_right}px {pad_bottom}px {pad_left}px;\
             background:#fff;overflow:visible;align-self:flex-start;flex:0 0 auto}}\n\
             .lbl-stock-print{{box-sizing:border-box;display:flex;flex-direction:column;\
             justify-content:center;{print_width};{print_height}}}\n\
             html,body{{margin:0;{width_rule};{height_rule};align-items:flex-start}}\n\
             .lbl-label{{width:max-content;max-width:none;flex:0 0 auto}}\n",
            width_rule = width_rule,
            height_rule = height_rule,
            print_width = if open_width {
                "width:max-content;max-width:none"
            } else {
                "width:100%"
            },
            print_height = if open_height {
                "height:max-content;max-height:none"
            } else {
                "height:100%"
            },
            pad_top = pad_top,
            pad_right = pad_right,
            pad_bottom = pad_bottom,
            pad_left = pad_left,
        )
    } else {
        format!(
            ".lbl-stock{{position:relative;width:{w:.2}px;height:{h:.2}px;background:#fff;overflow:hidden;box-sizing:border-box}}\n\
             .lbl-stock-print{{position:absolute;left:{left}px;top:{top}px;width:{cw:.2}px;height:{ch:.2}px;box-sizing:border-box}}\n\
             html,body{{margin:0;width:{w:.2}px;height:{h:.2}px}}\n",
            w = frame.width_px,
            h = frame.height_px,
            left = pad_left,
            top = pad_top,
            cw = frame.content_width_px,
            ch = frame.content_height_px,
        )
    };

    let with_css = if let Some(idx) = html.rfind("</style>") {
        let mut out = String::with_capacity(html.len() + stock_css.len());
        out.push_str(&html[..idx]);
        out.push_str(&stock_css);
        out.push_str(&html[idx..]);
        out
    } else if let Some(idx) = html.find("</head>") {
        let mut out = String::with_capacity(html.len() + stock_css.len() + 16);
        out.push_str(&html[..idx]);
        out.push_str("<style>\n");
        out.push_str(&stock_css);
        out.push_str("</style>\n");
        out.push_str(&html[idx..]);
        out
    } else {
        format!("<style>\n{stock_css}</style>\n{html}")
    };

    if let Some(body_open) = with_css.find("<body>") {
        let after = body_open + "<body>".len();
        if let Some(body_rel) = with_css[after..].rfind("</body>") {
            let body_end = after + body_rel;
            let inner = &with_css[after..body_end];
            let mut out = String::with_capacity(with_css.len() + 64);
            out.push_str(&with_css[..after]);
            out.push_str("\n<div class=\"lbl-stock\"><div class=\"lbl-stock-print\">");
            out.push_str(inner);
            out.push_str("</div></div>\n");
            out.push_str(&with_css[body_end..]);
            return out;
        }
    }
    format!("<div class=\"lbl-stock\"><div class=\"lbl-stock-print\">{with_css}</div></div>")
}

/// Physical page size for vector PDF export in the label reading frame.
pub fn page_size_mm(media: &Media, rotation: Rotation) -> PageSizeMm {
    let (width_mm, height_mm) = match media.length {
        lbl_core::media::MediaLength::Fixed(len) => {
            if rotation.swaps_axes() {
                (len, Some(media.width_mm))
            } else {
                (media.width_mm, Some(len))
            }
        }
        lbl_core::media::MediaLength::Continuous => (media.width_mm, None),
    };
    PageSizeMm {
        width_mm,
        height_mm,
    }
}

/// Browser-ready HTML from the vector transpile path (no Chromium render).
#[derive(Debug, Clone)]
pub struct TranspiledLabelHtml {
    pub html: String,
    pub width_px: f64,
    pub height_px: f64,
    pub width_mm: f64,
    pub height_mm: Option<f64>,
    pub corner_radius_px: f64,
}

/// Transpile authoring HTML for browser preview or OS print.
///
/// Uses CSS-reference vector geometry for vector export/preview, or
/// device-DPI × supersample geometry when [`PipelineOptions::virtual_export_mode`]
/// is raster (print-resolution capture).
pub fn transpile_label_html(
    authoring_html: &str,
    opts: &PipelineOptions,
) -> Result<TranspiledLabelHtml> {
    let viewport = if opts.virtual_export_mode == VirtualExportMode::Raster {
        render_viewport_px(
            &opts.media,
            opts.supersample,
            opts.rotation,
            Some(&opts.encode_caps),
        )
    } else {
        render_viewport_vector(&opts.media, opts.rotation, Some(&opts.encode_caps))
    };
    let page_size = page_size_mm(&opts.media, opts.rotation);
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: opts.assets_base.clone(),
            font_delivery: opts.font_delivery.clone(),
            index: None,
            count: None,
            style: opts.style.clone(),
            label_fit: opts.label_fit,
            viewport: Some(viewport.clone()),
            label_align: opts.label_align,
            label_valign: opts.label_valign,
            label_fit_scale: opts.label_fit_scale,
            font_fit_scale: opts.font_fit_scale,
            media_inset: opts.media_inset,
            page_size: Some(page_size),
        },
    );

    let corner_radius_px = match opts.media.length {
        lbl_core::media::MediaLength::Fixed(_) => opts.style.corner_radius_px,
        lbl_core::media::MediaLength::Continuous => 0.0,
    };

    Ok(TranspiledLabelHtml {
        html: transpiled,
        width_px: viewport.width.unwrap_or(0.0),
        height_px: viewport.height.unwrap_or(0.0),
        width_mm: page_size.width_mm,
        height_mm: page_size.height_mm,
        corner_radius_px,
    })
}

/// Raster output from the print transpile + render path (before dither/encode).
#[derive(Debug, Clone)]
pub struct LabelRaster {
    /// RGBA bitmap at print resolution, after any head rotation.
    pub image: RgbaImage,
    /// Browser-ready HTML used for rendering.
    pub transpiled_html: String,
}

fn feed_margin_px(mm: Option<f64>, dpi: f64) -> u32 {
    mm.filter(|value| *value > f64::EPSILON)
        .map(|value| ((value / 25.4) * dpi).round().max(0.0) as u32)
        .unwrap_or(0)
}

fn feed_mm_px(mm: f64, dpi: f64) -> u32 {
    if mm > f64::EPSILON {
        feed_margin_px(Some(mm), dpi)
    } else {
        0
    }
}

/// Build a feed plan for preview from caps + optional job padding/cut fields.
pub fn preview_resolve_feed_plan(
    caps: &DeviceCapabilities,
    cut_mode: CutMode,
    feed_lead_mm: Option<f64>,
    feed_end_mm: Option<f64>,
    precut: Option<bool>,
) -> Result<lbl_core::FeedPlan, lbl_core::FeedPlanError> {
    let mut job = JobSpec::new(Media::continuous(caps.max_width_mm.max(1.0), caps.dpi));
    job.cut_mode = cut_mode;
    job.feed_lead_mm = feed_lead_mm;
    job.feed_end_mm = feed_end_mm;
    job.precut = precut;
    lbl_core::resolve_feed_plan(caps, &job)
}

/// Lead / end / cutter-gap along feed for preview from a resolved [`FeedPlan`].
///
/// When `precut`, trail is scrap metadata only (not painted onto the sticker).
/// When trail-only legacy caps are used via a plan with lead = \(D_x\) and end = 0,
/// only the lead is drawn (unset end → 0).
fn preview_feed_margins_from_plan(plan: &lbl_core::FeedPlan, dpi: f64) -> (u32, u32, u32) {
    let lead = feed_mm_px(plan.lead_mm, dpi);
    let end = feed_mm_px(plan.end_mm, dpi);
    let dx = feed_mm_px(plan.cutter_gap_mm, dpi);
    (lead, end, dx)
}

/// Feed-axis padding applied to a preview raster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFeedPad {
    pub image: RgbaImage,
    /// Feed-axis position where rendered content ends (start of right padding).
    pub content_feed_end_px: u32,
    /// Right padding on the sticker face before the cutter gap.
    pub feed_end_margin_px: u32,
    /// Cutter-gap / ejected scrap width along the feed (metadata; not drawn on sticker).
    pub trail_feed_px: u32,
    /// Blank tape before content on the kept label.
    pub lead_feed_px: u32,
    /// Whether pre-cut ejects [`Self::trail_feed_px`] as scrap before the label.
    pub precut: bool,
}

/// Extend a preview raster with encode feed margins for tape printers.
pub fn pad_preview_encode_feed(
    image: RgbaImage,
    caps: &DeviceCapabilities,
    feed_along_width: bool,
) -> PreviewFeedPad {
    // Caps-only preview: no cut → allow any lead (including catalog small lead).
    let plan = preview_resolve_feed_plan(caps, CutMode::None, None, None, None).unwrap_or_default();
    pad_preview_encode_feed_plan(image, &plan, caps.dpi.0, feed_along_width)
}

/// Like [`pad_preview_encode_feed`] with an explicit [`lbl_core::FeedPlan`].
pub fn pad_preview_encode_feed_plan(
    image: RgbaImage,
    plan: &lbl_core::FeedPlan,
    dpi: f64,
    feed_along_width: bool,
) -> PreviewFeedPad {
    use image::{imageops, Rgba};

    let (preview_lead, end_margin, cutter_gap) = preview_feed_margins_from_plan(plan, dpi);

    let content_feed = if feed_along_width {
        image.width()
    } else {
        image.height()
    };

    if preview_lead == 0 && end_margin == 0 {
        return PreviewFeedPad {
            image,
            content_feed_end_px: content_feed,
            feed_end_margin_px: 0,
            trail_feed_px: if plan.precut { cutter_gap } else { 0 },
            lead_feed_px: 0,
            precut: plan.precut,
        };
    }

    let label_white = Rgba([255, 255, 255, 255]);
    let content_end = preview_lead + content_feed;
    let trail_feed_px = if plan.precut || cutter_gap > 0 {
        cutter_gap
    } else {
        0
    };

    if feed_along_width {
        let (w, h) = image.dimensions();
        let mut out = RgbaImage::from_pixel(w + preview_lead + end_margin, h, label_white);
        imageops::overlay(&mut out, &image, preview_lead as i64, 0);
        PreviewFeedPad {
            image: out,
            content_feed_end_px: content_end,
            feed_end_margin_px: end_margin,
            trail_feed_px,
            lead_feed_px: preview_lead,
            precut: plan.precut,
        }
    } else {
        let (w, h) = image.dimensions();
        let mut out = RgbaImage::from_pixel(w, h + preview_lead + end_margin, label_white);
        imageops::overlay(&mut out, &image, 0, preview_lead as i64);
        PreviewFeedPad {
            image: out,
            content_feed_end_px: content_end,
            feed_end_margin_px: end_margin,
            trail_feed_px,
            lead_feed_px: preview_lead,
            precut: plan.precut,
        }
    }
}

struct PrintRenderStage {
    transpiled: String,
    rendered: RgbaImage,
    applied_rotation: Rotation,
    req_width: Option<u32>,
    req_height: Option<u32>,
}

/// Transpile for print, rasterize with Chromium, optionally mirror the reading
/// frame, then apply the same head rotation the encoder would use.
fn render_label_print<B: RenderBackend>(
    backend: &B,
    authoring_html: &str,
    opts: &PipelineOptions,
) -> Result<PrintRenderStage> {
    let viewport = render_viewport_px(
        &opts.media,
        opts.supersample,
        opts.rotation,
        Some(&opts.encode_caps),
    );
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: opts.assets_base.clone(),
            font_delivery: opts.font_delivery.clone(),
            index: None,
            count: None,
            style: opts.style.clone(),
            label_fit: opts.label_fit,
            viewport: Some(viewport),
            label_align: opts.label_align,
            label_valign: opts.label_valign,
            label_fit_scale: opts.label_fit_scale,
            font_fit_scale: opts.font_fit_scale,
            media_inset: opts.media_inset,
            ..Default::default()
        },
    );

    let head_dots = effective_render_head_dots(&opts.media, &opts.encode_caps);
    let feed_dots = opts.media.length_dots().map(|d| d.0);
    let (req_width, req_height) = if opts.rotation.swaps_axes() {
        (feed_dots, Some(head_dots))
    } else {
        (Some(head_dots), feed_dots)
    };
    let req = RenderRequest {
        width_dots: req_width,
        height_dots: req_height,
        supersample: opts.supersample,
    };
    let rendered = render_two_pass(backend, &transpiled, &req).context("rendering")?;

    // Mirror the reading frame first so Virtual preview and hardware encode agree.
    let rendered = if opts.mirror {
        apply_mirror_horizontal(rendered)
    } else {
        rendered
    };

    let applied_rotation = if opts.protocol.targets_print_head() {
        opts.head_rotation
    } else {
        Rotation::None
    };
    let rendered = apply_rotation(rendered, applied_rotation);

    Ok(PrintRenderStage {
        transpiled,
        rendered,
        applied_rotation,
        req_width,
        req_height,
    })
}

/// Rasterize one label through the same print pipeline used before dither/encode.
pub fn render_label_raster<B: RenderBackend>(
    backend: &B,
    authoring_html: &str,
    opts: &PipelineOptions,
) -> Result<LabelRaster> {
    let stage = render_label_print(backend, authoring_html, opts)?;
    Ok(LabelRaster {
        image: stage.rendered,
        transpiled_html: stage.transpiled,
    })
}

/// Dither + protocol-encode result from a pre-rendered RGBA image.
#[derive(Debug)]
pub struct EncodeFromRgbaResult {
    /// 1-bit primary plane after dithering (or black plane for two-color).
    pub dithered: MonoBitmap,
    /// Final protocol (or virtual image) bytes.
    pub encoded: Vec<u8>,
    /// Driver name that produced [`Self::encoded`].
    pub driver_name: String,
}

/// Dither and encode a pre-rendered RGBA label (no transpile/render).
///
/// Used by the Chromium print path after rasterization, and by callers that
/// already hold a print-resolution image.
pub fn encode_label_from_rgba(
    registry: &Registry,
    rendered: &RgbaImage,
    opts: &PipelineOptions,
) -> Result<EncodeFromRgbaResult> {
    let two_color = opts.media.two_color;
    let color_png =
        if !two_color && opts.encode_caps.supports_color && opts.protocol == Protocol::EscLabel {
            Some(encode_rgba_png(rendered).context("encoding color PNG")?)
        } else {
            None
        };
    let (dithered, secondary_plane) = if two_color {
        let (primary, secondary) = split_black_red(rendered, 80);
        (primary, Some(secondary))
    } else {
        (dither(rendered, opts.dither), None)
    };

    let mut job = JobSpec::new(opts.media.clone());
    job.cut_mode = opts.cut_mode;
    job.copies = opts.copies;
    job.batch_index = opts.batch_index;
    job.batch_total = opts.batch_total;
    job.density = opts.density;
    job.feed_lead_mm = opts.feed_lead_mm;
    job.feed_end_mm = opts.feed_end_mm;
    job.precut = opts.precut;
    job.driver = opts.driver.clone();
    let caps = opts.encode_caps.clone();
    let driver = registry
        .get(opts.protocol)
        .ok_or_else(|| anyhow!("no driver for protocol {:?}", opts.protocol))?;
    let feed_plan = lbl_core::resolve_feed_plan(&caps, &job).map_err(|e| anyhow!(e))?;
    let mut ctx = EncodeContext::with_feed_plan(&job, &caps, feed_plan);
    if let Some(secondary) = &secondary_plane {
        ctx = ctx.with_secondary(secondary);
    }
    if let Some(png) = &color_png {
        ctx = ctx.with_color_png(png);
    }
    let encoded = driver.encode(&dithered, &ctx).context("encoding")?;
    Ok(EncodeFromRgbaResult {
        dithered,
        encoded,
        driver_name: driver.name().to_string(),
    })
}

/// Run one authoring-HTML label through transpile -> render -> dither -> encode,
/// producing printer-native bytes.
pub fn encode_label<B: RenderBackend>(
    backend: &B,
    registry: &Registry,
    authoring_html: &str,
    opts: &PipelineOptions,
) -> Result<Vec<u8>> {
    let trace = encode_label_traced(backend, registry, 0, authoring_html, opts)?;
    Ok(trace.encoded)
}

/// Like [`encode_label`], but captures every stage's input and output into a
/// [`LabelTrace`] for the debug report. The final protocol bytes are available
/// as [`LabelTrace::encoded`].
pub fn encode_label_traced<B: RenderBackend>(
    backend: &B,
    registry: &Registry,
    index: usize,
    authoring_html: &str,
    opts: &PipelineOptions,
) -> Result<crate::debug::LabelTrace> {
    if opts.protocol == Protocol::Virtual && opts.virtual_export_mode == VirtualExportMode::Vector {
        return encode_label_vector_traced(backend, index, authoring_html, opts);
    }

    let PrintRenderStage {
        transpiled,
        rendered,
        applied_rotation,
        req_width,
        req_height,
    } = render_label_print(backend, authoring_html, opts)?;

    let EncodeFromRgbaResult {
        dithered,
        encoded,
        driver_name,
    } = encode_label_from_rgba(registry, &rendered, opts)?;

    Ok(crate::debug::LabelTrace {
        index,
        authoring_html: authoring_html.to_string(),
        transpiled_html: transpiled,
        assets_base: opts.assets_base.clone(),
        width_dots: req_width,
        height_dots: req_height,
        rotation: applied_rotation,
        supersample: opts.supersample,
        rendered,
        dither: opts.dither,
        dithered,
        protocol: opts.protocol,
        driver_name,
        media_type: opts.media_type,
        encoded,
    })
}

fn encode_label_vector_traced<B: RenderBackend>(
    backend: &B,
    index: usize,
    authoring_html: &str,
    opts: &PipelineOptions,
) -> Result<crate::debug::LabelTrace> {
    let applied_rotation = Rotation::None;
    let viewport = render_viewport_vector(&opts.media, opts.rotation, Some(&opts.encode_caps));
    let page_size = page_size_mm(&opts.media, opts.rotation);
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: opts.assets_base.clone(),
            font_delivery: opts.font_delivery.clone(),
            index: None,
            count: None,
            style: opts.style.clone(),
            label_fit: opts.label_fit,
            viewport: Some(viewport),
            label_align: opts.label_align,
            label_valign: opts.label_valign,
            label_fit_scale: opts.label_fit_scale,
            font_fit_scale: opts.font_fit_scale,
            media_inset: opts.media_inset,
            page_size: Some(page_size),
        },
    );

    let head_dots = effective_render_head_dots(&opts.media, &opts.encode_caps);
    let feed_dots = opts.media.length_dots().map(|d| d.0);
    let (req_width, req_height) = if opts.rotation.swaps_axes() {
        (feed_dots, Some(head_dots))
    } else {
        (Some(head_dots), feed_dots)
    };

    let pdf_req = PdfExportRequest {
        width_mm: page_size.width_mm,
        height_mm: page_size.height_mm,
    };
    let encoded = backend
        .export_pdf(&transpiled, &pdf_req)
        .map_err(|e| anyhow!(e.to_string()))
        .context("exporting vector PDF")?;

    Ok(crate::debug::LabelTrace {
        index,
        authoring_html: authoring_html.to_string(),
        transpiled_html: transpiled,
        assets_base: opts.assets_base.clone(),
        width_dots: req_width,
        height_dots: req_height,
        rotation: applied_rotation,
        supersample: 1,
        rendered: mono_preview_rgba(&MonoBitmap::new(1, 1)),
        dither: opts.dither,
        dithered: MonoBitmap::new(1, 1),
        protocol: opts.protocol,
        driver_name: "vector-pdf".to_string(),
        media_type: Some(MediaType::Pdf),
        encoded,
    })
}

/// Encode a calibration sample pattern straight to protocol bytes (no render,
/// dither, rotation, or rescaling). `head_dots` is the pattern height across
/// the print head (classic LabelManager `--sample-pattern` layout).
pub fn encode_sample_pattern(
    registry: &Registry,
    head_dots: u32,
    opts: &PipelineOptions,
) -> Result<Vec<u8>> {
    let trace = encode_sample_pattern_traced(registry, 0, head_dots, opts)?;
    Ok(trace.encoded)
}

/// Like [`encode_sample_pattern`], but captures a [`LabelTrace`] for previews
/// and debug output.
pub fn encode_sample_pattern_traced(
    registry: &Registry,
    index: usize,
    head_dots: u32,
    opts: &PipelineOptions,
) -> Result<crate::debug::LabelTrace> {
    if head_dots == 0 {
        bail!("sample pattern height must be at least 1 dot");
    }
    let bitmap = sample_pattern_for_media(head_dots, &opts.media, opts.protocol);

    let mut job = JobSpec::new(opts.media.clone());
    job.cut_mode = opts.cut_mode;
    job.copies = opts.copies;
    job.batch_index = opts.batch_index;
    job.batch_total = opts.batch_total;
    job.density = opts.density;
    job.feed_lead_mm = opts.feed_lead_mm;
    job.feed_end_mm = opts.feed_end_mm;
    job.precut = opts.precut;
    job.driver = opts.driver.clone();
    let caps = opts.encode_caps.clone();
    let driver = registry
        .get(opts.protocol)
        .ok_or_else(|| anyhow!("no driver for protocol {:?}", opts.protocol))?;
    let feed_plan = lbl_core::resolve_feed_plan(&caps, &job).map_err(|e| anyhow!(e))?;
    let ctx = EncodeContext::with_feed_plan(&job, &caps, feed_plan);
    let encoded = driver.encode(&bitmap, &ctx).context("encoding")?;

    Ok(crate::debug::LabelTrace {
        index,
        authoring_html: format!("(sample pattern, head={head_dots} dots)"),
        transpiled_html: String::new(),
        assets_base: opts.assets_base.clone(),
        width_dots: Some(bitmap.width),
        height_dots: Some(bitmap.height),
        rotation: Rotation::None,
        supersample: 1,
        rendered: mono_preview_rgba(&bitmap),
        dither: Algorithm::Threshold(128),
        dithered: bitmap,
        protocol: opts.protocol,
        driver_name: driver.name().to_string(),
        media_type: opts.media_type,
        encoded,
    })
}

fn mono_preview_rgba(bmp: &lbl_core::bitmap::MonoBitmap) -> image::RgbaImage {
    use image::{Rgba, RgbaImage};
    let mut img = RgbaImage::new(bmp.width, bmp.height);
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            let v = if bmp.get(x, y) { 0 } else { 255 };
            img.put_pixel(x, y, Rgba([v, v, v, 255]));
        }
    }
    img
}

fn encode_rgba_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .context("PNG encode")?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::Orientation;

    /// A backend that returns a solid raster of exactly the requested size, so
    /// tests can exercise orientation/rotation without a browser.
    struct SolidBackend;
    impl RenderBackend for SolidBackend {
        fn rasterize(
            &self,
            _html: &str,
            width: Option<u32>,
            height: Option<u32>,
        ) -> lbl_render::Result<image::RgbaImage> {
            let w = width.or(height).unwrap_or(1).max(1);
            let h = height.or(width).unwrap_or(1).max(1);
            Ok(image::RgbaImage::from_pixel(
                w,
                h,
                image::Rgba([0, 0, 0, 255]),
            ))
        }
    }

    /// Landscape options for a 12×40 mm label (head 12 mm, feed 40 mm).
    fn landscape_opts(protocol: Protocol) -> PipelineOptions {
        let rotation = Rotation::Cw90;
        let head_rotation = Rotation::for_head_with_media(
            Orientation::Landscape,
            &Media::fixed(12.0, 40.0, Dpi(203.0)),
            0,
            0,
            protocol,
        );
        PipelineOptions {
            protocol,
            media: Media::fixed(12.0, 40.0, Dpi(203.0)),
            supports_cut: false,
            cut_mode: CutMode::None,
            copies: 1,
            batch_index: 0,
            batch_total: 1,
            density: None,
            feed_lead_mm: None,
            feed_end_mm: None,
            precut: None,
            driver: lbl_core::DriverOptions::default(),
            dither: Algorithm::Threshold(128),
            rotation,
            head_rotation,
            mirror: false,
            supersample: 1,
            assets_base: AssetsBase::Cdn,
            font_delivery: FontDelivery::default(),
            style: LabelStyle::default(),
            media_type: None,
            virtual_export_mode: VirtualExportMode::Raster,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            font_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
            encode_caps: DeviceCapabilities::default(),
        }
    }

    #[test]
    fn inked_content_bounds_excludes_blank_margins() {
        use image::Rgba;

        let mut image = RgbaImage::from_pixel(20, 10, Rgba([255, 255, 255, 255]));
        // Inked block from (3,2) through (14,7) inclusive → 12×6 px.
        for y in 2..=7 {
            for x in 3..=14 {
                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let bounds = inked_content_bounds(&image).expect("inked bounds");
        assert_eq!(
            bounds,
            ContentBoundsPx {
                x: 3,
                y: 2,
                width: 12,
                height: 6,
            }
        );
    }

    #[test]
    fn inked_content_bounds_includes_gap_between_ink_runs() {
        use image::Rgba;

        let mut image = RgbaImage::from_pixel(30, 8, Rgba([255, 255, 255, 255]));
        // Two inked blocks with a white gap between them.
        for y in 1..=6 {
            for x in 2..=8 {
                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
            for x in 16..=24 {
                image.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let bounds = inked_content_bounds(&image).expect("inked bounds");
        assert_eq!(bounds.x, 2);
        assert_eq!(bounds.width, 23); // 2..=24 inclusive
    }

    #[test]
    fn preview_pads_physical_stock_wider_than_print_head() {
        use image::Rgba;
        use lbl_core::units::Dpi;

        let media = Media::fixed(15.0, 30.0, Dpi(203.0));
        let caps = DeviceCapabilities {
            dpi: Dpi(203.0),
            max_width_mm: 12.0,
            ..Default::default()
        };
        let printable = effective_render_head_dots(&media, &caps);
        let tape = media.width_dots().0;
        assert!(
            printable < tape,
            "12 mm head should be narrower than 15 mm stock"
        );

        let image = RgbaImage::from_pixel(printable, 40, Rgba([0, 0, 0, 255]));
        let padded = pad_preview_head_tape(image, &media, &caps, false);
        assert_eq!(padded.image.width(), tape);
        assert_eq!(padded.image.height(), 40);
        assert!(padded.pad_before_px > 0);
        assert!(padded.pad_after_px > 0);
        assert_eq!(padded.pad_before_px + printable + padded.pad_after_px, tape);
    }

    #[test]
    fn printable_width_clamps_to_max_and_laminate_band() {
        use lbl_core::units::Dpi;

        let media = Media::fixed(12.0, 40.0, Dpi(180.0));
        let caps = DeviceCapabilities {
            dpi: Dpi(180.0),
            max_width_mm: 12.0,
            head_printable_height_mm: Some(8.2),
            ..Default::default()
        };
        assert!((effective_printable_width_mm(&media, &caps) - 8.2).abs() < 1e-9);
    }

    #[test]
    fn preview_stock_frame_pads_head_axis_at_css_dpi() {
        use lbl_core::units::Dpi;

        let media = Media::fixed(15.0, 30.0, Dpi(203.0));
        let caps = DeviceCapabilities {
            dpi: Dpi(203.0),
            max_width_mm: 12.0,
            ..Default::default()
        };
        let content_w = mm_to_layout_px(12.0, VECTOR_CSS_DPI);
        let content_h = mm_to_layout_px(30.0, VECTOR_CSS_DPI);
        let frame = preview_stock_frame(
            content_w,
            content_h,
            &media,
            &caps,
            false,
            VECTOR_CSS_DPI,
            &PreviewFeedOverrides::default(),
        );
        assert!(frame.head_pad_before_px > 0);
        assert!(frame.head_pad_after_px > 0);
        assert!(
            (frame.width_px - mm_to_layout_px(15.0, VECTOR_CSS_DPI)).abs() < 1.0,
            "stock width should match 15 mm tape, got {}",
            frame.width_px
        );
        assert_eq!(frame.content_height_px, content_h);
        let framed = frame_html_preview_stock(
            "<!doctype html><html><head><style>body{}</style></head><body><div class=\"lbl-label\">x</div></body></html>",
            &frame,
        );
        assert!(framed.contains("lbl-stock"));
        assert!(framed.contains("lbl-stock-print"));
        assert!(framed.contains(&format!("left:{}px", frame.head_pad_before_px)));
    }

    #[test]
    fn preview_stock_frame_keeps_continuous_feed_axis_open() {
        use lbl_core::units::Dpi;

        // LabelManager-style: continuous D1 with feed trail + laminate band.
        let media = Media::continuous(12.0, Dpi(180.0));
        let caps = DeviceCapabilities {
            dpi: Dpi(180.0),
            max_width_mm: 12.0,
            feed_trail_mm: Some(8.1),
            head_printable_height_mm: Some(8.2),
            ..Default::default()
        };
        let content_h = mm_to_layout_px(8.2, VECTOR_CSS_DPI);
        let frame = preview_stock_frame(
            0.0,
            content_h,
            &media,
            &caps,
            true,
            VECTOR_CSS_DPI,
            &PreviewFeedOverrides::default(),
        );
        let dx = feed_margin_px(caps.feed_trail_mm, VECTOR_CSS_DPI);
        assert_eq!(
            frame.width_px, 0.0,
            "continuous feed must stay open (not margins-only)"
        );
        assert!(
            frame.height_px > content_h,
            "12 mm stock should pad the 8.2 mm printable band"
        );
        assert_eq!(
            frame.lead_feed_px, dx,
            "open continuous feed still shows head-to-cutter lead"
        );
        assert_eq!(
            frame.feed_end_margin_px, 0,
            "unset end padding is 0 (lead absorbs Dx when precut is off)"
        );
        assert_eq!(frame.trail_feed_px, dx);
        assert_eq!(
            frame.content_feed_end_px, 0,
            "content end is unknown until feed length is measured"
        );
        assert!(frame.head_pad_before_px > 0 || frame.head_pad_after_px > 0);

        let framed = frame_html_preview_stock(
            "<!doctype html><html><head><style>body{}</style></head><body><div class=\"lbl-label\">Hello, World!</div></body></html>",
            &frame,
        );
        assert!(framed.contains("lbl-stock"));
        assert!(
            !framed.contains("width:0.00px"),
            "open feed axis must not clip content in a 0-wide print box: {framed}"
        );
        assert!(
            framed.contains("width:max-content"),
            "open feed axis must size to content, got: {framed}"
        );
        assert!(
            framed.contains(&format!(
                "padding:{}px 0px {}px {}px",
                frame.head_pad_before_px, frame.head_pad_after_px, dx
            )),
            "stock must pad head laminate and feed lead, got: {framed}"
        );
    }

    #[test]
    fn preview_stock_frame_precut_uses_small_lead_and_scrap_flag() {
        use lbl_core::units::Dpi;

        let media = Media::continuous(12.0, Dpi(180.0));
        let caps = DeviceCapabilities {
            dpi: Dpi(180.0),
            max_width_mm: 12.0,
            supports_cut: true,
            feed_trail_mm: Some(24.0),
            feed_lead_mm: Some(2.0),
            supports_precut: true,
            head_printable_height_mm: Some(8.2),
            ..Default::default()
        };
        let content_h = mm_to_layout_px(8.2, VECTOR_CSS_DPI);
        let frame = preview_stock_frame(
            0.0,
            content_h,
            &media,
            &caps,
            true,
            VECTOR_CSS_DPI,
            &PreviewFeedOverrides {
                cut_mode: CutMode::Every,
                feed_lead_mm: Some(2.0),
                feed_end_mm: None,
                precut: Some(true),
                ..Default::default()
            },
        );
        let lead = feed_margin_px(Some(2.0), VECTOR_CSS_DPI);
        let dx = feed_margin_px(Some(24.0), VECTOR_CSS_DPI);
        assert_eq!(frame.lead_feed_px, lead);
        assert_eq!(frame.trail_feed_px, dx);
        assert!(frame.precut);
    }

    #[test]
    fn preview_shows_lead_from_cutter_gap_when_unset() {
        use image::Rgba;

        let image = RgbaImage::from_pixel(10, 4, Rgba([0, 0, 0, 255]));
        let caps = DeviceCapabilities {
            dpi: Dpi(180.0),
            feed_trail_mm: Some(8.1),
            ..Default::default()
        };
        let padded = pad_preview_encode_feed(image, &caps, true);
        let dx = feed_margin_px(caps.feed_trail_mm, caps.dpi.0);
        assert_eq!(padded.lead_feed_px, dx);
        assert_eq!(padded.trail_feed_px, dx);
        assert_eq!(padded.content_feed_end_px, dx + 10);
        assert_eq!(padded.feed_end_margin_px, 0);
        assert_eq!(padded.image.width(), 10 + dx);
        assert!(!padded.precut);
    }

    #[test]
    fn preview_precut_plan_keeps_scrap_as_metadata() {
        use image::Rgba;

        let image = RgbaImage::from_pixel(10, 4, Rgba([0, 0, 0, 255]));
        let plan = lbl_core::FeedPlan {
            lead_mm: 2.0,
            end_mm: 0.0,
            precut: true,
            cutter_gap_mm: 24.0,
        };
        let padded = pad_preview_encode_feed_plan(image, &plan, 180.0, true);
        let lead = feed_mm_px(2.0, 180.0);
        let dx = feed_mm_px(24.0, 180.0);
        assert_eq!(padded.lead_feed_px, lead);
        assert_eq!(padded.feed_end_margin_px, 0);
        assert_eq!(padded.trail_feed_px, dx);
        assert!(padded.precut);
        assert_eq!(padded.image.width(), 10 + lead);
    }

    #[test]
    fn preview_explicit_lead_via_plan_without_precut() {
        use image::Rgba;

        // Caps-only path uses unset→Dx; explicit small lead needs a FeedPlan.
        let image = RgbaImage::from_pixel(10, 4, Rgba([0, 0, 0, 255]));
        let plan = lbl_core::FeedPlan {
            lead_mm: 2.0,
            end_mm: 0.0,
            precut: false,
            cutter_gap_mm: 24.0,
        };
        let padded = pad_preview_encode_feed_plan(image, &plan, 180.0, true);
        let lead = feed_mm_px(2.0, 180.0);
        let dx = feed_mm_px(24.0, 180.0);
        assert_eq!(padded.lead_feed_px, lead);
        assert_eq!(padded.feed_end_margin_px, 0);
        assert_eq!(padded.trail_feed_px, dx);
        assert_eq!(padded.content_feed_end_px, lead + 10);
        assert_eq!(padded.image.width(), 10 + lead);
        assert!(!padded.precut);
    }

    #[test]
    fn file_output_keeps_landscape_reading_orientation() {
        let registry = Registry::with_builtin_drivers();
        let trace = encode_label_traced(
            &SolidBackend,
            &registry,
            0,
            "<div>x</div>",
            &landscape_opts(Protocol::Virtual),
        )
        .unwrap();
        let (w, h) = trace.rendered.dimensions();
        // A 12×40 landscape label should render wider than tall in the file —
        // not turned onto a (nonexistent) print head.
        assert!(w > h, "expected landscape file, got {w}×{h}");
        assert_eq!(trace.rotation, Rotation::None);
    }

    #[test]
    fn hardware_output_turns_landscape_onto_the_head() {
        let registry = Registry::with_builtin_drivers();
        let trace = encode_label_traced(
            &SolidBackend,
            &registry,
            0,
            "<div>x</div>",
            &landscape_opts(Protocol::Zpl),
        )
        .unwrap();
        let (w, h) = trace.rendered.dimensions();
        // A physical head is 12 mm wide, so the same label is turned to be
        // taller than wide before encoding.
        assert!(h > w, "expected head-oriented raster, got {w}×{h}");
        assert_eq!(trace.rotation, Rotation::Cw90);
    }

    #[test]
    fn dymo_landscape_keeps_feed_along_bitmap_width() {
        let registry = Registry::with_builtin_drivers();
        let trace = encode_label_traced(
            &SolidBackend,
            &registry,
            0,
            "<div>x</div>",
            &landscape_opts(Protocol::Dymo),
        )
        .unwrap();
        let (w, h) = trace.rendered.dimensions();
        assert!(w > h, "expected feed-oriented DYMO raster, got {w}×{h}");
        assert_eq!(trace.rotation, Rotation::None);
    }

    #[test]
    fn dymo_lw_landscape_turns_onto_the_head() {
        let registry = Registry::with_builtin_drivers();
        // 54×101 mm name-badge stock is longer than the 57 mm head along feed.
        // Misclassifying DymoLw as feed-oriented leaves feed on bitmap width and
        // LabelWriter550Driver::pad_to_head rejects it (> 672 dots).
        let mut opts = landscape_opts(Protocol::DymoLw);
        opts.media = Media::fixed(54.0, 101.0, Dpi(300.0));
        opts.encode_caps = DeviceCapabilities {
            dpi: Dpi(300.0),
            max_width_mm: 57.0,
            ..Default::default()
        };
        opts.rotation = Rotation::for_print_with_media(Orientation::Landscape, &opts.media, 0, 0);
        opts.head_rotation = Rotation::for_head_with_media(
            Orientation::Landscape,
            &opts.media,
            0,
            0,
            Protocol::DymoLw,
        );
        let trace = encode_label_traced(&SolidBackend, &registry, 0, "<div>x</div>", &opts)
            .expect("LabelWriter encode should accept head-oriented raster");
        let (w, h) = trace.rendered.dimensions();
        assert!(
            h > w,
            "expected head-oriented LabelWriter raster, got {w}×{h}"
        );
        assert_eq!(trace.rotation, Rotation::Cw90);
        assert!(
            w <= 672,
            "bitmap width {w} must fit the 57 mm LabelWriter head"
        );
    }

    #[test]
    fn text_source_makes_one_label() {
        let labels = authoring_labels(
            Source::Text {
                text: "hi [[qr:x]]".into(),
                raw: false,
            },
            &BatchSelection::default(),
        )
        .unwrap();
        assert_eq!(labels.len(), 1);
        assert!(labels[0].html.contains("<qr>x</qr>"));
    }

    #[test]
    fn stamp_directives_resolve_in_authoring_labels() {
        let labels = authoring_labels(
            Source::Text {
                text: "Prep [[date:%Y-%m-%d]]".into(),
                raw: false,
            },
            &BatchSelection::default(),
        )
        .unwrap();
        assert_eq!(labels.len(), 1);
        assert!(
            !labels[0].html.contains("<stamp"),
            "stamp should be resolved: {}",
            labels[0].html
        );
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert!(
            labels[0].html.contains(&today),
            "expected {today} in {}",
            labels[0].html
        );
    }

    #[test]
    fn html_stamp_elements_resolve() {
        let labels = authoring_labels(
            Source::Html(
                r#"<div class="lbl-label"><stamp kind="time" format="%H:%M"></stamp></div>"#.into(),
            ),
            &BatchSelection::default(),
        )
        .unwrap();
        assert!(!labels[0].html.contains("<stamp"));
        let now_hm = chrono::Local::now().format("%H:%M").to_string();
        assert!(
            labels[0].html.contains(&now_hm),
            "expected {now_hm} in {}",
            labels[0].html
        );
    }

    #[test]
    fn template_source_batches() {
        let labels = authoring_labels(
            Source::Template {
                template: "<div>{{ name }}</div>".into(),
                data: Some(serde_json::json!([{"name":"A"},{"name":"B"}])),
                each: None,
                format: TemplateFormat::Html,
            },
            &BatchSelection::default(),
        )
        .unwrap();
        assert_eq!(labels.len(), 2);
    }

    #[test]
    fn infer_template_format_from_path_maps_extensions() {
        assert_eq!(
            infer_template_format_from_path("card.html"),
            Some(TemplateFormat::Html)
        );
        assert_eq!(
            infer_template_format_from_path("combined.lbl"),
            Some(TemplateFormat::Html)
        );
        assert_eq!(
            infer_template_format_from_path("note.md"),
            Some(TemplateFormat::Markdown)
        );
        assert_eq!(infer_template_format_from_path("User #{{ it }}"), None);
        assert_eq!(infer_template_format_from_path("-"), None);
    }

    #[test]
    fn resolve_template_format_prefers_explicit_override() {
        assert_eq!(
            resolve_template_format("card.html", Some(TemplateFormat::Text)),
            TemplateFormat::Text
        );
        assert_eq!(
            resolve_template_format("card.html", None),
            TemplateFormat::Html
        );
    }

    #[test]
    fn text_template_format_batches_through_lbl_text() {
        let labels = authoring_labels(
            Source::Template {
                template: "User #{{ index + 1 }}".into(),
                data: Some(serde_json::json!([{}, {}])),
                each: None,
                format: TemplateFormat::Text,
            },
            &BatchSelection::default(),
        )
        .unwrap();
        assert_eq!(labels.len(), 2);
        assert!(labels[0].html.contains("User #1"));
        assert!(labels[1].html.contains("User #2"));
        assert!(labels[0].html.contains("lbl-label"));
    }

    #[test]
    fn template_interpolation_composes_with_directives() {
        // `[[…]]` directives pass through minijinja untouched, so template
        // data can feed per-label directive payloads.
        let labels = authoring_labels(
            Source::Template {
                template: "[[size:1.4:{{ name }}]]\n[[qr:https://x/{{ id }}]]".into(),
                data: Some(serde_json::json!([
                    {"name":"Alpha","id":1},
                    {"name":"Beta","id":2}
                ])),
                each: None,
                format: TemplateFormat::Text,
            },
            &BatchSelection::default(),
        )
        .unwrap();
        assert_eq!(labels.len(), 2);
        assert!(
            labels[0].html.contains(">Alpha</span>"),
            "{}",
            labels[0].html
        );
        assert!(
            labels[0].html.contains("<qr>https://x/1</qr>"),
            "{}",
            labels[0].html
        );
        assert!(
            labels[1].html.contains("<qr>https://x/2</qr>"),
            "{}",
            labels[1].html
        );
    }

    #[test]
    fn resolve_media_from_catalog_sku() {
        let catalog = Catalog::bundled().unwrap();
        let media = resolve_media(&catalog, Some("11352"), None, None, 300.0).unwrap();
        assert_eq!(media.width_mm, 25.0);
    }

    #[test]
    fn resolve_status_from_catalog_printer() {
        let catalog = Catalog::bundled().unwrap();
        let config = lbl_config::Config::default();
        let target =
            resolve_status_target(&catalog, &config, Some("LabelWriter 550"), None, None).unwrap();
        assert_eq!(target.usb, "0922:0028");
        assert_eq!(target.protocol, Protocol::DymoLw);
    }

    #[test]
    fn resolve_media_requires_something() {
        let catalog = Catalog::bundled().unwrap();
        assert!(resolve_media(&catalog, None, None, None, 300.0).is_err());
    }

    #[test]
    fn resolve_head_dots_from_media_width() {
        use lbl_pattern::resolve_head_dots;
        let media = Media::fixed(12.0, 30.0, Dpi(203.0));
        assert_eq!(resolve_head_dots(None, &media).unwrap(), 96);
        assert_eq!(resolve_head_dots(Some(64), &media).unwrap(), 64);
    }

    #[test]
    fn encode_from_rgba_produces_protocol_bytes() {
        let registry = Registry::with_builtin_drivers();
        let opts = PipelineOptions {
            protocol: Protocol::EscPos,
            media: Media::continuous(58.0, Dpi(203.0)),
            supports_cut: false,
            cut_mode: CutMode::None,
            copies: 1,
            batch_index: 0,
            batch_total: 1,
            density: None,
            feed_lead_mm: None,
            feed_end_mm: None,
            precut: None,
            driver: lbl_core::DriverOptions::default(),
            dither: Algorithm::Threshold(128),
            rotation: Rotation::None,
            head_rotation: Rotation::None,
            mirror: false,
            supersample: 1,
            assets_base: AssetsBase::Cdn,
            font_delivery: FontDelivery::default(),
            style: LabelStyle::default(),
            media_type: None,
            virtual_export_mode: VirtualExportMode::Raster,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            font_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
            encode_caps: DeviceCapabilities::default(),
        };
        let img = RgbaImage::from_pixel(64, 32, image::Rgba([0, 0, 0, 255]));
        let result = encode_label_from_rgba(&registry, &img, &opts).unwrap();
        assert_eq!(result.dithered.width, 64);
        assert_eq!(result.dithered.height, 32);
        assert!(!result.encoded.is_empty());
        assert_eq!(result.driver_name, "escpos-raster");
    }

    #[test]
    fn sample_pattern_encodes_without_render_or_dither() {
        let registry = Registry::with_builtin_drivers();
        let opts = PipelineOptions {
            protocol: Protocol::Dymo,
            media: Media::continuous(12.0, Dpi(180.0)),
            supports_cut: false,
            cut_mode: CutMode::None,
            copies: 1,
            batch_index: 0,
            batch_total: 1,
            density: None,
            feed_lead_mm: None,
            feed_end_mm: None,
            precut: None,
            driver: lbl_core::DriverOptions::default(),
            dither: Algorithm::Auto,
            rotation: Rotation::Cw90,
            head_rotation: Rotation::None,
            mirror: false,
            supersample: 3,
            assets_base: AssetsBase::Cdn,
            font_delivery: FontDelivery::default(),
            style: LabelStyle::default(),
            media_type: None,
            virtual_export_mode: VirtualExportMode::Raster,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            font_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
            encode_caps: DeviceCapabilities::default(),
        };
        let trace = encode_sample_pattern_traced(&registry, 0, 64, &opts).unwrap();
        assert_eq!(trace.dithered.height, 64);
        assert_eq!(trace.dithered.width, 191);
        assert_eq!(trace.rotation, Rotation::None);
        assert!(trace.transpiled_html.is_empty());
        assert!(!trace.encoded.is_empty());
    }

    #[test]
    fn sample_pattern_fills_fixed_niimbot_label() {
        let registry = Registry::with_builtin_drivers();
        let opts = PipelineOptions {
            protocol: Protocol::Niimbot,
            media: Media::fixed(12.0, 30.0, Dpi(203.0)),
            supports_cut: false,
            cut_mode: CutMode::None,
            copies: 1,
            batch_index: 0,
            batch_total: 1,
            density: None,
            feed_lead_mm: None,
            feed_end_mm: None,
            precut: None,
            driver: lbl_core::DriverOptions::default(),
            dither: Algorithm::Auto,
            rotation: Rotation::Cw90,
            head_rotation: Rotation::None,
            mirror: false,
            supersample: 3,
            assets_base: AssetsBase::Cdn,
            font_delivery: FontDelivery::default(),
            style: LabelStyle::default(),
            media_type: None,
            virtual_export_mode: VirtualExportMode::Raster,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            font_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
            encode_caps: DeviceCapabilities::default(),
        };
        let trace = encode_sample_pattern_traced(&registry, 0, 96, &opts).unwrap();
        assert_eq!(trace.dithered.width, 96);
        assert_eq!(trace.dithered.height, 240);
    }
}
