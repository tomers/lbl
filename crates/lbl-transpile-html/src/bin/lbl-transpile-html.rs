//! `lbl-transpile-html` — authoring HTML to browser-ready HTML.

use std::io::{Read, Write};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use lbl_core::job::OutputMode;
use lbl_transpile_html::{transpile, AssetsBase, LabelStyle, TranspileOptions};

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// Deterministic output for the rasterizer/printer.
    Print,
    /// Screen-oriented, gallery-friendly output.
    Preview,
}

impl From<Mode> for OutputMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Print => OutputMode::Print,
            Mode::Preview => OutputMode::Preview,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "lbl-transpile-html",
    about = "Transpile authoring HTML (<qr>, <barcode>, flex) into browser-ready HTML",
    color = clap::ColorChoice::Auto,
)]
struct Cli {
    /// Input authoring HTML file. If omitted, read from stdin.
    input: Option<std::path::PathBuf>,

    /// Output target mode.
    #[arg(long, value_enum, default_value = "print")]
    mode: Mode,

    /// Serve third-party libraries from this base URL/path instead of the CDN.
    #[arg(long)]
    assets_base: Option<String>,

    /// Label index within a batch (preview addressing).
    #[arg(long)]
    index: Option<usize>,

    /// Total labels in the batch (preview addressing).
    #[arg(long)]
    count: Option<usize>,

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

    /// Barcode single-module (narrowest bar) width, in pixels.
    #[arg(long)]
    barcode_module_px: Option<f64>,

    /// Inner padding between the label edge and its content, in pixels.
    #[arg(long)]
    padding_px: Option<f64>,

    /// Border drawn around the label, in pixels (0 = none).
    #[arg(long)]
    border_px: Option<f64>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let input = match &cli.input {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    let mut style = LabelStyle::default();
    if let Some(v) = cli.font_size_px {
        style.font_size_px = v;
    }
    if let Some(v) = cli.qr_size_px {
        style.qr_size_px = v;
    }
    if let Some(v) = &cli.qr_error_correction {
        style.qr_error_correction =
            lbl_transpile_html::QrErrorCorrection::parse(v).unwrap_or_default();
    }
    if let Some(v) = cli.qr_margin {
        style.qr_margin = v;
    }
    if let Some(v) = &cli.qr_dark {
        style.qr_dark = v.clone();
    }
    if let Some(v) = &cli.qr_light {
        style.qr_light = v.clone();
    }
    if let Some(v) = cli.barcode_height_px {
        style.barcode_height_px = v;
    }
    if let Some(v) = cli.barcode_module_px {
        style.barcode_module_width_px = v;
    }
    if let Some(v) = cli.padding_px {
        style.padding_px = v;
    }
    if let Some(v) = cli.border_px {
        style.border_width_px = v;
    }

    let opts = TranspileOptions {
        mode: cli.mode.into(),
        assets_base: cli
            .assets_base
            .map(AssetsBase::Local)
            .unwrap_or(AssetsBase::Cdn),
        index: cli.index,
        count: cli.count,
        style,
    };

    let out = transpile(&input, &opts);
    std::io::stdout().lock().write_all(out.as_bytes())?;
    Ok(())
}
