//! `lbl` — the orchestrator for the label pipeline.
//!
//! High-level flows (`print`, `preview`) chain the stages together; individual
//! stage subcommands (`text`, `transpile`, `render`, `dither`, `encode`,
//! `catalog`, `device`) expose each step, mirroring the standalone `lbl-*`
//! binaries.

use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lbl::pipeline::{authoring_labels, resolve_label_align, resolve_label_fit, resolve_label_fit_scale, resolve_label_valign, resolve_media, resolve_print_transport, resolve_style, render_viewport_px, PipelineOptions, Source};
use lbl_catalog::Catalog;
use lbl_config::StyleConfig;
use lbl_core::job::OutputMode;
use lbl_core::printer::Protocol;
use lbl_core::{Orientation, Rotation};
use lbl_dither::Algorithm;
use lbl_driver_file::MediaType;
use lbl_encode::Registry;
use lbl_render::{ChromiumBackend, RenderBackend, SidecarBackend};
use lbl_transpile_html::{parse_fit_scale, transpile, AssetsBase, LabelAlign, LabelFit, LabelFitSetting, LabelValign, TranspileOptions};

#[derive(Parser)]
#[command(
    name = "lbl",
    version,
    about = "Orchestrate the lbl label-printing pipeline",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Render text/template/HTML through the full pipeline and print it.
    Print(PrintArgs),
    /// Produce browser-ready preview HTML (and optional PNGs) for a gallery.
    Preview(PreviewArgs),
    /// Convert text/CLI directives into authoring HTML.
    Text(TextArgs),
    /// Convert Markdown (with inline directives) into authoring HTML.
    Markdown(MarkdownArgs),
    /// Transpile authoring HTML into browser-ready HTML.
    Transpile(TranspileArgs),
    /// Browse the media catalog.
    Catalog(CatalogArgs),
    /// Discover connected printers.
    Device(DeviceArgs),
}

// ---------------------------------------------------------------------------
// Shared argument groups
// ---------------------------------------------------------------------------

#[derive(Args, Clone)]
struct SourceArgs {
    /// Plain text input (run through lbl-text).
    #[arg(long, group = "src")]
    text: Option<String>,

    /// Treat --text literally (no inline directives).
    #[arg(long)]
    raw: bool,

    /// Markdown input (run through lbl-markdown; inline directives still apply).
    #[arg(long, group = "src")]
    markdown: Option<String>,

    /// Authoring HTML file (or `-` for stdin).
    #[arg(long, group = "src")]
    html: Option<String>,

    /// Template file (rendered with --data).
    #[arg(long, group = "src")]
    template: Option<String>,

    /// Data file/URL for --template.
    #[arg(long)]
    data: Option<String>,

    /// JSON-pointer to a batch array within the data.
    #[arg(long)]
    each: Option<String>,
}

#[derive(Args, Clone)]
struct MediaArgs {
    /// Media SKU/alias resolved via the catalog (e.g. 11352).
    #[arg(long)]
    media: Option<String>,

    /// Media width in mm (when not using --media).
    #[arg(long)]
    width_mm: Option<f64>,

    /// Fixed media length in mm (omit for continuous).
    #[arg(long)]
    length_mm: Option<f64>,

    /// Device resolution in DPI (defaults to the selected printer's native DPI).
    #[arg(long, default_value_t = lbl_catalog::CLI_DEFAULT_DPI)]
    dpi: f64,
}

/// CLI overrides for label sizing, in millimetres. Any value left unset falls
/// back to the loaded configuration (`[style]`), which itself defaults to
/// readable sizes.
#[derive(Args, Clone, Default)]
struct StyleArgs {
    /// Base text size, in mm (overrides config `style.font_size_mm`).
    #[arg(long)]
    font_size_mm: Option<f64>,

    /// QR code edge length, in mm (overrides config `style.qr_size_mm`).
    #[arg(long)]
    qr_size_mm: Option<f64>,

    /// QR error-correction level: L/M/Q/H or low/medium/quartile/high
    /// (overrides config `style.qr_error_correction`).
    #[arg(long = "qr-ec", alias = "qr-error-correction")]
    qr_error_correction: Option<String>,

    /// QR quiet zone, in modules; 0 = none (overrides config `style.qr_margin`).
    #[arg(long)]
    qr_margin: Option<u32>,

    /// QR dark module color, hex (overrides config `style.qr_dark`).
    #[arg(long)]
    qr_dark: Option<String>,

    /// QR light module color, hex (overrides config `style.qr_light`).
    #[arg(long)]
    qr_light: Option<String>,

    /// Barcode bar height, in mm (overrides config `style.barcode_height_mm`).
    #[arg(long)]
    barcode_height_mm: Option<f64>,

    /// Barcode narrowest-bar width, in mm (overrides config
    /// `style.barcode_module_width_mm`).
    #[arg(long)]
    barcode_module_mm: Option<f64>,

