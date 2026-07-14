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

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, RgbaImage};
use lbl::AuthoringLabel;
use lbl_core::media::Media;
use lbl_core::MonoBitmap;
use lbl_core::OutputMode;
use lbl_dither::{dither, Algorithm};
use lbl_render::{
    render_two_pass, ChromiumBackend, PdfExportRequest, RenderBackend, RenderRequest,
};
use lbl_template::{DefaultResolver, Engine, RenderOptions, ResourceResolver, TemplateError};
use lbl_transpile_html::{
    transpile, AssetsBase, LabelAlign, LabelFit, LabelStyle, LabelValign, MediaInsetPx, PageSizeMm,
    TranspileOptions,
};

/// Authoring HTML for layout/fit golden cases: short text + QR on a tall label.
pub const LAYOUT_FIXTURE_HTML: &str = include_str!("../fixtures/layout/short_label.html");

/// Transpile/render options that control fit-box sizing and alignment.
#[derive(Debug, Clone, Copy)]
pub struct LayoutOptions {
    pub label_fit: LabelFit,
    pub label_align: LabelAlign,
    pub label_valign: LabelValign,
    pub label_fit_scale: f64,
    pub media_inset: MediaInsetPx,
}

impl LayoutOptions {
    /// Defaults for fixed-length media (fill + centered + full scale).
    pub fn fill_defaults() -> Self {
        Self {
            label_fit: LabelFit::Fill,
            label_align: LabelAlign::Center,
            label_valign: LabelValign::Center,
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
        }
    }

    /// Defaults for continuous media (content width + horizontal centering).
    pub fn continuous_defaults() -> Self {
        Self {
            label_fit: LabelFit::Content,
            label_align: LabelAlign::Center,
            label_valign: LabelValign::Center,
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
        }
    }
}

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
    LabelStyle::from_mm(4.0, 18.0, 10.0, 0.33, 2.0, 2.0, 0.0, 2.0, dpi, supersample)
}

/// Like [`default_style`] but draws a border and uses generous padding, to
/// exercise the border/padding styling path.
pub fn bordered_style(dpi: f64, supersample: u32) -> LabelStyle {
    LabelStyle::from_mm(4.0, 18.0, 10.0, 0.33, 3.0, 2.0, 1.0, 2.0, dpi, supersample)
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
    let layout = if media.length_dots().is_some() {
        LayoutOptions::fill_defaults()
    } else {
        LayoutOptions::continuous_defaults()
    };
    render_bitmap_with_layout(
        backend,
        authoring_html,
        media,
        style,
        supersample,
        algorithm,
        layout,
    )
}

/// Like [`render_bitmap`], but with explicit fit/alignment options.
pub fn render_bitmap_with_layout(
    backend: &ChromiumBackend,
    authoring_html: &str,
    media: &Media,
    style: &LabelStyle,
    supersample: u32,
    algorithm: Algorithm,
    layout: LayoutOptions,
) -> MonoBitmap {
    let viewport =
        lbl::pipeline::render_viewport_px(media, supersample, lbl_core::Rotation::None, None);
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: AssetsBase::Cdn,
            index: None,
            count: None,
            style: style.clone(),
            label_fit: layout.label_fit,
            viewport: Some(viewport),
            label_align: layout.label_align,
            label_valign: layout.label_valign,
            label_fit_scale: layout.label_fit_scale,
            media_inset: layout.media_inset,
            ..Default::default()
        },
    );
    let html = inline_assets(&transpiled);

    let req = RenderRequest {
        width_dots: Some(media.width_dots().0),
        height_dots: media.length_dots().map(|d| d.0),
        supersample,
    };
    let raster = render_two_pass(backend, &html, &req).expect("render label");
    dither(&raster, algorithm)
}

/// Run a label through the vector PDF export path (no rasterization/dithering).
pub fn render_vector_pdf(
    backend: &ChromiumBackend,
    authoring_html: &str,
    media: &Media,
    style: &LabelStyle,
) -> Vec<u8> {
    let layout = if media.length_dots().is_some() {
        LayoutOptions::fill_defaults()
    } else {
        LayoutOptions::continuous_defaults()
    };
    render_vector_pdf_with_layout(backend, authoring_html, media, style, layout)
}

/// Like [`render_vector_pdf`], but with explicit fit/alignment options.
pub fn render_vector_pdf_with_layout(
    backend: &ChromiumBackend,
    authoring_html: &str,
    media: &Media,
    style: &LabelStyle,
    layout: LayoutOptions,
) -> Vec<u8> {
    use lbl::pipeline::{page_size_mm, render_viewport_vector};

    let viewport = render_viewport_vector(media, lbl_core::Rotation::None);
    let page_size = page_size_mm(media, lbl_core::Rotation::None);
    let transpiled = transpile(
        authoring_html,
        &TranspileOptions {
            mode: OutputMode::Print,
            assets_base: AssetsBase::Cdn,
            index: None,
            count: None,
            style: style.clone(),
            label_fit: layout.label_fit,
            viewport: Some(viewport),
            label_align: layout.label_align,
            label_valign: layout.label_valign,
            label_fit_scale: layout.label_fit_scale,
            font_fit_scale: 1.0,
            media_inset: layout.media_inset,
            page_size: Some(page_size),
            ..Default::default()
        },
    );
    let html = inline_assets(&transpiled);
    let req = PdfExportRequest {
        width_mm: page_size.width_mm,
        height_mm: page_size.height_mm,
    };
    backend.export_pdf(&html, &req).expect("export vector PDF")
}

