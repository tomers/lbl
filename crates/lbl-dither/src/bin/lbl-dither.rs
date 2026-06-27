//! `lbl-dither` — convert a raster image to a 1-bit PBM with dithering.

use std::io::{Read, Write};

use anyhow::Result;
use clap::Parser;
use lbl_dither::{dither, Algorithm};

#[derive(Parser)]
#[command(
    name = "lbl-dither",
    about = "Dither a raster image to a 1-bit bitmap (PBM) for printing"
)]
struct Cli {
    /// Input image (PNG/JPEG/...). If omitted, read from stdin.
    input: Option<std::path::PathBuf>,

    /// Dithering algorithm: auto | floyd-steinberg | ordered | none.
    #[arg(long, default_value = "auto")]
    algorithm: String,

    /// Threshold level (0-255) when --algorithm none/threshold is used.
    #[arg(long, default_value_t = 128)]
    threshold: u8,

    /// Output PBM path. If omitted, the PBM is written to stdout.
    #[arg(long)]
    out: Option<std::path::PathBuf>,

    /// Also write a viewable PNG preview to this path.
    #[arg(long)]
    preview_png: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let img = match &cli.input {
        Some(path) => image::open(path)?.to_rgba8(),
        None => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf)?;
            image::load_from_memory(&buf)?.to_rgba8()
        }
    };

    let algorithm = match Algorithm::parse(&cli.algorithm)? {
        Algorithm::Threshold(_) => Algorithm::Threshold(cli.threshold),
        other => other,
    };

    let bmp = dither(&img, algorithm);

    if let Some(path) = &cli.preview_png {
        write_preview(&bmp, path)?;
    }

    let pbm = bmp.to_pbm();
    match &cli.out {
        Some(path) => std::fs::write(path, pbm)?,
        None => std::io::stdout().lock().write_all(&pbm)?,
    }
    Ok(())
}

fn write_preview(bmp: &lbl_core::bitmap::MonoBitmap, path: &std::path::Path) -> Result<()> {
    let mut img = image::GrayImage::new(bmp.width, bmp.height);
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            let v = if bmp.get(x, y) { 0 } else { 255 };
            img.put_pixel(x, y, image::Luma([v]));
        }
    }
    img.save(path)?;
    Ok(())
}
