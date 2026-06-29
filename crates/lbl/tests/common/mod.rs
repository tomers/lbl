//! Shared harness for the golden-image integration tests.
//!
//! These tests drive the real label pipeline end to end — authoring HTML is
//! transpiled, rasterized through headless Chromium, dithered, and encoded to
//! an image file — then the resulting bytes are compared against a checked-in
//! reference ("golden") image.
//!
//! ## Hermeticity
//!
//! The QR and barcode placeholders are rendered by third-party JS libraries
//! that the transpiler normally loads from a CDN. To keep the tests offline and
//! reproducible, vendored copies (`tests/assets/*.js`) are inlined into the
//! document before rendering (see [`inline_assets`]).
//!
//! ## Updating references
//!
//! Chromium rasterization is not bit-identical across machines/Chrome versions,
//! so comparison allows a small fraction of differing pixels. When the pipeline
//! legitimately changes, regenerate every reference by running the suite with
//! `LBL_BLESS=1` set; missing references are always written on first run.

#![allow(dead_code)]

use std::io::Cursor;
use std::path::PathBuf;

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, RgbaImage};
use lbl_core::media::Media;
use lbl_core::MonoBitmap;
use lbl_core::OutputMode;
use lbl_dither::{dither, Algorithm};
use lbl_render::{render_two_pass, ChromiumBackend, RenderRequest};
use lbl_transpile_html::{transpile, AssetsBase, LabelStyle, TranspileOptions};

/// Vendored QR library (exposes the global `QRCode`).
const QR_JS: &str = include_str!("../assets/qrcode.min.js");
/// Vendored barcode library (exposes the global `JsBarcode`).
const BARCODE_JS: &str = include_str!("../assets/JsBarcode.all.min.js");

/// Per-pixel grayscale delta above which two pixels are considered different.
const PIXEL_DELTA: i16 = 8;
/// Default fraction of pixels allowed to differ before a comparison fails.
const DEFAULT_TOLERANCE: f64 = 0.02;

/// Replace the transpiler's external `<script src=...>` references to the QR and
/// barcode libraries with inline copies of the vendored files, so rendering
/// needs no network access.
pub fn inline_assets(html: &str) -> String {
    let qr_tag = format!("<script src=\"{}\"></script>", AssetsBase::Cdn.qrcode_url());
    let barcode_tag = format!(
        "<script src=\"{}\"></script>",
        AssetsBase::Cdn.jsbarcode_url()
    );
    html.replace(&qr_tag, &format!("<script>{QR_JS}</script>"))
        .replace(&barcode_tag, &format!("<script>{BARCODE_JS}</script>"))
}

/// A readable default label style (millimetre sizes resolved for `dpi` and
/// `supersample`). Mirrors the kind of style the orchestrator produces.
pub fn default_style(dpi: f64, supersample: u32) -> LabelStyle {
    // font, qr, barcode height, barcode module, padding, border (all mm).
    LabelStyle::from_mm(4.0, 18.0, 10.0, 0.33, 2.0, 0.0, dpi, supersample)
}

/// Like [`default_style`] but draws a border and uses generous padding, to
/// exercise the border/padding styling path.
pub fn bordered_style(dpi: f64, supersample: u32) -> LabelStyle {
    LabelStyle::from_mm(4.0, 18.0, 10.0, 0.33, 3.0, 1.0, dpi, supersample)
}

/// Run a single authoring-HTML label through transpile -> render -> dither,
/// returning the 1-bit bitmap that the file driver would encode.
///
/// This mirrors `lbl::pipeline::encode_label`, except the vendored QR/barcode
/// assets are inlined for hermeticity.
pub fn render_bitmap(
    backend: &ChromiumBackend,
    authoring_html: &str,
    media: &Media,
    style: &LabelStyle,
    supersample: u32,
    algorithm: Algorithm,
) -> MonoBitmap {
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: AssetsBase::Cdn,
            index: None,
            count: None,
            style: style.clone(),
        },
    );
    let html = inline_assets(&transpiled);

    let req = RenderRequest {
        width_dots: media.width_dots().0,
        height_dots: media.length_dots().map(|d| d.0),
        supersample,
    };
    let raster = render_two_pass(backend, &html, &req).expect("render label");
    dither(&raster, algorithm)
}