    /// Inner padding between the label edge and its content, in mm (overrides
    /// config `style.padding_mm`).
    #[arg(long)]
    padding_mm: Option<f64>,

    /// Border drawn around the label, in mm; 0 disables it (overrides config
    /// `style.border_width_mm`).
    #[arg(long)]
    border_mm: Option<f64>,

    /// How the label root fills the media viewport: `auto` fills fixed-length
    /// media and shrinks on continuous; `fill` or `content` force a mode
    /// (overrides config `style.label_fit`).
    #[arg(long, value_enum)]
    label_fit: Option<LabelFitArg>,

    /// Cross-axis alignment when the media width is known: `start`, `center`,
    /// or `end` (overrides config `style.label_align`).
    #[arg(long, value_enum)]
    label_align: Option<LabelAlignArg>,

    /// Main-axis alignment in fill mode: `start`, `center`, or `end` (overrides
    /// config `style.label_valign`).
    #[arg(long, value_enum)]
    label_valign: Option<LabelValignArg>,

    /// Fit-box scale in fill mode (`0.8`, `80%`, …; overrides config
    /// `style.label_fit_scale`).
    #[arg(long)]
    label_fit_scale: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum LabelValignArg {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, ValueEnum)]
enum LabelAlignArg {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, ValueEnum)]
enum LabelFitArg {
    Auto,
    Fill,
    Content,
}

impl StyleArgs {
    /// Merge these CLI overrides over the loaded configuration's style section.
    fn resolve(&self) -> StyleConfig {
        let mut style = lbl_config::Loader::new()
            .load()
            .map(|cfg| cfg.style)
            .unwrap_or_default();
        if let Some(v) = self.font_size_mm {
            style.font_size_mm = v;
        }
        if let Some(v) = self.qr_size_mm {
            style.qr_size_mm = v;
        }
        if let Some(v) = &self.qr_error_correction {
            style.qr_error_correction = v.clone();
        }
        if let Some(v) = self.qr_margin {
            style.qr_margin = v;
        }
        if let Some(v) = &self.qr_dark {
            style.qr_dark = v.clone();
        }
        if let Some(v) = &self.qr_light {
            style.qr_light = v.clone();
        }
        if let Some(v) = self.barcode_height_mm {
            style.barcode_height_mm = v;
        }
        if let Some(v) = self.barcode_module_mm {
            style.barcode_module_width_mm = v;
        }
        if let Some(v) = self.padding_mm {
            style.padding_mm = v;
        }
        if let Some(v) = self.border_mm {
            style.border_width_mm = v;
        }
        if let Some(v) = self.label_fit {
            style.label_fit = match v {
                LabelFitArg::Auto => "auto",
                LabelFitArg::Fill => "fill",
                LabelFitArg::Content => "content",
            }
            .into();
        }
        if let Some(v) = self.label_align {
            style.label_align = match v {
                LabelAlignArg::Start => "start",
                LabelAlignArg::Center => "center",
                LabelAlignArg::End => "end",
            }
            .into();
        }
        if let Some(v) = self.label_valign {
            style.label_valign = match v {
                LabelValignArg::Start => "start",
                LabelValignArg::Center => "center",
                LabelValignArg::End => "end",
            }
            .into();
        }
        style
    }

