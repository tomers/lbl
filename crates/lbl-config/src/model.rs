//! The configuration data model.

use lbl_core::Orientation;
use serde::{Deserialize, Serialize};

/// The fully-merged configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// General, top-level settings.
    pub general: GeneralConfig,
    /// Rendering/dithering defaults.
    pub render: RenderConfig,
    /// Default label visual sizing (fonts, codes).
    pub style: StyleConfig,
    /// Default `lbl print` options (overridable per run on the CLI).
    pub print: PrintConfig,
    /// Media catalog settings.
    pub catalog: CatalogConfig,
}

/// General, top-level settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct GeneralConfig {
    /// Id of the default printer (matches a persisted `PrinterProfile.id`).
    pub default_printer: Option<String>,
    /// Override for the cache directory (catalog images, render scratch).
    pub cache_dir: Option<String>,
}

/// Default rendering/dithering parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RenderConfig {
    /// Supersample factor for the high-resolution first pass (>= 1). See
    /// `docs/src/guides/rendering-quality.md` and ADR-0004.
    pub supersample: u32,
    /// Default dithering algorithm (`floyd-steinberg` | `ordered` | `none`).
    pub dither: String,
    /// Prefer the Node/Playwright sidecar over the in-process Chromium driver.
    pub use_sidecar: bool,
    /// Default label orientation (`portrait` | `landscape`). Landscape is the
    /// default because stripe labels are usually printed along their longer
    /// dimension.
    pub orientation: Orientation,
    /// Warn after a hardware print when print efficiency (`print time ÷ total
    /// time`) falls below this fraction (0.0–1.0). Set to `0` to disable.
    pub efficiency_warn_below: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            supersample: 4,
            dither: "floyd-steinberg".to_string(),
            use_sidecar: false,
            orientation: Orientation::default(),
            efficiency_warn_below: 0.55,
        }
    }
}

/// Default label visual sizing, in millimetres.
///
/// These are physical sizes on the printed label; the pipeline converts them
/// to pixels using the target DPI and supersample factor, so they stay
/// consistent regardless of resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleConfig {
    /// Base text size, in mm.
    pub font_size_mm: f64,
    /// QR code edge length, in mm.
    pub qr_size_mm: f64,
    /// QR error-correction level: `L`, `M`, `Q`, or `H` (aliases: `low`,
    /// `medium`, `quartile`, `high`, or `7%`/`15%`/`25%`/`30%`).
    pub qr_error_correction: String,
    /// QR quiet zone, in modules (0 = none).
    pub qr_margin: u32,
    /// QR dark module color (hex, e.g. `#000000`).
    pub qr_dark: String,
    /// QR light module color (hex, e.g. `#ffffff`).
    pub qr_light: String,
    /// Barcode bar height, in mm.
    pub barcode_height_mm: f64,
    /// Barcode single-module (narrowest bar) width, in mm.
    pub barcode_module_width_mm: f64,
    /// Inner padding between the label edge and its content, in mm.
    pub padding_mm: f64,
    /// Gap between sibling elements inside `.lbl-label` / flex rows, in mm.
    pub element_gap_mm: f64,
    /// Border drawn around the label, in mm (0 = no border).
    pub border_width_mm: f64,
    /// Corner radius on fixed die-cut labels, in mm (preview only).
    pub corner_radius_mm: f64,
    /// How `.lbl-label` fills the render viewport: `auto` (fill fixed-length
    /// media, shrink on continuous), `fill`, or `content`.
    pub label_fit: String,
    /// Cross-axis alignment when the media width is known: `start`, `center`,
    /// or `end` (`left` / `right` aliases).
    pub label_align: String,
    /// Main-axis alignment in fill mode: `start`, `center`, or `end` (`top` /
    /// `bottom` aliases).
    pub label_valign: String,
    /// Fit-box scale in fill mode (`1.0` = 100%; also accepts `0.8` or `80%`).
    pub label_fit_scale: f64,
    /// Multiplier applied to auto-fit text size in fill mode (`1.0` = grow to
    /// fill; `0.8` = 80% of the computed maximum).
    pub font_fit_scale: f64,
    /// Inset from the physical media edge, uniform (mm). See also the axis and
    /// side-specific `media_inset_*` fields.
    pub media_inset_mm: f64,
    /// Inset on both cross-axis sides (left + right in portrait).
    pub media_inset_horizontal_mm: Option<f64>,
    /// Inset on both main-axis sides (top + bottom in portrait).
    pub media_inset_vertical_mm: Option<f64>,
    /// Main-axis start inset (top in portrait; aliases: top).
    pub media_inset_start_mm: Option<f64>,
    /// Main-axis end inset (bottom in portrait; aliases: bottom).
    pub media_inset_end_mm: Option<f64>,
    /// Cross-axis start inset (left in portrait; aliases: left).
    pub media_inset_cross_start_mm: Option<f64>,
    /// Cross-axis end inset (right in portrait; aliases: right).
    pub media_inset_cross_end_mm: Option<f64>,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            font_size_mm: 2.0,
            qr_size_mm: 15.0,
            qr_error_correction: "M".into(),
            qr_margin: 0,
            qr_dark: "#000000".into(),
            qr_light: "#ffffff".into(),
            barcode_height_mm: 12.0,
            barcode_module_width_mm: 0.33,
            padding_mm: 2.0,
            element_gap_mm: 4.0,
            border_width_mm: 0.0,
            corner_radius_mm: 2.0,
            label_fit: "auto".into(),
            label_align: "center".into(),
            label_valign: "center".into(),
            label_fit_scale: 1.0,
            font_fit_scale: 1.0,
            media_inset_mm: 0.0,
            media_inset_horizontal_mm: None,
            media_inset_vertical_mm: None,
            media_inset_start_mm: None,
            media_inset_end_mm: None,
            media_inset_cross_start_mm: None,
            media_inset_cross_end_mm: None,
        }
    }
}

