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
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            supersample: 3,
            dither: "floyd-steinberg".to_string(),
            use_sidecar: false,
            orientation: Orientation::default(),
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
    /// Barcode bar height, in mm.
    pub barcode_height_mm: f64,
    /// Barcode single-module (narrowest bar) width, in mm.
    pub barcode_module_width_mm: f64,
    /// Inner padding between the label edge and its content, in mm.
    pub padding_mm: f64,
    /// Border drawn around the label, in mm (0 = no border).
    pub border_width_mm: f64,
}

impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            font_size_mm: 2.0,
            qr_size_mm: 15.0,
            barcode_height_mm: 12.0,
            barcode_module_width_mm: 0.33,
            padding_mm: 2.0,
            border_width_mm: 0.0,
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