/// Absolute path to the directory holding the reference images.
pub fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// Whether references should be (re)written rather than compared.
pub fn blessing() -> bool {
    std::env::var("LBL_BLESS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Compare `bytes` (an encoded PNG) against `tests/golden/<name>.png`.
pub fn check_png(name: &str, bytes: &[u8]) -> Result<(), String> {
    check_image(name, "png", bytes, DEFAULT_TOLERANCE)
}

/// Compare `bytes` (an encoded image of type `ext`) against the matching
/// reference, allowing up to `max_diff_fraction` of pixels to differ.
///
/// In bless mode, or when the reference does not yet exist, the bytes are
/// written to disk and the check passes.
pub fn check_image(
    name: &str,
    ext: &str,
    bytes: &[u8],
    max_diff_fraction: f64,
) -> Result<(), String> {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create golden dir: {e}"))?;
    let path = dir.join(format!("{name}.{ext}"));

    if blessing() || !path.exists() {
        std::fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
        eprintln!("blessed reference {}", path.display());
        return Ok(());
    }

    let expected = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    compare_images(&expected, bytes, max_diff_fraction)
        .map_err(|e| format!("{name}.{ext}: {e} (re-run with LBL_BLESS=1 to update)"))
}

/// Decode both images and compare them pixel-by-pixel in grayscale.
fn compare_images(expected: &[u8], actual: &[u8], max_diff_fraction: f64) -> Result<(), String> {
    let expected = image::load_from_memory(expected)
        .map_err(|e| format!("decode reference: {e}"))?
        .to_luma8();
    let actual = image::load_from_memory(actual)
        .map_err(|e| format!("decode output: {e}"))?
        .to_luma8();

    if expected.dimensions() != actual.dimensions() {
        return Err(format!(
            "dimension mismatch: reference {:?}, output {:?}",
            expected.dimensions(),
            actual.dimensions()
        ));
    }

    let total = (expected.width() as u64) * (expected.height() as u64);
    let mut differing = 0u64;
    for (pe, pa) in expected.pixels().zip(actual.pixels()) {
        if (pe[0] as i16 - pa[0] as i16).abs() > PIXEL_DELTA {
            differing += 1;
        }
    }

    let fraction = differing as f64 / total.max(1) as f64;
    if fraction > max_diff_fraction {
        return Err(format!(
            "{differing}/{total} pixels differ ({:.2}%), exceeds allowed {:.2}%",
            fraction * 100.0,
            max_diff_fraction * 100.0
        ));
    }
    Ok(())
}

/// A small grayscale checkerboard, embedded as a `data:` URI, for the image
/// directive tests (self-contained so the rendered page needs no network).
pub fn checkerboard_data_uri() -> String {
    let mut img = RgbaImage::new(16, 16);
    for (x, y, px) in img.enumerate_pixels_mut() {
        let v = if (x / 4 + y / 4) % 2 == 0 { 0 } else { 255 };
        *px = image::Rgba([v, v, v, 255]);
    }
    png_data_uri(DynamicImage::ImageRgba8(img))
}

/// A horizontal black-to-white gradient, embedded as a `data:` URI, used to make
/// the differences between dithering algorithms visible and deterministic.
pub fn gradient_data_uri(width: u32, height: u32) -> String {
    let mut img = RgbaImage::new(width.max(1), height.max(1));
    let w = img.width();
    for (x, _y, px) in img.enumerate_pixels_mut() {
        let v = ((x * 255) / (w.max(2) - 1)) as u8;
        *px = image::Rgba([v, v, v, 255]);
    }
    png_data_uri(DynamicImage::ImageRgba8(img))
}

fn png_data_uri(img: DynamicImage) -> String {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)
        .expect("encode sample png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(buf.into_inner());
    format!("data:image/png;base64,{b64}")
}
