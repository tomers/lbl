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

#[derive(Parser)]
#[command(name = "lbl-encode", about = "Encode a 1-bit PBM into printer protocol bytes")]
struct Cli {
    /// Input PBM (P4) file. If omitted, read from stdin.
    input: Option<std::path::PathBuf>,

    /// Target protocol / driver.
    #[arg(long, value_enum)]
    protocol: ProtocolArg,

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

    let pbm = match &cli.input {
        Some(path) => std::fs::read(path).with_context(|| format!("reading {}", path.display()))?,
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            buf
        }
    };
    let bitmap = MonoBitmap::from_pbm(&pbm).map_err(|e| anyhow::anyhow!("parsing PBM: {e}"))?;

    let dpi = Dpi(cli.dpi);
    let media = match cli.length_mm {
        Some(len) => Media::fixed(cli.width_mm, len, dpi),
        None => Media::continuous(cli.width_mm, dpi),
    };
    let mut job = JobSpec::new(media);
    job.cut = cli.cut;
    job.copies = cli.copies;

    let caps = PrinterCapabilities {
        dpi,
        max_width_mm: cli.width_mm,
        supports_cut: cli.supports_cut,
        reports_media: false,
    };

    let registry = Registry::with_builtin_drivers();
    let protocol: Protocol = cli.protocol.into();
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
