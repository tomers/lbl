//! `lbl` — the orchestrator for the label pipeline.
//!
//! High-level flows (`print`, `preview`) chain the stages together; individual
//! stage subcommands (`text`, `transpile`, `render`, `dither`, `encode`,
//! `catalog`, `device`) expose each step, mirroring the standalone `lbl-*`
//! binaries.

use std::io::Read;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lbl::pipeline::{authoring_labels, encode_label, resolve_media, PipelineOptions, Source};
use lbl_catalog::Catalog;
use lbl_core::job::OutputMode;
use lbl_core::printer::Protocol;
use lbl_dither::Algorithm;
use lbl_encode::Registry;
use lbl_render::{ChromiumBackend, RenderBackend, SidecarBackend};
use lbl_transpile_html::{transpile, AssetsBase, TranspileOptions};

#[derive(Parser)]
#[command(
    name = "lbl",
    version,
    about = "Orchestrate the lbl label-printing pipeline"
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

    /// Device resolution in DPI.
    #[arg(long, default_value_t = 300.0)]
    dpi: f64,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProtocolArg {
    Dymo,
    #[value(name = "dymo-lw", alias = "lw550")]
    DymoLw,
    Escpos,
    Zpl,
    Tspl,
}

impl From<ProtocolArg> for Protocol {
    fn from(p: ProtocolArg) -> Self {
        match p {
            ProtocolArg::Dymo => Protocol::Dymo,
            ProtocolArg::DymoLw => Protocol::DymoLw,
            ProtocolArg::Escpos => Protocol::EscPos,
            ProtocolArg::Zpl => Protocol::Zpl,
            ProtocolArg::Tspl => Protocol::Tspl,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Chromium,
    Sidecar,
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

    /// Target protocol.
    #[arg(long, value_enum)]
    protocol: ProtocolArg,

    /// Supersample factor for rendering.
    #[arg(long, default_value_t = 3)]
    supersample: u32,

    /// Dithering algorithm.
    #[arg(long, default_value = "auto")]
    dither: String,

    /// Request a cut after each label.
    #[arg(long)]
    cut: bool,

    /// Mark target printer as cut-capable.
    #[arg(long)]
    supports_cut: bool,

    /// Copies per label.
    #[arg(long, default_value_t = 1)]
    copies: u32,

    /// Rendering backend.
    #[arg(long, value_enum, default_value = "chromium")]
    backend: BackendArg,

    /// Network target host:port.
    #[arg(long)]
    network: Option<String>,

    /// USB target vid:pid (hex).
    #[arg(long)]
    usb: Option<String>,

    /// Instead of printing, write encoded bytes to this directory.
    #[arg(long)]
    out_dir: Option<std::path::PathBuf>,
}

fn run_print(args: PrintArgs) -> Result<()> {
    let catalog = Catalog::bundled()?;
    let media = resolve_media(
        &catalog,
        args.media.media.as_deref(),
        args.media.width_mm,
        args.media.length_mm,
        args.media.dpi,
    )?;
    let opts = PipelineOptions {
        protocol: args.protocol.into(),
        media,
        supports_cut: args.supports_cut,
        cut: args.cut,
        copies: args.copies,
        dither: Algorithm::parse(&args.dither)?,
        supersample: args.supersample,
        assets_base: AssetsBase::Cdn,
    };

    let labels = authoring_labels(read_source(&args.source)?)?;
    let registry = Registry::with_builtin_drivers();

    // Encode every label, then dispatch.
    let encoded: Vec<(String, Vec<u8>)> = match args.backend {
        BackendArg::Chromium => {
            let backend = ChromiumBackend::launch()?;
            encode_all(&backend, &registry, &labels, &opts)?
        }
        BackendArg::Sidecar => {
            let backend = SidecarBackend::node_default();
            encode_all(&backend, &registry, &labels, &opts)?
        }
    };

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

    dispatch(encoded, args.network, args.usb)
}

fn encode_all<B: RenderBackend>(
    backend: &B,
    registry: &Registry,
    labels: &[lbl::pipeline::AuthoringLabel],
    opts: &PipelineOptions,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for label in labels {
        let bytes = encode_label(backend, registry, &label.html, opts)
            .with_context(|| format!("encoding label {}", label.index))?;
        out.push((format!("label-{:04}.bin", label.index), bytes));
    }
    Ok(out)
}

fn dispatch(
    encoded: Vec<(String, Vec<u8>)>,
    network: Option<String>,
    usb: Option<String>,
) -> Result<()> {
    use lbl_spool::Spooler;
    let mut spool = Spooler::new();
    for (name, bytes) in encoded {
        spool.enqueue(name, bytes, None);
    }

    let report = if let Some(target) = network {
        let (host, port) = target
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("network target must be host:port"))?;
        let mut t = lbl_device::NetworkTransport::new(host, port.parse()?);
        spool.run(&mut t)
    } else if let Some(target) = usb {
        let (vid, pid) = target
            .split_once(':')
            .ok_or_else(|| anyhow!("usb target must be vid:pid"))?;
        let mut t = lbl_device::UsbTransport::new(
            u16::from_str_radix(vid, 16)?,
            u16::from_str_radix(pid, 16)?,
            None,
        );
        spool.run(&mut t)
    } else {
        bail!("no target; pass --network, --usb, or --out-dir");
    };

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

fn run_preview(args: PreviewArgs) -> Result<()> {
    let labels = authoring_labels(read_source(&args.source)?)?;
    let count = labels.len();
    std::fs::create_dir_all(&args.out_dir)?;

    let assets_base = args
        .assets_base
        .clone()
        .map(AssetsBase::Local)
        .unwrap_or(AssetsBase::Cdn);

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
            },
        );
        let html_name = format!("preview-{:04}.html", label.index);
        std::fs::write(args.out_dir.join(&html_name), &html)?;

        let mut entry = serde_json::json!({"index": label.index, "html": html_name});

        if let Some(backend) = &backend {
            let catalog = Catalog::bundled()?;
            let media = resolve_media(
                &catalog,
                args.media.media.as_deref(),
                args.media.width_mm.or(Some(50.0)),
                args.media.length_mm,
                args.media.dpi,
            )?;
            let req = lbl_render::RenderRequest {
                width_dots: media.width_dots().0,
                height_dots: media.length_dots().map(|d| d.0),
                supersample: 2,
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
struct TranspileArgs {
    /// Authoring HTML file (or stdin).
    input: Option<std::path::PathBuf>,
    #[arg(long, value_enum, default_value = "print")]
    mode: ModeArg,
    #[arg(long)]
    assets_base: Option<String>,
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
    let opts = TranspileOptions {
        mode,
        assets_base: args
            .assets_base
            .map(AssetsBase::Local)
            .unwrap_or(AssetsBase::Cdn),
        index: None,
        count: None,
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
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
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
}

fn run_device(args: DeviceArgs) -> Result<()> {
    match args.command {
        DeviceCommand::List => {
            let printers = lbl_device::discover_usb();
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
    bail!("no input; pass --text, --html, or --template")
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
        Command::Transpile(a) => run_transpile(a),
        Command::Catalog(a) => run_catalog(a),
        Command::Device(a) => run_device(a),
    }
}
