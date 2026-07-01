//! Pipeline chaining used by the orchestrator's high-level flows.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use lbl_catalog::{Catalog, ConnectionHint, PrinterEntry};
use lbl_core::job::{JobSpec, OutputMode};
use lbl_core::media::Media;
use lbl_core::printer::{PrinterCapabilities, Protocol};
use lbl_core::units::Dpi;
use lbl_core::Rotation;
use lbl_dither::{dither, Algorithm};
use lbl_driver_api::EncodeContext;
use lbl_driver_file::MediaType;
use lbl_encode::Registry;
use lbl_pattern::sample_pattern_for_media;
use lbl_render::{apply_rotation, render_two_pass, RenderBackend, RenderRequest};

type PrintTransportTargets = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);
pub use lbl_template::BatchSelection;
use lbl_template::{select_batch_indices, Engine, RenderOptions};
use lbl_transpile_html::{
    transpile, AssetsBase, LabelAlign, LabelFit, LabelFitSetting, LabelStyle, LabelValign,
    MediaInset, MediaInsetPx, QrErrorCorrection, TranspileOptions, ViewportPx,
};

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
pub fn authoring_labels(source: Source, selection: &BatchSelection) -> Result<Vec<AuthoringLabel>> {
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
    printer: Option<&PrinterEntry>,
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

fn discover_serial_port(printer: &PrinterEntry) -> Option<String> {
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
        style.padding_mm,
        style.element_gap_mm,
        style.border_width_mm,
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

/// Options for encoding a label all the way to protocol bytes.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    /// Target protocol.
    pub protocol: Protocol,
    /// Resolved media.
    pub media: Media,
    /// Whether the printer can cut.
    pub supports_cut: bool,
    /// Whether to request a cut.
    pub cut: bool,
    /// Copies.
    pub copies: u32,
    /// Dithering algorithm.
    pub dither: Algorithm,
    /// Net rotation applied to the rendered raster (resolved from the requested
    /// orientation plus any extra quarter-turns).
    pub rotation: Rotation,
    /// Supersample factor.
    pub supersample: u32,
    /// Where transpilation loads JS libraries from.
    pub assets_base: AssetsBase,
    /// Font / QR / barcode sizing (already resolved to pixels for this run's
    /// DPI and supersample factor; see [`resolve_style`]).
    pub style: LabelStyle,
    /// For the virtual (`Protocol::Virtual`) printer, the output file format
    /// ("media type"). Ignored by hardware protocols.
    pub media_type: Option<MediaType>,
    /// How the label root fills the render viewport (resolved from config/CLI).
    pub label_fit: LabelFit,
    /// Cross-axis alignment when the viewport width is known (resolved from config/CLI).
    pub label_align: LabelAlign,
    /// Main-axis alignment in fill mode (resolved from config/CLI).
    pub label_valign: LabelValign,
    /// Fit-box scale in fill mode (resolved from config/CLI).
    pub label_fit_scale: f64,
    /// Inset from the physical media edge (resolved from config/CLI).
    pub media_inset: MediaInsetPx,
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

    for label in labels {
        let started = Instant::now();
        let trace = encode_label_traced(backend, registry, label.index, &label.html, opts)
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

/// CSS-pixel viewport matching the rasterizer's [`RenderRequest`] dimensions.
pub fn render_viewport_px(media: &Media, supersample: u32, rotation: Rotation) -> ViewportPx {
    let factor = supersample.max(1) as f64;
    let head_dots = media.width_dots().0;
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
    let viewport = render_viewport_px(&opts.media, opts.supersample, opts.rotation);
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: opts.assets_base.clone(),
            index: None,
            count: None,
            style: opts.style.clone(),
            label_fit: opts.label_fit,
            viewport: Some(viewport),
            label_align: opts.label_align,
            label_valign: opts.label_valign,
            label_fit_scale: opts.label_fit_scale,
            media_inset: opts.media_inset,
        },
    );

    // The print head spans `head_dots` and the media advances along
    // `feed_dots` (content-determined for continuous media). Lay content out in
    // the chosen reading frame: a quarter-turn (landscape) transposes the
    // render canvas so text runs along the feed, then we rotate the raster back
    // onto the head.
    let head_dots = opts.media.width_dots().0;
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

    // The quarter-turn maps the reading frame onto a physical print head. On
    // screen sinks (image file, console preview) have no head, so they keep the
    // reading orientation — a landscape label stays landscape in the file.
    let applied_rotation = if opts.protocol.targets_print_head() {
        opts.rotation
    } else {
        Rotation::None
    };
    let rendered = apply_rotation(rendered, applied_rotation);

    let dithered = dither(&rendered, opts.dither);

    let (encoded, driver_name) = if opts.protocol == Protocol::Html {
        (Vec::new(), "html-preview".to_string())
    } else {
        let mut job = JobSpec::new(opts.media.clone());
        job.cut = opts.cut;
        job.copies = opts.copies;
        let caps = PrinterCapabilities {
            dpi: opts.media.dpi,
            max_width_mm: opts.media.width_mm,
            supports_cut: opts.supports_cut,
            reports_media: false,
        };
        let driver = registry
            .get(opts.protocol)
            .ok_or_else(|| anyhow!("no driver for protocol {:?}", opts.protocol))?;
        let ctx = EncodeContext::new(&job, &caps);
        (
            driver.encode(&dithered, &ctx).context("encoding")?,
            driver.name().to_string(),
        )
    };

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

/// Encode a calibration sample pattern straight to protocol bytes (no render,
/// dither, rotation, or rescaling). `head_dots` is the pattern height across
/// the print head, matching Labelle's `--sample-pattern`.
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
    job.cut = opts.cut;
    job.copies = opts.copies;
    let caps = PrinterCapabilities {
        dpi: opts.media.dpi,
        max_width_mm: opts.media.width_mm,
        supports_cut: opts.supports_cut,
        reports_media: false,
    };
    let driver = registry
        .get(opts.protocol)
        .ok_or_else(|| anyhow!("no driver for protocol {:?}", opts.protocol))?;
    let ctx = EncodeContext::new(&job, &caps);
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

#[cfg(test)]
mod tests {
    use super::*;

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
        PipelineOptions {
            protocol,
            media: Media::fixed(12.0, 40.0, Dpi(203.0)),
            supports_cut: false,
            cut: false,
            copies: 1,
            dither: Algorithm::Threshold(128),
            rotation: Rotation::Cw90,
            supersample: 1,
            assets_base: AssetsBase::Cdn,
            style: LabelStyle::default(),
            media_type: None,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
        }
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
    fn text_source_makes_one_label() {
        let labels = authoring_labels(
            Source::Text {
                text: "hi {{qr:x}}".into(),
                raw: false,
            },
            &BatchSelection::default(),
        )
        .unwrap();
        assert_eq!(labels.len(), 1);
        assert!(labels[0].html.contains("<qr>x</qr>"));
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
    fn resolve_media_from_catalog_sku() {
        let catalog = Catalog::bundled().unwrap();
        let media = resolve_media(&catalog, Some("11352"), None, None, 300.0).unwrap();
        assert_eq!(media.width_mm, 25.0);
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
    fn sample_pattern_encodes_without_render_or_dither() {
        let registry = Registry::with_builtin_drivers();
        let opts = PipelineOptions {
            protocol: Protocol::Dymo,
            media: Media::continuous(12.0, Dpi(180.0)),
            supports_cut: false,
            cut: false,
            copies: 1,
            dither: Algorithm::Auto,
            rotation: Rotation::Cw90,
            supersample: 3,
            assets_base: AssetsBase::Cdn,
            style: LabelStyle::default(),
            media_type: None,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
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
            cut: false,
            copies: 1,
            dither: Algorithm::Auto,
            rotation: Rotation::Cw90,
            supersample: 3,
            assets_base: AssetsBase::Cdn,
            style: LabelStyle::default(),
            media_type: None,
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
        };
        let trace = encode_sample_pattern_traced(&registry, 0, 96, &opts).unwrap();
        assert_eq!(trace.dithered.width, 96);
        assert_eq!(trace.dithered.height, 240);
    }
}