    fn fit_scale(&self, style: &StyleConfig) -> f64 {
        if let Some(raw) = &self.label_fit_scale {
            parse_fit_scale(raw).unwrap_or(style.label_fit_scale)
        } else {
            style.label_fit_scale
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ProtocolArg {
    Dymo,
    #[value(name = "dymo-lw", alias = "lw550")]
    DymoLw,
    Escpos,
    Zpl,
    Tspl,
    /// NIIMBOT thermal label printers (D11 / D110 family).
    Niimbot,
    /// Virtual printer: render to an image file instead of hardware.
    #[value(alias = "file")]
    Virtual,
    /// Console printer: render the raster to the terminal as text.
    #[value(alias = "term")]
    Console,
}

impl From<ProtocolArg> for Protocol {
    fn from(p: ProtocolArg) -> Self {
        match p {
            ProtocolArg::Dymo => Protocol::Dymo,
            ProtocolArg::DymoLw => Protocol::DymoLw,
            ProtocolArg::Escpos => Protocol::EscPos,
            ProtocolArg::Zpl => Protocol::Zpl,
            ProtocolArg::Tspl => Protocol::Tspl,
            ProtocolArg::Niimbot => Protocol::Niimbot,
            ProtocolArg::Virtual => Protocol::Virtual,
            ProtocolArg::Console => Protocol::Console,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum NiimbotTaskArg {
    /// Legacy D110 / B-series (PageStart + separate PrintQuantity).
    Standard,
    /// D110M V4 (9-byte PrintStart, 13-byte SetPageSize, no PageStart).
    V4,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Chromium,
    Sidecar,
}

#[derive(Clone, Copy, ValueEnum)]
enum OrientationArg {
    /// Content runs across the head (the media's natural width × length frame).
    Portrait,
    /// Content runs along the feed (transposed and turned onto the head).
    Landscape,
}

impl From<OrientationArg> for Orientation {
    fn from(o: OrientationArg) -> Self {
        match o {
            OrientationArg::Portrait => Orientation::Portrait,
            OrientationArg::Landscape => Orientation::Landscape,
        }
    }
}

// ---------------------------------------------------------------------------
// print
// ---------------------------------------------------------------------------

#[derive(Args)]
struct PrintArgs {
    #[command(flatten)]
    source: SourceArgs,
    #[command(flatten)]
    media: MediaArgs,
    #[command(flatten)]
    style: StyleArgs,

    /// Target protocol. May also be set in config (`[print] protocol`) or via
    /// `LBL_PRINT__PROTOCOL`.
    #[arg(long, value_enum)]
    protocol: Option<ProtocolArg>,

    /// Printer model key from the catalog (e.g. `LabelWriter 550`, `D110`).
    /// Sets protocol, native DPI, default media, and transport when the
    /// corresponding flags are omitted.
    #[arg(long)]
    printer: Option<String>,

    /// For `--protocol virtual`: the output image format ("media type"):
    /// png | bmp | tiff | gif | pbm. Defaults to png.
    #[arg(long)]
    media_type: Option<String>,

    /// Supersample factor for the high-resolution render pass (>= 1). Controls
    /// the two-pass downscale in `lbl-render` and CSS pixel sizing during
    /// transpilation. Overrides config `render.supersample` when set.
    #[arg(long)]
    supersample: Option<u32>,

    /// Dithering algorithm. Overrides config `[print] dither` /
    /// `LBL_PRINT__DITHER` when set.
    #[arg(long)]
    dither: Option<String>,

    /// Lay content out in portrait or landscape. Landscape (the default) runs
    /// text along the feed — the longer dimension of typical stripe labels.
    /// Overrides config `render.orientation` when set.
    #[arg(long, value_enum)]
    orientation: Option<OrientationArg>,

    /// Rotate the label an extra 90° clockwise (repeatable, and combines with
    /// --orientation).
    #[arg(long, action = clap::ArgAction::Count)]
    rotate_cw: u8,

    /// Rotate the label an extra 90° counter-clockwise (repeatable, and
    /// combines with --orientation).
    #[arg(long, action = clap::ArgAction::Count)]
    rotate_ccw: u8,

    /// Request a cut after each label. Overrides config `[print] cut` /
    /// `LBL_PRINT__CUT`.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    cut: Option<bool>,

    /// Mark target printer as cut-capable. Overrides config `[print]
    /// supports_cut` / `LBL_PRINT__SUPPORTS_CUT`.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    supports_cut: Option<bool>,

    /// Copies per label. Overrides config `[print] copies` /
    /// `LBL_PRINT__COPIES`.
    #[arg(long)]
    copies: Option<u32>,

    /// Rendering backend. Overrides config `[print] backend` /
    /// `LBL_PRINT__BACKEND`.
    #[arg(long, value_enum)]
    backend: Option<BackendArg>,

    /// Network target host:port.
    #[arg(long)]
    network: Option<String>,

    /// USB target vid:pid (hex).
    #[arg(long)]
    usb: Option<String>,

    /// Serial (USB CDC-ACM) target: a device path, optionally with a baud rate
    /// (`/dev/ttyACM0` or `/dev/ttyACM0:115200`). Used by NIIMBOT B-series.
    #[arg(long)]
    serial: Option<String>,

    /// Bluetooth LE target: the printer's advertised name or address
    /// (e.g. `D110`). Requires building with the `ble` feature. Used by the
    /// cable-less NIIMBOT D-series.
    #[arg(long)]
    bluetooth: Option<String>,

    /// NIIMBOT print-task variant. Use `v4` for 2025+ D110M firmware over BLE;
    /// `standard` (default) for B-series USB and older D110 units. Overrides
    /// config `[print] niimbot_task` / `LBL_PRINT__NIIMBOT_TASK`.
    #[arg(long, value_enum)]
    niimbot_task: Option<NiimbotTaskArg>,

    /// Instead of printing, write encoded bytes to this directory.
    #[arg(long)]
    out_dir: Option<std::path::PathBuf>,

    /// Print to a file (the virtual-printer target). For multiple labels,
    /// siblings are numbered (out.png, out-01.png, ...).
    #[arg(long)]
    file: Option<std::path::PathBuf>,

    /// Write an HTML report documenting every pipeline stage (command-line
    /// equivalents plus before/after views) to this path.
    #[arg(long)]
    debug_html: Option<std::path::PathBuf>,

    /// Print a per-stage debug dump to stderr: the authoring and transpiled
    /// HTML (syntax-highlighted when stderr is a TTY), the dithered raster as
    /// terminal art, and an encoded-byte preview. Overrides config `[print]
    /// debug` / `LBL_PRINT__DEBUG`.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    debug: Option<bool>,

    /// Before sending to a device or file, show a terminal preview of each
    /// label and ask for confirmation. Ignored for `--protocol console`.
    /// Overrides config `[print] confirm` / `LBL_PRINT__CONFIRM`.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    confirm: Option<bool>,
}

fn config_enum<E: ValueEnum>(name: &str, value: &str) -> Result<E> {
    E::from_str(value, true).map_err(|e| anyhow!("invalid config {name}: {e}"))
}

fn run_print(args: PrintArgs) -> Result<()> {
    let config = lbl_config::Loader::new()
        .load()
        .unwrap_or_else(|_| lbl_config::Config::default());
    let print_cfg = &config.print;

    let catalog = Catalog::bundled()?;
    let printer_entry = args.printer.as_deref().and_then(|key| {
        catalog
            .lookup_printer(key)
            .or_else(|| catalog.match_printer(key))
    });

    let protocol: Protocol = match args.protocol {
        Some(p) => p.into(),
        None => {
            if let Some(printer) = printer_entry {
                printer.protocol
            } else {
                match &print_cfg.protocol {
                    Some(name) => config_enum::<ProtocolArg>("print.protocol", name)?.into(),
                    None => bail!(
                        "protocol required: pass --protocol, --printer, or set [print] protocol / LBL_PRINT__PROTOCOL"
                    ),
                }
            }
        }
    };

    let confirm = args.confirm.unwrap_or(print_cfg.confirm);
    let debug = args.debug.unwrap_or(print_cfg.debug);
    let cut = args.cut.unwrap_or(print_cfg.cut);
    let supports_cut = args.supports_cut.unwrap_or(print_cfg.supports_cut);
    let copies = args.copies.unwrap_or(print_cfg.copies);
    let dither = args
        .dither
        .unwrap_or_else(|| print_cfg.dither.clone());
    let backend = match args.backend {
        Some(b) => b,
        None => config_enum::<BackendArg>("print.backend", &print_cfg.backend)?,
    };
    let niimbot_task = match args.niimbot_task {
        Some(t) => t,
        None => config_enum::<NiimbotTaskArg>("print.niimbot_task", &print_cfg.niimbot_task)?,
    };
    let (network, usb, serial, bluetooth) = resolve_print_transport(
        printer_entry,
        args.network.or_else(|| print_cfg.network.clone()),
        args.usb.or_else(|| print_cfg.usb.clone()),
        args.serial.or_else(|| print_cfg.serial.clone()),
        args.bluetooth.or_else(|| print_cfg.bluetooth.clone()),
    )?;
    let media_type_name = args.media_type.or_else(|| print_cfg.media_type.clone());

    let dpi = catalog.resolve_dpi(args.printer.as_deref(), protocol, args.media.dpi);
    let media_sku = args
        .media
        .media
        .clone()
        .or_else(|| printer_entry.and_then(|p| p.default_media.clone()));
    if let (Some(printer), Some(media_key)) = (&args.printer, &media_sku) {
        if !catalog.supports_media(printer, &media_key) {
            bail!("media '{media_key}' is not supported by printer '{printer}'");
        }
    }
    let media = resolve_media(
        &catalog,
        media_sku.as_deref(),
        args.media.width_mm,
        args.media.length_mm,
        dpi,
    )?;

    // The virtual printer's "media type" is its output file format.
    let media_type = if protocol == Protocol::Virtual {
        Some(match &media_type_name {
            Some(name) => MediaType::parse(name).map_err(|e| anyhow!(e))?,
            None => MediaType::Png,
        })
    } else {
        if media_type_name.is_some() {
            bail!("--media-type only applies to --protocol virtual");
        }
        None
    };

    let render_cfg = config.render;
    let supersample = args.supersample.unwrap_or(render_cfg.supersample);

    // Orientation: explicit CLI flag wins, otherwise the configured default
    // (which itself defaults to landscape). Extra --rotate-cw/--rotate-ccw
    // quarter-turns compose on top to yield the net rotation.
    let orientation = args
        .orientation
        .map(Orientation::from)
        .unwrap_or(render_cfg.orientation);
    let rotation = Rotation::for_print(orientation, args.rotate_cw as u32, args.rotate_ccw as u32);

    let style_cfg = args.style.resolve();
    let style = resolve_style(&style_cfg, media.dpi.0, supersample);
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(args.style.fit_scale(&style_cfg));

    let opts = PipelineOptions {
        protocol,
        media,
        supports_cut,
        cut,
        copies,
        dither: Algorithm::parse(&dither)?,
        rotation,
        supersample,
        assets_base: AssetsBase::Cdn,
        style,
        media_type,
        label_fit,
        label_align,
        label_valign,
        label_fit_scale,
    };

    let labels = authoring_labels(read_source(&args.source)?)?;

    // The virtual driver carries its selected media type, so register an
    // instance configured for this run (overriding the PNG default).
    let mut registry = Registry::with_builtin_drivers();
    if let Some(mt) = media_type {
        registry.register(Box::new(lbl_driver_file::FileDriver::new(mt)));
    }
    if protocol == Protocol::Niimbot && niimbot_task == NiimbotTaskArg::V4 {
        registry.register(Box::new(lbl_driver_niimbot::NiimbotDriver::v4()));
    }

    let extension = if protocol == Protocol::Console {
        "txt"
    } else {
        media_type.map(|mt| mt.extension()).unwrap_or("bin")
    };

    // Encode every label, capturing per-stage traces when any feature needs the
    // intermediate artifacts (HTML report, console output, preview, or debug
    // dump), then dispatch.
    let want_trace = args.debug_html.is_some()
        || debug
        || confirm
        || protocol == Protocol::Console;
    let (encoded, traces): (Vec<(String, Vec<u8>)>, Vec<lbl::debug::LabelTrace>) =
        match backend {
            BackendArg::Chromium => {
                let backend = ChromiumBackend::launch()?;
                encode_all(&backend, &registry, &labels, &opts, extension, want_trace)?
            }
            BackendArg::Sidecar => {
                let backend = SidecarBackend::node_default();
                encode_all(&backend, &registry, &labels, &opts, extension, want_trace)?
            }
        };

    if let Some(path) = &args.debug_html {
        let html = lbl::debug::render_report(&traces);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, html)?;
        eprintln!("wrote pipeline debug report to {}", path.display());
    }

    if debug {
        lbl::terminal::dump_debug(&traces)?;
    }

    // Console "printer": render the dithered raster to the terminal. Unless a
    // file target is given, this writes to stdout (in color when it is a TTY)
    // rather than to a device, and never needs confirmation.
    if protocol == Protocol::Console && args.out_dir.is_none() && args.file.is_none() {
        lbl::terminal::dump_rasters(&traces)?;
        return Ok(());
    }

    // Preview-and-confirm before committing to a non-console output.
    if confirm && protocol != Protocol::Console && !lbl::terminal::confirm_print(&traces)? {
        eprintln!("aborted.");
        return Ok(());
    }

    if let Some(dir) = &args.out_dir {
        std::fs::create_dir_all(dir)?;
        for (name, bytes) in &encoded {
            std::fs::write(dir.join(name), bytes)?;
        }
        eprintln!(
            "wrote {} encoded label(s) to {}",
            encoded.len(),
            dir.display()
        );
        return Ok(());
    }

    if let Some(file) = &args.file {
        let mut t = lbl_device::FileTransport::new(file.clone());
        return dispatch_with(encoded, protocol, &mut t);
    }

    if protocol == Protocol::Virtual {
        bail!("virtual printer needs an output target; pass --file or --out-dir");
    }

    dispatch(
        encoded,
        protocol,
        network,
        usb,
        serial,
        bluetooth,
    )
}

#[allow(clippy::type_complexity)]
fn encode_all<B: RenderBackend>(
    backend: &B,
    registry: &Registry,
    labels: &[lbl::pipeline::AuthoringLabel],
    opts: &PipelineOptions,
    extension: &str,
    want_trace: bool,
) -> Result<(Vec<(String, Vec<u8>)>, Vec<lbl::debug::LabelTrace>)> {
    let mut out = Vec::new();
    let mut traces = Vec::new();
    for label in labels {
        let trace = lbl::encode_label_traced(backend, registry, label.index, &label.html, opts)
            .with_context(|| format!("encoding label {}", label.index))?;
        out.push((
            format!("label-{:04}.{extension}", label.index),
            trace.encoded.clone(),
        ));
        if want_trace {
            traces.push(trace);
        }
    }
    Ok((out, traces))
}

fn dispatch(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    network: Option<String>,
    usb: Option<String>,
    serial: Option<String>,
    bluetooth: Option<String>,
) -> Result<()> {
    if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("network target must be host:port"))?;
        let mut t = lbl_device::NetworkTransport::new(host, port.parse()?);
        dispatch_with(encoded, protocol, &mut t)
    } else if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow!("usb target must be vid:pid"))?;
        let mut t = lbl_device::UsbTransport::new(
            u16::from_str_radix(vid, 16)?,
            u16::from_str_radix(pid, 16)?,
            None,
        );
        dispatch_with(encoded, protocol, &mut t)
    } else if let Some(target) = serial {
        let (path, baud) = lbl::dispatch::parse_serial_target(&target);
        let mut t = lbl_device::SerialTransport::new(path, baud);
        dispatch_with(encoded, protocol, &mut t)
    } else if let Some(target) = bluetooth {
        dispatch_bluetooth(encoded, protocol, target)
    } else {
        bail!("no target; pass --network, --usb, --serial, --bluetooth, --file, or --out-dir");
    }
}

/// Dispatch over Bluetooth LE. Available only when built with the `ble`
/// feature; otherwise it explains how to enable it.
#[cfg(feature = "ble")]
fn dispatch_bluetooth(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    target: String,
) -> Result<()> {
    let mut t = lbl_device::BleTransport::new(target);
    dispatch_with(encoded, protocol, &mut t)
}

#[cfg(not(feature = "ble"))]
fn dispatch_bluetooth(
    _encoded: Vec<(String, Vec<u8>)>,
    _protocol: Protocol,
    _target: String,
) -> Result<()> {
    bail!(
        "Bluetooth LE support is not compiled in; rebuild with `--features ble` \
         (e.g. `cargo install --path crates/lbl --features ble`)"
    )
}

fn dispatch_with<T: lbl_device::Transport>(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    transport: &mut T,
) -> Result<()> {
    let report = lbl::dispatch::dispatch_encoded(encoded, protocol, transport);
    println!(
        "completed={} remaining={} disconnected={}",
        report.completed, report.remaining, report.disconnected
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// preview
// ---------------------------------------------------------------------------

#[derive(Args)]
struct PreviewArgs {
    #[command(flatten)]
    source: SourceArgs,
    #[command(flatten)]
    style: StyleArgs,

    /// Output directory for preview HTML (and PNGs with --render).
    #[arg(long)]
    out_dir: std::path::PathBuf,

    /// Serve JS libraries from this base instead of the CDN.
    #[arg(long)]
    assets_base: Option<String>,

    /// Also rasterize each preview to a PNG (requires a browser).
    #[arg(long)]
    render: bool,

    #[command(flatten)]
    media: MediaArgs,
}

/// Supersample factor used when rasterizing previews to PNG.
const PREVIEW_SUPERSAMPLE: u32 = 2;

fn run_preview(args: PreviewArgs) -> Result<()> {
    let labels = authoring_labels(read_source(&args.source)?)?;
    let count = labels.len();
    std::fs::create_dir_all(&args.out_dir)?;

    let assets_base = args
        .assets_base
        .clone()
        .map(AssetsBase::Local)
        .unwrap_or(AssetsBase::Cdn);

    // Preview rasterization uses a fixed supersample factor (see below); resolve
    // the physical style sizes against it so previews match print sizing.
    let style_cfg = args.style.resolve();
    let style = resolve_style(&style_cfg, args.media.dpi, PREVIEW_SUPERSAMPLE);
    let catalog = Catalog::bundled()?;
    let media = resolve_media(
        &catalog,
        args.media.media.as_deref(),
        args.media.width_mm.or(Some(50.0)),
        args.media.length_mm,
        args.media.dpi,
    )?;
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(args.style.fit_scale(&style_cfg));
    let viewport = render_viewport_px(&media, PREVIEW_SUPERSAMPLE, Rotation::None);

    let backend = if args.render {
        Some(ChromiumBackend::launch()?)
    } else {
        None
    };

    let mut manifest = Vec::new();
    for label in &labels {
        let html = transpile(
            &label.html,
            &TranspileOptions {
                mode: OutputMode::Preview,
                assets_base: assets_base.clone(),
                index: Some(label.index),
                count: Some(count),
                style: style.clone(),
                label_fit,
                viewport: Some(viewport.clone()),
                label_align,
                label_valign,
                label_fit_scale,
            },
        );
        let html_name = format!("preview-{:04}.html", label.index);
        std::fs::write(args.out_dir.join(&html_name), &html)?;

        let mut entry = serde_json::json!({"index": label.index, "html": html_name});

        if let Some(backend) = &backend {
            let req = lbl_render::RenderRequest {
                width_dots: Some(media.width_dots().0),
                height_dots: media.length_dots().map(|d| d.0),
                supersample: PREVIEW_SUPERSAMPLE,
            };
            let img = lbl_render::render_two_pass(backend, &html, &req)?;
            let png_name = format!("preview-{:04}.png", label.index);
            img.save(args.out_dir.join(&png_name))?;
            entry["png"] = serde_json::json!(png_name);
        }

        manifest.push(entry);
    }

    std::fs::write(
        args.out_dir.join("gallery.json"),
        serde_json::to_string_pretty(&serde_json::json!({"count": count, "labels": manifest}))?,
    )?;
    eprintln!(
        "wrote {count} preview label(s) to {}",
        args.out_dir.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// text / transpile / catalog / device
// ---------------------------------------------------------------------------

#[derive(Args)]
struct TextArgs {
    /// Text (joined with spaces). If omitted, read from stdin.
    text: Vec<String>,
    #[arg(long)]
    raw: bool,
    #[arg(long = "qr")]
    qr: Vec<String>,
    #[arg(long = "barcode")]
    barcode: Vec<String>,
    #[arg(long = "image")]
    image: Vec<String>,
    #[arg(long)]
    fragment: bool,
}

fn run_text(args: TextArgs) -> Result<()> {
    let text = if args.text.is_empty() {
        read_stdin_string()?
    } else {
        args.text.join(" ")
    };
    let mut doc = lbl_text::Document::parse(text.trim_end_matches('\n'), args.raw);
    for q in args.qr {
        doc.push_qr(q);
    }
    for b in args.barcode {
        doc.push_barcode(&b);
    }
    for i in args.image {
        doc.push_image(i);
    }
    if args.fragment {
        println!("{}", doc.to_authoring_html());
    } else {
        print!("{}", doc.to_authoring_document());
    }
    Ok(())
}

#[derive(Args)]
struct MarkdownArgs {
    /// Markdown (joined with spaces). If omitted, read from stdin.
    markdown: Vec<String>,
    #[arg(long = "qr")]
    qr: Vec<String>,
    #[arg(long = "barcode")]
    barcode: Vec<String>,
    #[arg(long = "image")]
    image: Vec<String>,
    #[arg(long)]
    fragment: bool,
}

fn run_markdown(args: MarkdownArgs) -> Result<()> {
    let markdown = if args.markdown.is_empty() {
        read_stdin_string()?
    } else {
        args.markdown.join(" ")
    };
    let mut doc = lbl_markdown::MarkdownDocument::parse(&markdown);
    for q in args.qr {
        doc.push_qr(q);
    }
    for b in args.barcode {
        doc.push_barcode(&b);
    }
    for i in args.image {
        doc.push_image(i);
    }
    if args.fragment {
        println!("{}", doc.to_authoring_html());
    } else {
        print!("{}", doc.to_authoring_document());
    }
    Ok(())
}

#[derive(Args)]
struct TranspileArgs {
    /// Authoring HTML file (or stdin).
    input: Option<std::path::PathBuf>,
    #[arg(long, value_enum, default_value = "print")]
    mode: ModeArg,
    #[arg(long)]
    assets_base: Option<String>,

    /// Base text size, in pixels.
    #[arg(long)]
    font_size_px: Option<f64>,
    /// QR code edge length, in pixels.
    #[arg(long)]
    qr_size_px: Option<f64>,
    /// QR error-correction level: L/M/Q/H or low/medium/quartile/high.
    #[arg(long = "qr-ec", alias = "qr-error-correction")]
    qr_error_correction: Option<String>,
    /// QR quiet zone, in modules; 0 = none.
    #[arg(long)]
    qr_margin: Option<u32>,
    /// QR dark module color, hex.
    #[arg(long)]
    qr_dark: Option<String>,
    /// QR light module color, hex.
    #[arg(long)]
    qr_light: Option<String>,
    /// Barcode bar height, in pixels.
    #[arg(long)]
    barcode_height_px: Option<f64>,
    /// Barcode narrowest-bar width, in pixels.
    #[arg(long)]
    barcode_module_px: Option<f64>,
    /// Inner padding between the label edge and its content, in pixels.
    #[arg(long)]
    padding_px: Option<f64>,
    /// Border drawn around the label, in pixels (0 = none).
    #[arg(long)]
    border_px: Option<f64>,
}

#[derive(Clone, Copy, ValueEnum)]
enum ModeArg {
    Print,
    Preview,
}

fn run_transpile(args: TranspileArgs) -> Result<()> {
    let input = match &args.input {
        Some(p) => std::fs::read_to_string(p)?,
        None => read_stdin_string()?,
    };
    let mode = match args.mode {
        ModeArg::Print => OutputMode::Print,
        ModeArg::Preview => OutputMode::Preview,
    };
    let mut style = lbl_transpile_html::LabelStyle::default();
    if let Some(v) = args.font_size_px {
        style.font_size_px = v;
    }
    if let Some(v) = args.qr_size_px {
        style.qr_size_px = v;
    }
    if let Some(v) = &args.qr_error_correction {
        style.qr_error_correction =
            lbl_transpile_html::QrErrorCorrection::parse(v).unwrap_or_default();
    }
    if let Some(v) = args.qr_margin {
        style.qr_margin = v;
    }
    if let Some(v) = &args.qr_dark {
        style.qr_dark = v.clone();
    }
    if let Some(v) = &args.qr_light {
        style.qr_light = v.clone();
    }
    if let Some(v) = args.barcode_height_px {
        style.barcode_height_px = v;
    }
    if let Some(v) = args.barcode_module_px {
        style.barcode_module_width_px = v;
    }
    if let Some(v) = args.padding_px {
        style.padding_px = v;
    }
    if let Some(v) = args.border_px {
        style.border_width_px = v;
    }
    let opts = TranspileOptions {
        mode,
        assets_base: args
            .assets_base
            .map(AssetsBase::Local)
            .unwrap_or(AssetsBase::Cdn),
        index: None,
        count: None,
        style,
        label_fit: LabelFit::Content,
        viewport: None,
        label_align: LabelAlign::default(),
        label_valign: LabelValign::default(),
        label_fit_scale: 1.0,
    };
    print!("{}", transpile(&input, &opts));
    Ok(())
}

#[derive(Args)]
struct CatalogArgs {
    #[command(subcommand)]
    command: CatalogCommand,
}

#[derive(Subcommand)]
enum CatalogCommand {
    List,
    Show {
        key: String,
    },
    Compatible {
        #[arg(long)]
        printer: String,
    },
    Search {
        query: String,
    },
    Printers {
        #[command(subcommand)]
        command: CatalogPrinterCommand,
    },
}

#[derive(Subcommand)]
enum CatalogPrinterCommand {
    List,
    Show {
        key: String,
    },
}

fn run_catalog(args: CatalogArgs) -> Result<()> {
    let catalog = Catalog::bundled()?;
    match args.command {
        CatalogCommand::List => {
            for e in catalog.entries() {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
        CatalogCommand::Show { key } => {
            let e = catalog
                .lookup(&key)
                .ok_or_else(|| anyhow!("no entry for '{key}'"))?;
            println!("{}", serde_json::to_string_pretty(e)?);
        }
        CatalogCommand::Compatible { printer } => {
            for e in catalog.compatible_with(&printer) {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
        CatalogCommand::Search { query } => {
            for e in catalog.search(&query) {
                println!("media  {:<12} {}", e.canonical_key(), e.name);
            }
            for p in catalog.search_printers(&query) {
                println!("printer {:<20} {}", p.canonical_key(), p.name);
            }
        }
        CatalogCommand::Printers { command } => match command {
            CatalogPrinterCommand::List => {
                for p in catalog.printers() {
                    println!("{:<20} {}", p.canonical_key(), p.name);
                }
            }
            CatalogPrinterCommand::Show { key } => {
                let p = catalog
                    .lookup_printer(&key)
                    .or_else(|| catalog.match_printer(&key))
                    .ok_or_else(|| anyhow!("no printer entry for '{key}'"))?;
                println!("{}", serde_json::to_string_pretty(p)?);
            }
        },
    }
    Ok(())
}

#[derive(Args)]
struct DeviceArgs {
    #[command(subcommand)]
    command: DeviceCommand,
}

#[derive(Subcommand)]
enum DeviceCommand {
    List,
}

fn run_device(args: DeviceArgs) -> Result<()> {
    match args.command {
        DeviceCommand::List => {
            let printers = lbl_device::discover();
            println!("{}", serde_json::to_string_pretty(&printers)?);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn read_source(args: &SourceArgs) -> Result<Source> {
    if let Some(text) = &args.text {
        return Ok(Source::Text {
            text: text.clone(),
            raw: args.raw,
        });
    }
    if let Some(markdown) = &args.markdown {
        let content = if markdown == "-" {
            read_stdin_string()?
        } else if std::path::Path::new(markdown).is_file() {
            std::fs::read_to_string(markdown)?
        } else {
            markdown.clone()
        };
        return Ok(Source::Markdown(content));
    }
    if let Some(html) = &args.html {
        let content = if html == "-" {
            read_stdin_string()?
        } else {
            std::fs::read_to_string(html)?
        };
        return Ok(Source::Html(content));
    }
    if let Some(template) = &args.template {
        let template_src = std::fs::read_to_string(template)?;
        let data = match &args.data {
            Some(src) => Some(load_data(src)?),
            None => None,
        };
        return Ok(Source::Template {
            template: template_src,
            data,
            each: args.each.clone(),
        });
    }
    bail!("no input; pass --text, --markdown, --html, or --template")
}

fn load_data(src: &str) -> Result<serde_json::Value> {
    let text = if src.starts_with("http://") || src.starts_with("https://") {
        bail!("URL data sources are supported via the lbl-template binary");
    } else {
        std::fs::read_to_string(src)?
    };
    lbl_template::data::parse_auto(&text).map_err(|e| anyhow!("{e}"))
}

fn read_stdin_string() -> Result<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Print(a) => run_print(a),
        Command::Preview(a) => run_preview(a),
        Command::Text(a) => run_text(a),
        Command::Markdown(a) => run_markdown(a),
        Command::Transpile(a) => run_transpile(a),
        Command::Catalog(a) => run_catalog(a),
        Command::Device(a) => run_device(a),
    }
}
