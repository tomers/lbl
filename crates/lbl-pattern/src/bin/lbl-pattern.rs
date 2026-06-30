//! `lbl-pattern` — emit a calibration test pattern as binary PBM (P4).

use std::io::Write;

use anyhow::Result;
use clap::Parser;
use lbl_core::media::Media;
use lbl_core::units::Dpi;
use lbl_pattern::{resolve_head_dots, sample_pattern_sized};

#[derive(Parser)]
#[command(
    name = "lbl-pattern",
    about = "Generate a calibration test pattern as a 1-bit PBM (no dithering or rescaling)",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    /// Pattern height in device dots across the print head. When omitted, derived
    /// from `--width-mm` at `--dpi` (same as `lbl print --sample-pattern` with
    /// no value).
    #[arg(long, num_args = 0..=1)]
    height: Option<Option<u32>>,

    /// Media width in millimeters (used when `--height` is omitted).
    #[arg(long, default_value_t = 12.0)]
    width_mm: f64,

    /// Fixed media length in millimeters. When set, the pattern is extended along
    /// the feed to match (Labelle / DYMO bitmap layout).
    #[arg(long)]
    length_mm: Option<f64>,

    /// Device resolution in DPI (used when `--height` is omitted).
    #[arg(long, default_value_t = 203.0)]
    dpi: f64,

    /// Output PBM path. If omitted, the PBM is written to stdout.
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
    let head_dots =
        resolve_head_dots(cli.height.flatten(), &media).map_err(|e| anyhow::anyhow!(e))?;

    let bmp = sample_pattern_sized(head_dots, media.length_dots().map(|d| d.0));
    let pbm = bmp.to_pbm();
    match &cli.out {
        Some(path) => std::fs::write(path, pbm)?,
        None => std::io::stdout().lock().write_all(&pbm)?,
    }
    Ok(())
}
