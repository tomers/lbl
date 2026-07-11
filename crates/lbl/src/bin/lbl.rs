//! `lbl` — the orchestrator for the label pipeline.
//!
//! High-level flows (`print`, `preview`) chain the stages together; individual
//! stage subcommands (`text`, `transpile`, `render`, `dither`, `encode`,
//! `catalog`, `device`) expose each step, mirroring the standalone `lbl-*`
//! binaries.

use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lbl::job_input;
use lbl::pipeline::{
    authoring_labels, encode_labels, render_viewport_px, resolve_font_fit_scale,
    resolve_label_align, resolve_label_fit, resolve_label_fit_scale, resolve_label_valign,
    resolve_media, resolve_media_inset, resolve_print_transport, resolve_status_target,
    resolve_style, resolve_style_vector, resolve_template_format, EncodeLabelsOptions,
    EncodeLabelsResult, PipelineOptions, Source, VECTOR_CSS_DPI,
};
use lbl::print_stats::{feed_dots_for_trace, LabelFeedDots, PrintRunTimings, PrintSummaryInput};
use lbl_catalog::{encode_capabilities_for, Catalog};
use lbl_config::StyleConfig;
use lbl_core::job::{CutMode, OutputMode};
use lbl_core::printer::Protocol;
use lbl_core::{Orientation, Rotation};
use lbl_dither::Algorithm;
use lbl_driver_file::{MediaType, VirtualExportMode};
use lbl_encode::Registry;
use lbl_pattern::resolve_head_dots;
use lbl_render::{ChromiumBackend, SidecarBackend};
use lbl_transpile_html::{
    parse_fit_scale, transpile, AssetsBase, LabelAlign, LabelFit, LabelFitSetting, LabelValign,
    MediaInsetPx, TranspileOptions,
};

type EncodedPrintBatch = (
    Vec<(String, Vec<u8>)>,
    Vec<lbl::debug::LabelTrace>,
    Vec<LabelFeedDots>,
    Duration,
    usize,
);

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
    Print(Box<PrintArgs>),
    /// Produce browser-ready preview HTML (and optional PNGs) for a gallery.
    Preview(Box<PreviewArgs>),
    /// Convert text/CLI directives into authoring HTML.
    Text(TextArgs),
    /// Convert Markdown (with inline directives) into authoring HTML.
    Markdown(MarkdownArgs),
    /// Transpile authoring HTML into browser-ready HTML.
    Transpile(TranspileArgs),
    /// Browse the media catalog.
    Catalog(CatalogArgs),
    /// Inspect layered configuration (show|sources|paths).
    Config(ConfigArgs),
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

    /// Template file, inline source, or `-` for stdin (rendered with --data).
    #[arg(long, group = "src")]
    template: Option<String>,

    /// Data file or inline JSON/TOML/YAML for --template.
    #[arg(long)]
    data: Option<String>,

    /// JSON-pointer to a batch array within the data.
    #[arg(long)]
    each: Option<String>,

    /// Override how the rendered --template body is interpreted. When omitted,
    /// the format is inferred from the template path extension (`.html`/`.lbl`
    /// → html, `.md` → markdown; otherwise text).
    #[arg(long, value_enum)]
    template_format: Option<TemplateFormatArg>,
}

#[derive(Clone, Copy, ValueEnum, Default)]
enum TemplateFormatArg {
    #[default]
    Text,
    Markdown,
    Html,
}

impl From<TemplateFormatArg> for lbl::pipeline::TemplateFormat {
    fn from(v: TemplateFormatArg) -> Self {
        match v {
            TemplateFormatArg::Text => Self::Text,
            TemplateFormatArg::Markdown => Self::Markdown,
            TemplateFormatArg::Html => Self::Html,
        }
    }
}

#[derive(Args, Clone, Default)]
struct BatchSelectArgs {
    /// Only include labels whose data fields contain this substring
    /// (case-insensitive). Matches the HTML preview filter.
    #[arg(long)]
    filter: Option<String>,

    /// Print only the first label from the current selection (same as `--take 1`).
    #[arg(long, conflicts_with_all = ["take", "last"])]
    first: bool,

    /// Print only the last label from the current selection.
    #[arg(long, conflicts_with_all = ["take", "first"])]
    last: bool,

    /// Skip the first N labels in the current selection.
    #[arg(long, default_value_t = 0)]
    skip: usize,

