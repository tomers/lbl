//! `lbl-encode` — encode a 1-bit PBM bitmap into printer protocol bytes.

use std::io::{Read, Write};

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use lbl_core::bitmap::MonoBitmap;
use lbl_core::job::JobSpec;
use lbl_core::media::Media;
use lbl_core::printer::{PrinterCapabilities, Protocol};
use lbl_core::units::Dpi;
use lbl_driver_api::EncodeContext;
use lbl_encode::Registry;

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
    /// Virtual printer: encode to an image file ("media type" via --media-type).
    #[value(alias = "file")]
    Virtual,
    /// Console printer: encode to terminal art (plain block glyphs).
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

#[derive(Parser)]
#[command(
    name = "lbl-encode",
    about = "Encode a 1-bit PBM into printer protocol bytes",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    /// Input PBM (P4) file. If omitted, read from stdin (unless --sample-pattern).
    input: Option<std::path::PathBuf>,

    /// Generate a calibration sample pattern instead of reading a PBM. Omit the
    /// value to use `--width-mm` at `--dpi`; pass a number to override the head
    /// height in dots. The raster is not dithered or rescaled.
    #[arg(long, num_args = 0..=1, conflicts_with = "input")]
    sample_pattern: Option<Option<u32>>,

    /// Target protocol / driver.
    #[arg(long, value_enum)]
    protocol: ProtocolArg,

    /// For `--protocol virtual`: output image format ("media type"):
    /// png | bmp | tiff | gif | pbm. Defaults to png.
    #[arg(long)]
    media_type: Option<String>,

    /// Media width in millimeters.
    #[arg(long, default_value_t = 25.0)]
    width_mm: f64,

    /// Fixed media length in millimeters (omit for continuous media).
    #[arg(long)]
    length_mm: Option<f64>,

    /// Device resolution in DPI.
    #[arg(long, default_value_t = 300.0)]
    dpi: f64,

    /// Number of copies.
    #[arg(long, default_value_t = 1)]
    copies: u32,

    /// Request a cut after the label.
    #[arg(long)]
    cut: bool,

    /// Mark the target printer as cut-capable (so --cut is honored).
    #[arg(long)]
    supports_cut: bool,

    /// Output file. If omitted, bytes are written to stdout.
    #[arg(long)]
    out: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let dpi = Dpi(cli.dpi);
    let media = match cli.length_mm {
        Some(len) => Media::fixed(cli.width_mm, len, dpi),
        None => Media::continuous(cli.width_mm, dpi),
    };
    let protocol: Protocol = cli.protocol.into();

    let bitmap = if cli.sample_pattern.is_some() {
        let head_dots = lbl_pattern::resolve_head_dots(cli.sample_pattern.flatten(), &media)
            .map_err(|e| anyhow!(e))?;
        lbl_pattern::sample_pattern_for_media(head_dots, &media, protocol)
    } else {
        let pbm = match &cli.input {
            Some(path) => {
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?
            }
            None => {
                let mut buf = Vec::new();
                std::io::stdin().read_to_end(&mut buf)?;
                buf
            }
        };
        MonoBitmap::from_pbm(&pbm).map_err(|e| anyhow::anyhow!("parsing PBM: {e}"))?
    };

    let mut job = JobSpec::new(media.clone());
    job.cut = cli.cut;
    job.copies = cli.copies;

    let caps = PrinterCapabilities {
        dpi,
        max_width_mm: cli.width_mm,
        supports_cut: cli.supports_cut,
        reports_media: false,
    };

    let mut registry = Registry::with_builtin_drivers();
    if protocol == Protocol::Virtual {
        let mt = match &cli.media_type {
            Some(name) => {
                lbl_driver_file::MediaType::parse(name).map_err(|e| anyhow::anyhow!(e))?
            }
            None => lbl_driver_file::MediaType::Png,
        };
        registry.register(Box::new(lbl_driver_file::FileDriver::new(mt)));
    } else if cli.media_type.is_some() {
        bail!("--media-type only applies to --protocol virtual");
    }
    let driver = match registry.get(protocol) {
        Some(d) => d,
        None => bail!("no driver for protocol {protocol:?}"),
    };

    let ctx = EncodeContext::new(&job, &caps);
    let bytes = driver.encode(&bitmap, &ctx)?;

    match &cli.out {
        Some(path) => std::fs::write(path, &bytes)?,
        None => std::io::stdout().lock().write_all(&bytes)?,
    }
    Ok(())
}
