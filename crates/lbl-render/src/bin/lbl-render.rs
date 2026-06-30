//! `lbl-render` — rasterize HTML to a PNG sized for printer media.

use std::io::{Read, Write};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use lbl_render::{render_two_pass, RenderRequest, SidecarBackend};

#[derive(Clone, Copy, ValueEnum)]
enum Backend {
    /// In-process headless Chromium (default).
    Chromium,
    /// External Node/Playwright sidecar.
    Sidecar,
}

#[derive(Parser)]
#[command(
    name = "lbl-render",
    about = "Render HTML to a raster image (two-pass)",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    /// Input HTML file. If omitted, read from stdin.
    input: Option<std::path::PathBuf>,

    /// Target width in device dots (omit for content-determined width).
    #[arg(long)]
    width_dots: Option<u32>,

    /// Target height in device dots (omit for content-determined height).
    #[arg(long)]
    height_dots: Option<u32>,

    /// Supersample factor for the high-resolution first pass (>= 1). The page
    /// is rasterized at this multiple of the target device dots, then
    /// downscaled before dithering. Also scales mm→px style conversion during
    /// transpilation.
    #[arg(long, default_value_t = 3)]
    supersample: u32,

    /// Rendering backend.
    #[arg(long, value_enum, default_value = "chromium")]
    backend: Backend,

    /// Output PNG path. If omitted, the PNG is written to stdout.
    #[arg(long)]
    out: Option<std::path::PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let html = match &cli.input {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    if cli.width_dots.is_none() && cli.height_dots.is_none() {
        anyhow::bail!("at least one of --width-dots / --height-dots is required");
    }

    let req = RenderRequest {
        width_dots: cli.width_dots,
        height_dots: cli.height_dots,
        supersample: cli.supersample,
    };

    let image = match cli.backend {
        Backend::Sidecar => render_two_pass(&SidecarBackend::node_default(), &html, &req)?,
        Backend::Chromium => render_with_chromium(&html, &req)?,
    };

    match &cli.out {
        Some(path) => image.save(path)?,
        None => {
            let mut buf = std::io::Cursor::new(Vec::new());
            image.write_to(&mut buf, image::ImageFormat::Png)?;
            std::io::stdout().lock().write_all(&buf.into_inner())?;
        }
    }
    Ok(())
}

#[cfg(feature = "chromium")]
fn render_with_chromium(html: &str, req: &RenderRequest) -> Result<image::RgbaImage> {
    let backend = lbl_render::ChromiumBackend::launch()?;
    Ok(render_two_pass(&backend, html, req)?)
}

#[cfg(not(feature = "chromium"))]
fn render_with_chromium(_html: &str, _req: &RenderRequest) -> Result<image::RgbaImage> {
    anyhow::bail!("this build was compiled without the `chromium` feature; use --backend sidecar")
}
