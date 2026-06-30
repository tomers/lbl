//! Pipeline chaining used by the orchestrator's high-level flows.

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
use lbl_render::{apply_rotation, render_two_pass, RenderBackend, RenderRequest};
use lbl_template::{Engine, RenderOptions};
use lbl_transpile_html::{transpile, AssetsBase, LabelFit, LabelFitSetting, LabelStyle, QrErrorCorrection, TranspileOptions};

/// A single authoring-HTML label with its batch index.
#[derive(Debug, Clone)]
pub struct AuthoringLabel {
    /// Zero-based index within the batch.
    pub index: usize,
    /// Authoring HTML (pre-transpilation).
    pub html: String,
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
    },
}

/// Turn a [`Source`] into one or more authoring-HTML labels.
pub fn authoring_labels(source: Source) -> Result<Vec<AuthoringLabel>> {
    match source {
        Source::Text { text, raw } => {
            let doc = lbl_text::Document::parse(&text, raw);
            Ok(vec![AuthoringLabel {
                index: 0,
                html: doc.to_authoring_document(),
            }])
        }
        Source::Markdown(markdown) => {
            let doc = lbl_markdown::MarkdownDocument::parse(&markdown);
            Ok(vec![AuthoringLabel {
                index: 0,
                html: doc.to_authoring_document(),
            }])
        }
        Source::Html(html) => Ok(vec![AuthoringLabel { index: 0, html }]),
        Source::Template {
            template,
            data,
            each,
        } => {
            let labels = Engine::new()
                .render(&template, data, &RenderOptions { each })
                .context("rendering template")?;
            Ok(labels
                .into_iter()
                .map(|l| AuthoringLabel {
                    index: l.index,
                    html: l.html,
                })
                .collect())
        }
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
) -> Result<(Option<String>, Option<String>, Option<String>, Option<String>)> {
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
}

/// Resolve a [`LabelFitSetting`] against the target media.
pub fn resolve_label_fit(setting: LabelFitSetting, media: &Media) -> LabelFit {
    setting.resolve(media.length_dots().is_some())
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
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: opts.assets_base.clone(),
            index: None,
            count: None,
            style: opts.style.clone(),
            label_fit: opts.label_fit,
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
    let encoded = driver.encode(&dithered, &ctx).context("encoding")?;

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
        driver_name: driver.name().to_string(),
        media_type: opts.media_type,
        encoded,
    })
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
            Ok(image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255])))
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
        let labels = authoring_labels(Source::Text {
            text: "hi {{qr:x}}".into(),
            raw: false,
        })
        .unwrap();
        assert_eq!(labels.len(), 1);
        assert!(labels[0].html.contains("<qr>x</qr>"));
    }

    #[test]
    fn template_source_batches() {
        let labels = authoring_labels(Source::Template {
            template: "<div>{{ name }}</div>".into(),
            data: Some(serde_json::json!([{"name":"A"},{"name":"B"}])),
            each: None,
        })
        .unwrap();
        assert_eq!(labels.len(), 2);
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
}