/// A readable vector-export label style (mm sizes at the CSS reference DPI).
pub fn vector_style() -> LabelStyle {
    LabelStyle::from_mm(
        4.0,
        18.0,
        10.0,
        0.33,
        2.0,
        2.0,
        0.0,
        2.0,
        lbl::pipeline::VECTOR_CSS_DPI,
        1,
    )
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

/// Compare `bytes` (a PDF) against `tests/golden/<name>.pdf`.
///
/// Validates the PDF header and that the first page's MediaBox matches the
/// expected physical dimensions (within ~0.5 mm). In bless mode, or when the
/// reference does not yet exist, the bytes are written to disk.
pub fn check_pdf(name: &str, bytes: &[u8], expected: PageSizeMm) -> Result<(), String> {
    let dir = golden_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create golden dir: {e}"))?;
    let path = dir.join(format!("{name}.pdf"));

    if blessing() || !path.exists() {
        fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
        eprintln!("blessed reference {}", path.display());
        return Ok(());
    }

    let expected_bytes = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    validate_pdf_structure(&expected_bytes, expected)
        .map_err(|e| format!("reference {name}.pdf: {e}"))?;
    validate_pdf_structure(bytes, expected)
        .map_err(|e| format!("{name}.pdf: {e} (re-run with LBL_BLESS=1 to update)"))
}

fn validate_pdf_structure(bytes: &[u8], expected: PageSizeMm) -> Result<(), String> {
    if !bytes.starts_with(b"%PDF-") {
        return Err("missing %PDF- header".into());
    }
    let (w_pt, h_pt) = pdf_media_box_pt(bytes)?;
    let w_mm = w_pt * 25.4 / 72.0;
    let h_mm = h_pt * 25.4 / 72.0;
    if (w_mm - expected.width_mm).abs() > 0.6 {
        return Err(format!(
            "page width {w_mm:.2} mm, expected {:.2} mm",
            expected.width_mm
        ));
    }
    if let Some(exp_h) = expected.height_mm {
        if (h_mm - exp_h).abs() > 0.6 {
            return Err(format!("page height {h_mm:.2} mm, expected {exp_h:.2} mm"));
        }
    }
    Ok(())
}

fn pdf_media_box_pt(bytes: &[u8]) -> Result<(f64, f64), String> {
    let text = String::from_utf8_lossy(bytes);
    let start = text
        .find("/MediaBox")
        .ok_or_else(|| "MediaBox not found".to_string())?;
    let slice = &text[start..];
    let open = slice.find('[').ok_or_else(|| "MediaBox [".to_string())? + start;
    let close = text[open..]
        .find(']')
        .ok_or_else(|| "MediaBox ]".to_string())?
        + open;
    let inner = text[open + 1..close].split_whitespace();
    let nums: Vec<f64> = inner.filter_map(|s| s.parse().ok()).collect();
    if nums.len() != 4 {
        return Err("MediaBox values".into());
    }
    Ok((nums[2] - nums[0], nums[3] - nums[1]))
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
    fs::create_dir_all(&dir).map_err(|e| format!("create golden dir: {e}"))?;
    let path = dir.join(format!("{name}.{ext}"));

    if blessing() || !path.exists() {
        fs::write(&path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
        eprintln!("blessed reference {}", path.display());
        return Ok(());
    }

    let expected = fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
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

/// Path to the portrait identity-card batch fixture (`.lbl` single-file source).
pub fn identity_cards_lbl_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/identity_cards/identity_cards.lbl")
}

/// Load a `.lbl` single-file source the way `lbl-template --template PATH
/// --inline-resources` does: frontmatter data + template body, with `<img src>`
/// paths resolved relative to the fixture directory (or fetched over HTTP).
pub fn authoring_labels_from_lbl(lbl_path: &Path) -> Result<Vec<AuthoringLabel>, String> {
    let fixture_dir = lbl_path.parent().ok_or_else(|| {
        format!(
            "lbl fixture has no parent directory: {}",
            lbl_path.display()
        )
    })?;
    let source =
        fs::read_to_string(lbl_path).map_err(|e| format!("read {}: {e}", lbl_path.display()))?;

    let labels = Engine::new()
        .render_with_resources(
            &source,
            None,
            &RenderOptions::default(),
            &FixtureResolver {
                base: fixture_dir.to_path_buf(),
            },
        )
        .map_err(|e| format!("render {}: {e}", lbl_path.display()))?;

    Ok(labels
        .into_iter()
        .map(|label| AuthoringLabel {
            index: label.index,
            html: label.html,
        })
        .collect())
}

/// Resolve `<img src>` references relative to a fixture directory (mirrors local
/// paths passed to `lbl-template --inline-resources` when run from that folder),
/// or fetch remote URLs via HTTP.
struct FixtureResolver {
    base: PathBuf,
}

impl ResourceResolver for FixtureResolver {
    fn fetch(&self, reference: &str) -> Result<(Vec<u8>, String), TemplateError> {
        if reference.starts_with("http://") || reference.starts_with("https://") {
            return DefaultResolver.fetch(reference);
        }
        let path = self.base.join(reference);
        let bytes = fs::read(&path)
            .map_err(|e| TemplateError::Resource(format!("{}: {e}", path.display())))?;
        Ok((bytes, guess_image_mime(&path)))
    }
}

fn guess_image_mime(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// Like [`bordered_style`] but tuned for portrait name-badge / ID cards on
/// DYMO 99014 (54×101 mm) at print resolution.
pub fn identity_card_style(dpi: f64, supersample: u32) -> LabelStyle {
    // font, qr, barcode height, barcode module, padding, border (all mm).
    LabelStyle::from_mm(3.0, 11.0, 9.0, 0.32, 2.0, 2.0, 0.5, 2.0, dpi, supersample)
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
