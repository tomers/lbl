//! The configuration data model.

use serde::{Deserialize, Serialize};

/// The fully-merged configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// General, top-level settings.
    pub general: GeneralConfig,
    /// Rendering/dithering defaults.
    pub render: RenderConfig,
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
    /// Supersample factor for the high-resolution first pass (>= 1).
    pub supersample: u32,
    /// Default dithering algorithm (`floyd-steinberg` | `ordered` | `none`).
    pub dither: String,
    /// Prefer the Node/Playwright sidecar over the in-process Chromium driver.
    pub use_sidecar: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            supersample: 3,
            dither: "floyd-steinberg".to_string(),
            use_sidecar: false,
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