/// Default options for `lbl print`. CLI flags win when explicitly passed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrintConfig {
    /// Preview each label on the terminal and ask before printing.
    pub confirm: bool,
    /// Dump every pipeline stage to stderr.
    pub debug: bool,
    /// When to cut (`none`, `every`, `end`).
    pub cut_mode: String,
    /// Mark the target printer as cut-capable.
    pub supports_cut: bool,
    /// Copies per label.
    pub copies: u32,
    /// Optional print density / heat (driver-specific).
    pub density: Option<u8>,
    /// Dithering algorithm (`auto`, `floyd-steinberg`, `ordered`, `none`).
    pub dither: String,
    /// Default protocol (`dymo`, `niimbot`, `virtual`, …) when `--protocol` is
    /// omitted.
    pub protocol: Option<String>,
    /// Render backend (`chromium` or `sidecar`).
    pub backend: String,
    /// Default Bluetooth LE printer name/address.
    pub bluetooth: Option<String>,
    /// Default serial port (`/dev/ttyACM0` or `path:baud`).
    pub serial: Option<String>,
    /// Default USB target (`vid:pid` hex).
    pub usb: Option<String>,
    /// Default network target (`host:port`).
    pub network: Option<String>,
    /// NIIMBOT task variant (`standard`, `v4`, or `b1`).
    pub niimbot_task: String,
    /// Virtual-printer output format (`png`, `bmp`, …).
    pub media_type: Option<String>,
    /// Virtual-printer export mode: `raster` (default) or `vector` (PDF).
    pub export_mode: Option<String>,
}

impl Default for PrintConfig {
    fn default() -> Self {
        Self {
            confirm: false,
            debug: false,
            cut_mode: "none".into(),
            supports_cut: false,
            copies: 1,
            density: None,
            dither: "auto".into(),
            protocol: None,
            backend: "chromium".into(),
            bluetooth: None,
            serial: None,
            usb: None,
            network: None,
            niimbot_task: "standard".into(),
            media_type: None,
            export_mode: None,
        }
    }
}

/// Media-catalog-related settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogConfig {
    /// Whether affiliate/purchase links are shown.
    pub affiliate_enabled: bool,
    /// Optional affiliate tag appended to purchase URLs.
    pub affiliate_tag: Option<String>,
    /// Extra user-supplied catalog files (TOML/JSON), merged over the bundled
    /// catalog.
    pub extra_paths: Vec<String>,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            affiliate_enabled: true,
            affiliate_tag: None,
            extra_paths: Vec::new(),
        }
    }
}