    /// Print at most N labels from the current selection.
    #[arg(long, conflicts_with_all = ["first", "last"])]
    take: Option<usize>,

    /// Print only the label at this zero-based batch index. Repeat for multiple
    /// indices (applied before `--filter`, `--skip`, and `--take`).
    #[arg(long = "index")]
    indices: Vec<usize>,
}

impl BatchSelectArgs {
    fn to_selection(&self) -> lbl_template::BatchSelection {
        lbl_template::BatchSelection {
            filter: self.filter.clone(),
            skip: self.skip,
            take: self.take.or(if self.first { Some(1) } else { None }),
            last: self.last,
            indices: if self.indices.is_empty() {
                None
            } else {
                Some(self.indices.clone())
            },
        }
    }
}

impl SourceArgs {
    fn is_set(&self) -> bool {
        self.text.is_some()
            || self.markdown.is_some()
            || self.html.is_some()
            || self.template.is_some()
    }
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

    /// Gap between flex children (text, QR, barcode, …), in mm (overrides config
    /// `style.element_gap_mm`).
    #[arg(long)]
    element_gap_mm: Option<f64>,

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

    /// Auto-fit text scale in fill mode (`0.8`, `80%`, …; overrides config
    /// `style.font_fit_scale`).
    #[arg(long)]
    font_fit_scale: Option<String>,

    /// Inset from the physical media edge, all sides (overrides config
    /// `style.media_inset_mm`).
    #[arg(long)]
    media_inset_mm: Option<f64>,

    /// Inset on both cross-axis sides (left + right in portrait).
    #[arg(long)]
    media_inset_horizontal_mm: Option<f64>,

    /// Inset on both main-axis sides (top + bottom in portrait).
    #[arg(long)]
    media_inset_vertical_mm: Option<f64>,

    /// Main-axis start inset (top in portrait).
    #[arg(long)]
    media_inset_start_mm: Option<f64>,

    /// Main-axis end inset (bottom in portrait).
    #[arg(long)]
    media_inset_end_mm: Option<f64>,

    /// Cross-axis start inset (left in portrait).
    #[arg(long)]
    media_inset_cross_start_mm: Option<f64>,

    /// Cross-axis end inset (right in portrait).
    #[arg(long)]
    media_inset_cross_end_mm: Option<f64>,
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
        if let Some(v) = self.element_gap_mm {
            style.element_gap_mm = v;
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
        if let Some(v) = self.media_inset_mm {
            style.media_inset_mm = v;
        }
        if let Some(v) = self.media_inset_horizontal_mm {
            style.media_inset_horizontal_mm = Some(v);
        }
        if let Some(v) = self.media_inset_vertical_mm {
            style.media_inset_vertical_mm = Some(v);
        }
        if let Some(v) = self.media_inset_start_mm {
            style.media_inset_start_mm = Some(v);
        }
        if let Some(v) = self.media_inset_end_mm {
            style.media_inset_end_mm = Some(v);
        }
        if let Some(v) = self.media_inset_cross_start_mm {
            style.media_inset_cross_start_mm = Some(v);
        }
        if let Some(v) = self.media_inset_cross_end_mm {
            style.media_inset_cross_end_mm = Some(v);
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

    fn font_scale(&self, style: &StyleConfig) -> f64 {
        if let Some(raw) = &self.font_fit_scale {
            parse_fit_scale(raw).unwrap_or(style.font_fit_scale)
        } else {
            style.font_fit_scale
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum ProtocolArg {
    Dymo,
    #[value(name = "dymo-lw", alias = "lw550")]
    DymoLw,
    #[value(name = "dymo-lw-classic", aliases = ["dymolwclassic", "lw450"])]
    DymoLwClassic,
    Escpos,
    #[value(name = "phomemo", alias = "m02")]
    Phomemo,
    #[value(name = "phomemo-m02x", aliases = ["phomemom02x", "m02x"])]
    PhomemoM02x,
    #[value(name = "phomemo-m110", aliases = ["phomemom110", "m110"])]
    PhomemoM110,
    #[value(name = "phomemo-d30", aliases = ["phomemod30", "d30", "q30"])]
    PhomemoD30,
    Zpl,
    /// Epson ESC/Label (ColorWorks).
    #[value(name = "esc-label", aliases = ["esclabel", "colorworks"])]
    EscLabel,
    Tspl,
    /// NIIMBOT thermal label printers (D11 / D110 family).
    Niimbot,
    /// Brother QL-series raster printers (QL-820NWB(c), …).
    #[value(name = "brother-ql", alias = "brotherql")]
    BrotherQl,
    /// Brother P-touch / TZe tape printers (PT-P700, …).
    #[value(name = "brother-pt", aliases = ["brotherpt", "pt", "tze"])]
    BrotherPt,
    /// Virtual printer: render to an image file instead of hardware.
    #[value(alias = "file")]
    Virtual,
    /// Console printer: render the raster to the terminal as text.
    #[value(alias = "term")]
    Console,
    /// HTML preview: write a browser gallery of full-resolution PNGs.
    Html,
}

impl From<ProtocolArg> for Protocol {
    fn from(p: ProtocolArg) -> Self {
        match p {
            ProtocolArg::Dymo => Protocol::Dymo,
            ProtocolArg::DymoLw => Protocol::DymoLw,
            ProtocolArg::DymoLwClassic => Protocol::DymoLwClassic,
            ProtocolArg::Escpos => Protocol::EscPos,
            ProtocolArg::Phomemo => Protocol::Phomemo,
            ProtocolArg::PhomemoM02x => Protocol::PhomemoM02x,
            ProtocolArg::PhomemoM110 => Protocol::PhomemoM110,
            ProtocolArg::PhomemoD30 => Protocol::PhomemoD30,
            ProtocolArg::Zpl => Protocol::Zpl,
            ProtocolArg::EscLabel => Protocol::EscLabel,
            ProtocolArg::Tspl => Protocol::Tspl,
            ProtocolArg::Niimbot => Protocol::Niimbot,
            ProtocolArg::BrotherQl => Protocol::BrotherQl,
            ProtocolArg::BrotherPt => Protocol::BrotherPt,
            ProtocolArg::Virtual => Protocol::Virtual,
            ProtocolArg::Console => Protocol::Console,
            ProtocolArg::Html => Protocol::Html,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum NiimbotTaskArg {
    /// Legacy D110 / B-series (PageStart + separate PrintQuantity).
    Standard,
    /// D110M V4 (9-byte PrintStart, 13-byte SetPageSize, no PageStart).
    V4,
    /// B1 / B21 (protocol 3: 7-byte PrintStart, 6-byte SetPageSize, total-mode rows).
    B1,
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
    batch: BatchSelectArgs,
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
    /// png | bmp | tiff | gif | pbm. Defaults to png. Ignored in vector export
    /// mode (PDF is always emitted).
    #[arg(long)]
    media_type: Option<String>,

    /// For `--protocol virtual`: how labels are stored — `raster` (dithered
    /// bitmap, emulates a printed label) or `vector` (PDF with vector text,
    /// barcodes, and QR codes). Overrides config `print.export_mode`.
    #[arg(long, value_name = "MODE")]
    export_mode: Option<String>,

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

    /// When to cut: `none`, `every` (after each label), or `end` (after last).
    /// Overrides config `[print] cut_mode` / `LBL_PRINT__CUT_MODE`. The legacy
    /// `--cut` flag is an alias for `--cut-mode every`.
    #[arg(long, value_name = "MODE")]
    cut_mode: Option<String>,

    /// Request a cut after each label (alias for `--cut-mode every`).
    #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "cut_mode")]
    cut: bool,

    /// Mark target printer as cut-capable. Overrides config `[print]
    /// supports_cut` / `LBL_PRINT__SUPPORTS_CUT`.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    supports_cut: Option<bool>,

    /// Copies per label. Overrides config `[print] copies` /
    /// `LBL_PRINT__COPIES`.
    #[arg(long)]
    copies: Option<u32>,

    /// Print density / heat level (driver-specific; typically 1–5).
    /// Overrides config `[print] density` / `LBL_PRINT__DENSITY`.
    #[arg(long)]
    density: Option<u8>,

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
    /// label and ask for confirmation. Ignored for `--protocol console` and
    /// `--protocol html`. Overrides config `[print] confirm` /
    /// `LBL_PRINT__CONFIRM`.
    #[arg(long, action = clap::ArgAction::SetTrue, conflicts_with = "preview")]
    confirm: Option<bool>,

    /// Show a terminal preview of each label and stop without printing. Same
    /// art as `--confirm`, but no confirmation prompt. Ignored for
    /// `--protocol console` and `--protocol html`.
    #[arg(long, conflicts_with = "confirm")]
    preview: bool,

    /// Serve the HTML preview on loopback and open it in the system browser.
    /// Requires `--protocol html`. The process stays running until Ctrl+C.
    #[arg(long)]
    open_browser: bool,

    /// Print a calibration sample pattern. Omit the value to use the resolved
    /// media width in device dots (`--media` / `--width-mm` at `--dpi`); pass a
    /// number to override the head height (e.g. 64 on a 64-dot DYMO head).
    /// Skips label input, rendering, and dithering.
    #[arg(long, num_args = 0..=1)]
    sample_pattern: Option<Option<u32>>,
}

fn config_enum<E: ValueEnum>(name: &str, value: &str) -> Result<E> {
    E::from_str(value, true).map_err(|e| anyhow!("invalid config {name}: {e}"))
}

fn run_print(args: PrintArgs) -> Result<()> {
    let loader = lbl_config::Loader::new();
    let config = loader
        .load()
        .unwrap_or_else(|_| lbl_config::Config::default());
    let print_cfg = &config.print;

    let catalog = Catalog::bundled()?;
    let printer_entry = match args.printer.as_deref() {
        Some(key) => Some(catalog.require_printer(key).map_err(|e| anyhow!(e))?),
        None => None,
    };

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
    let cut_mode = if args.cut {
        CutMode::Every
    } else if let Some(mode) = args.cut_mode.as_deref() {
        CutMode::parse(mode)
            .ok_or_else(|| anyhow!("unknown cut mode '{mode}' (expected none|every|end)"))?
    } else {
        CutMode::parse(&print_cfg.cut_mode).unwrap_or(CutMode::None)
    };
    let supports_cut = args.supports_cut.unwrap_or(print_cfg.supports_cut);
    let copies = args.copies.unwrap_or(print_cfg.copies);
    let density = args.density.or(print_cfg.density);
    let dither = args.dither.unwrap_or_else(|| print_cfg.dither.clone());
    let backend = match args.backend {
        Some(b) => b,
        None => config_enum::<BackendArg>("print.backend", &print_cfg.backend)?,
    };
    let niimbot_task = match args.niimbot_task {
        Some(t) => t,
        None => {
            if let Some(key) = args
                .printer
                .as_deref()
                .or_else(|| printer_entry.map(|p| p.canonical_key()))
            {
                if let Some(task) = lbl_driver_niimbot::NiimbotDriver::task_for_printer_key(key) {
                    match task {
                        lbl_driver_niimbot::NiimbotTask::Standard => NiimbotTaskArg::Standard,
                        lbl_driver_niimbot::NiimbotTask::V4 => NiimbotTaskArg::V4,
                        lbl_driver_niimbot::NiimbotTask::B1 => NiimbotTaskArg::B1,
                    }
                } else {
                    config_enum::<NiimbotTaskArg>("print.niimbot_task", &print_cfg.niimbot_task)?
                }
            } else {
                config_enum::<NiimbotTaskArg>("print.niimbot_task", &print_cfg.niimbot_task)?
            }
        }
    };
    let (network, usb, serial, bluetooth) = resolve_print_transport(
        printer_entry,
        args.network.or_else(|| print_cfg.network.clone()),
        args.usb.or_else(|| print_cfg.usb.clone()),
        args.serial.or_else(|| print_cfg.serial.clone()),
        args.bluetooth.or_else(|| print_cfg.bluetooth.clone()),
    )?;
    let media_type_name = args.media_type.or_else(|| print_cfg.media_type.clone());
    let export_mode_name = args.export_mode.or_else(|| print_cfg.export_mode.clone());

    let dpi = catalog.resolve_dpi(args.printer.as_deref(), protocol, args.media.dpi);
    let media_sku = args
        .media
        .media
        .clone()
        .or_else(|| printer_entry.and_then(|p| p.default_media.clone()));
    let media_entry = media_sku.as_deref().and_then(|key| catalog.lookup(key));
    if let (Some(printer), Some(entry)) = (printer_entry, media_entry) {
        if !catalog.supports_media(printer.canonical_key(), entry.canonical_key()) {
            bail!(
                "media '{}' is not supported by printer '{}'",
                entry.canonical_key(),
                printer.canonical_key()
            );
        }
    }
    lbl::terminal::print_catalog_resolution_hints(
        lbl::terminal::CatalogEntryHint {
            input: args.printer.as_deref(),
            name: printer_entry.map(|p| p.name.as_str()),
            key: printer_entry.map(|p| p.canonical_key()),
        },
        lbl::terminal::CatalogEntryHint {
            input: media_sku.as_deref(),
            name: media_entry.map(|e| e.name.as_str()),
            key: media_entry.map(|e| e.canonical_key()),
        },
    )?;
    let media = resolve_media(
        &catalog,
        media_sku.as_deref(),
        args.media.width_mm,
        args.media.length_mm,
        dpi,
    )?;

    // The virtual printer's export mode and output file format.
    let virtual_export_mode = if protocol == Protocol::Virtual {
        match &export_mode_name {
            Some(name) => VirtualExportMode::parse(name).map_err(|e| anyhow!(e))?,
            None => VirtualExportMode::Raster,
        }
    } else {
        if export_mode_name.is_some() {
            bail!("--export-mode only applies to --protocol virtual");
        }
        VirtualExportMode::Raster
    };

    let media_type = if protocol == Protocol::Virtual {
        if virtual_export_mode == VirtualExportMode::Vector {
            if media_type_name.is_some() {
                bail!("--media-type is ignored in vector export mode (output is always PDF)");
            }
            Some(MediaType::Pdf)
        } else {
            Some(match &media_type_name {
                Some(name) => MediaType::parse(name).map_err(|e| anyhow!(e))?,
                None => MediaType::Png,
            })
        }
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
    let rotation = Rotation::for_print_with_media(
        orientation,
        &media,
        args.rotate_cw as u32,
        args.rotate_ccw as u32,
    );
    let head_rotation = Rotation::for_head_with_media(
        orientation,
        &media,
        args.rotate_cw as u32,
        args.rotate_ccw as u32,
        protocol,
    );

    let style_cfg = args.style.resolve();
    let (style, media_inset) = if virtual_export_mode == VirtualExportMode::Vector {
        (
            resolve_style_vector(&style_cfg),
            resolve_media_inset(&style_cfg).to_px(VECTOR_CSS_DPI, 1),
        )
    } else {
        (
            resolve_style(&style_cfg, media.dpi.0, supersample),
            resolve_media_inset(&style_cfg).to_px(media.dpi.0, supersample),
        )
    };
    let label_fit = resolve_label_fit(
        LabelFitSetting::parse(&style_cfg.label_fit).unwrap_or(LabelFitSetting::Auto),
        &media,
    );
    let label_align = resolve_label_align(&style_cfg.label_align);
    let label_valign = resolve_label_valign(&style_cfg.label_valign);
    let label_fit_scale = resolve_label_fit_scale(args.style.fit_scale(&style_cfg));
    let font_fit_scale = resolve_font_fit_scale(args.style.font_scale(&style_cfg));

    let preview_media = media.clone();

    let sample_head_dots = if args.sample_pattern.is_some() {
        Some(resolve_head_dots(args.sample_pattern.flatten(), &media).map_err(|e| anyhow!(e))?)
    } else {
        None
    };

    let efficiency_warn_below = render_cfg.efficiency_warn_below;

    let encode_caps = encode_capabilities_for(printer_entry, &media, supports_cut);

    let opts = PipelineOptions {
        protocol,
        media,
        supports_cut,
        cut_mode,
        copies,
        density,
        dither: Algorithm::parse(&dither)?,
        rotation,
        head_rotation,
        supersample,
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

    if args.open_browser && protocol != Protocol::Html {
        bail!("--open-browser requires --protocol html");
    }

    if protocol == Protocol::Html && args.sample_pattern.is_some() {
        bail!("--sample-pattern is not supported with --protocol html");
    }

    if args.source.is_set() && args.sample_pattern.is_some() {
        bail!("--sample-pattern cannot be combined with label input (--text, --markdown, --html, --template)");
    }

    // The virtual driver carries its selected media type, so register an
    // instance configured for this run (overriding the PNG default).
    let mut registry = Registry::with_builtin_drivers();
    if let Some(mt) = media_type.filter(|_| virtual_export_mode == VirtualExportMode::Raster) {
        registry.register(Box::new(lbl_driver_file::FileDriver::new(mt)));
    }
    if protocol == Protocol::Niimbot && niimbot_task == NiimbotTaskArg::V4 {
        registry.register(Box::new(lbl_driver_niimbot::NiimbotDriver::v4()));
    }
    if protocol == Protocol::Niimbot && niimbot_task == NiimbotTaskArg::B1 {
        registry.register(Box::new(lbl_driver_niimbot::NiimbotDriver::b1()));
    }

    let extension = if protocol == Protocol::Console {
        "txt"
    } else {
        media_type.map(|mt| mt.extension()).unwrap_or("bin")
    };

    let want_trace = args.debug_html.is_some()
        || debug
        || confirm
        || args.preview
        || protocol == Protocol::Console
        || protocol == Protocol::Html;

    let (encoded, traces, feed_dots, preprocess_duration, label_count): EncodedPrintBatch =
        if let Some(head_dots) = sample_head_dots {
            let started = Instant::now();
            let trace = lbl::encode_sample_pattern_traced(&registry, 0, head_dots, &opts)
                .context("encoding sample pattern")?;
            let preprocess_duration = started.elapsed();
            let feed_dots = vec![LabelFeedDots(feed_dots_for_trace(&trace, protocol))];
            let encoded = vec![(
                format!("sample-pattern-{head_dots:04}.{extension}"),
                trace.encoded.clone(),
            )];
            let traces = if want_trace { vec![trace] } else { vec![] };
            (encoded, traces, feed_dots, preprocess_duration, 1)
        } else {
            let labels = authoring_labels(read_source(&args.source)?, &args.batch.to_selection())?;
            let label_count = labels.len();
            let encode_opts = EncodeLabelsOptions {
                extension,
                want_trace,
                warn_preprocess: true,
                sidecar_backend: matches!(backend, BackendArg::Sidecar),
            };
            let EncodeLabelsResult {
                encoded,
                traces,
                feed_dots,
                preprocess_duration,
            } = match &backend {
                BackendArg::Chromium => {
                    let backend = ChromiumBackend::launch()?;
                    encode_labels(&backend, &registry, &labels, &opts, encode_opts)?
                }
                BackendArg::Sidecar => {
                    let backend = SidecarBackend::node_default();
                    encode_labels(&backend, &registry, &labels, &opts, encode_opts)?
                }
            };
            (encoded, traces, feed_dots, preprocess_duration, label_count)
        };

    let preprocess_input = job_input(
        label_count,
        &opts.media,
        opts.rotation,
        opts.supersample,
        matches!(backend, BackendArg::Sidecar),
    );

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
        lbl::terminal::dump_config_report(&loader)?;
        lbl::terminal::dump_debug(&traces)?;
    }

    // Console "printer": render the dithered raster to the terminal. Unless a
    // file target is given, this writes to stdout (in color when it is a TTY)
    // rather than to a device, and never needs confirmation.
    if protocol == Protocol::Console && args.out_dir.is_none() && args.file.is_none() {
        lbl::terminal::dump_rasters(&traces)?;
        return Ok(());
    }

    // HTML preview: write a gallery page plus full-resolution PNGs.
    if protocol == Protocol::Html {
        let paths = lbl::preview::resolve_html_preview_paths(
            args.out_dir.as_deref(),
            args.file.as_deref(),
        )?;
        let source = read_source(&args.source)?;
        let preview_input = lbl::preview::input_from_run(
            &source,
            lbl::preview::PreviewSourceArgs {
                template_path: args.source.template.as_deref(),
            },
            lbl::preview::PreviewRunContext {
                catalog: &catalog,
                printer_entry,
                printer_key: args.printer.as_deref(),
                protocol,
                dpi,
                media: &preview_media,
                media_sku: media_sku.as_deref(),
                transport: lbl::preview::PreviewTransport {
                    network: &network,
                    usb: &usb,
                    serial: &serial,
                    bluetooth: &bluetooth,
                },
            },
        )?;
        let context = lbl::preview::HtmlPreviewContext::build(preview_input, &traces);
        lbl::preview::write_html_preview(&context, &traces, &paths)?;
        eprintln!(
            "wrote HTML preview ({} label(s)) to {}",
            traces.len(),
            paths.index_html.display()
        );
        if args.open_browser {
            lbl::preview::serve_and_open(&paths.bundle_dir)?;
        } else {
            lbl::preview::print_open_hint(&paths.bundle_dir);
        }
        return Ok(());
    }

    // Preview-only: show terminal art and stop before any device/file output.
    if args.preview && protocol != Protocol::Console && protocol != Protocol::Html {
        lbl::terminal::preview_print(&traces)?;
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
        return dispatch_with(encoded, protocol, &mut t, None, None);
    }

    if protocol == Protocol::Virtual {
        bail!("virtual printer needs an output target; pass --file or --out-dir");
    }

    let summary = PrintSummaryInput {
        timings: PrintRunTimings {
            preprocess: preprocess_duration,
            print: Duration::ZERO,
        },
        label_count,
        copies,
        feed_dots: &feed_dots,
        media: &opts.media,
        rotation: opts.rotation,
        protocol,
        preprocess: &preprocess_input,
        efficiency_warn_below,
    };
    dispatch(
        encoded,
        protocol,
        network,
        usb,
        serial,
        bluetooth,
        Some(summary),
    )
}

fn dispatch(
    encoded: Vec<(String, Vec<u8>)>,
    protocol: Protocol,
    network: Option<String>,
    usb: Option<String>,
    serial: Option<String>,
    bluetooth: Option<String>,
    summary: Option<PrintSummaryInput<'_>>,
) -> Result<()> {
    if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("network target must be host:port"))?;
        let mut t = lbl_device::NetworkTransport::new(host, port.parse()?);
        dispatch_with(encoded, protocol, &mut t, None, summary)
    } else if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow!("usb target must be vid:pid"))?;
        let vendor_id = u16::from_str_radix(vid, 16)?;
        let product_id = u16::from_str_radix(pid, 16)?;
        let target_info = Some(lbl_device::TransportTarget::Usb {
            vendor_id,
            product_id,
        });
        let usb = lbl_device::UsbTransport::new(vendor_id, product_id, None);
        if protocol == Protocol::DymoLw {
            let mut t = lbl_device::DymoLwUsbTransport::new(usb);
            dispatch_with(encoded, protocol, &mut t, target_info, summary)
        } else {
            let mut t = usb;
            dispatch_with(encoded, protocol, &mut t, target_info, summary)
        }
    } else if let Some(target) = serial {
        let (path, baud) = lbl::dispatch::parse_serial_target(&target);
        let mut t = lbl_device::SerialTransport::new(path.clone(), baud);
        dispatch_with(
            encoded,
            protocol,
            &mut t,
            Some(lbl_device::TransportTarget::Serial { path }),
            summary,
        )
    } else if let Some(target) = bluetooth {
        dispatch_bluetooth(encoded, protocol, target, summary)
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
    summary: Option<PrintSummaryInput<'_>>,
) -> Result<()> {
    let mut t = lbl_device::BleTransport::new(target);
    dispatch_with(encoded, protocol, &mut t, None, summary)
}

#[cfg(not(feature = "ble"))]
fn dispatch_bluetooth(
    _encoded: Vec<(String, Vec<u8>)>,
    _protocol: Protocol,
    _target: String,
    _summary: Option<PrintSummaryInput<'_>>,
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
    target: Option<lbl_device::TransportTarget>,
    mut summary: Option<PrintSummaryInput<'_>>,
) -> Result<()> {
    let report = lbl::dispatch::dispatch_encoded(encoded, protocol, transport);
    let print_duration = report.duration;
    if report.disconnected {
        eprintln!(
            "completed={} remaining={} disconnected={}",
            report.completed, report.remaining, report.disconnected
        );
        lbl::dispatch::finish_dispatch(report, target).map_err(anyhow::Error::msg)?;
    } else if summary.is_none() {
        println!(
            "completed={} remaining={} disconnected={}",
            report.completed, report.remaining, report.disconnected
        );
    }
    if let Some(mut input) = summary.take() {
        input.timings = PrintRunTimings {
            preprocess: input.timings.preprocess,
            print: print_duration,
        };
        lbl::terminal::print_run_summary(&input)?;
    }
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
    batch: BatchSelectArgs,
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
    let labels = authoring_labels(read_source(&args.source)?, &args.batch.to_selection())?;
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
    let font_fit_scale = resolve_font_fit_scale(args.style.font_scale(&style_cfg));
    let media_inset = resolve_media_inset(&style_cfg).to_px(args.media.dpi, PREVIEW_SUPERSAMPLE);
    let viewport = render_viewport_px(&media, PREVIEW_SUPERSAMPLE, Rotation::None, None);

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
                font_fit_scale,
                media_inset,
                ..Default::default()
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
        media_inset: MediaInsetPx::default(),
        ..Default::default()
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
    Show { key: String },
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
            let printer = catalog.require_printer(&printer).map_err(|e| anyhow!(e))?;
            for e in catalog.media_for_printer(printer) {
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
                let p = catalog.require_printer(&key).map_err(|e| anyhow!(e))?;
                println!("{}", serde_json::to_string_pretty(p)?);
            }
        },
    }
    Ok(())
}

#[derive(Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the fully-merged effective configuration as JSON.
    Show,
    /// Print where each effective value came from (provenance).
    Sources,
    /// Print the resolved configuration file paths.
    Paths,
}

fn run_config(args: ConfigArgs) -> Result<()> {
    let loader = lbl_config::Loader::new();
    match args.command {
        ConfigCommand::Show => {
            let cfg = loader.load()?;
            let json = serde_json::to_string_pretty(&cfg)?;
            let color = lbl::terminal::stdout_color();
            if color {
                print!("{}", lbl::terminal::highlight_json(&json, true));
            } else {
                print!("{json}");
            }
        }
        ConfigCommand::Sources => {
            for (key, source) in lbl_config::describe_sources(loader.figment()) {
                println!("{key}\t{source}");
            }
        }
        ConfigCommand::Paths => {
            let catalog_extra = loader
                .load()
                .map(|c| c.catalog.extra_paths)
                .unwrap_or_default();
            print!(
                "{}",
                lbl_config::format_paths_report(
                    loader.paths(),
                    &catalog_extra,
                    lbl::terminal::stdout_color(),
                )
            );
        }
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
    /// Query print-engine status when supported by the printer protocol.
    Status(StatusArgs),
}

#[derive(Args)]
struct StatusArgs {
    /// Printer model key from the catalog (e.g. `LabelWriter 550`, `LW550`).
    /// Uses the catalog's default USB target when no `--usb` is set (same as
    /// `lbl print`).
    #[arg(long)]
    printer: Option<String>,

    /// USB target `vid:pid` in hex. Overrides config and catalog defaults.
    #[arg(long)]
    usb: Option<String>,

    /// Printer profile id from `printers.toml` (or `[general] default_printer`).
    #[arg(long)]
    profile: Option<String>,
}

fn run_device(args: DeviceArgs) -> Result<()> {
    match args.command {
        DeviceCommand::List => {
            let printers = lbl_device::discover();
            println!("{}", serde_json::to_string_pretty(&printers)?);
        }
        DeviceCommand::Status(status) => run_device_status(status)?,
    }
    Ok(())
}

fn run_device_status(args: StatusArgs) -> Result<()> {
    let loader = lbl_config::Loader::new();
    let config = loader
        .load()
        .unwrap_or_else(|_| lbl_config::Config::default());
    let catalog = Catalog::bundled()?;

    let target = resolve_status_target(
        &catalog,
        &config,
        args.printer.as_deref(),
        args.profile.as_deref(),
        args.usb,
    )?;

    let (vid, pid) = target
        .usb
        .split_once(':')
        .ok_or_else(|| anyhow!("usb target must be vid:pid (hex)"))?;
    let vendor_id = u16::from_str_radix(vid, 16)?;
    let product_id = u16::from_str_radix(pid, 16)?;
    let transport = lbl_device::UsbTransport::new(vendor_id, product_id, target.serial);
    let status =
        lbl_device::query_print_status(target.protocol, &transport).map_err(|e| anyhow!("{e}"))?;
    println!("{}", serde_json::to_string_pretty(&status)?);
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
        let template_src = if template == "-" {
            read_stdin_string()?
        } else if std::path::Path::new(template).is_file() {
            std::fs::read_to_string(template)?
        } else {
            template.clone()
        };
        let data = match &args.data {
            Some(src) => Some(load_data(src)?),
            None => None,
        };
        return Ok(Source::Template {
            template: template_src,
            data,
            each: args.each.clone(),
            format: resolve_template_format(template, args.template_format.map(Into::into)),
        });
    }
    bail!("no input; pass --text, --markdown, --html, or --template")
}

fn load_data(src: &str) -> Result<serde_json::Value> {
    if src.starts_with("http://") || src.starts_with("https://") {
        bail!("URL data sources are supported via the lbl-template binary");
    }
    let text = if std::path::Path::new(src).is_file() {
        std::fs::read_to_string(src)?
    } else {
        src.to_string()
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
        Command::Print(a) => run_print(*a),
        Command::Preview(a) => run_preview(*a),
        Command::Text(a) => run_text(a),
        Command::Markdown(a) => run_markdown(a),
        Command::Transpile(a) => run_transpile(a),
        Command::Catalog(a) => run_catalog(a),
        Command::Config(a) => run_config(a),
        Command::Device(a) => run_device(a),
    }
}
