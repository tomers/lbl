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
    /// Id of the default printer (matches a persisted `DeviceProfile.id`).
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
/// Domain bags are flattened so TOML / env / JSON keep historical flat keys
/// under `[style]` (`font_size_mm`, `padding_mm`, …).
///
/// These are physical sizes on the printed label; the pipeline converts them
/// to pixels using the target DPI and supersample factor, so they stay
/// consistent regardless of resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct StyleConfig {
    /// Base typography.
    #[serde(flatten)]
    pub typography: StyleTypography,
    /// QR code defaults.
    #[serde(flatten)]
    pub qr: StyleQr,
    /// 1D barcode defaults.
    #[serde(flatten)]
    pub barcode: StyleBarcode,
    /// Inner content padding cascade.
    #[serde(flatten)]
    pub padding: StylePadding,
    /// Border / gap / corner chrome.
    #[serde(flatten)]
    pub chrome: StyleChrome,
    /// Viewport fit and alignment.
    #[serde(flatten)]
    pub fit: StyleFit,
    /// Physical media-edge inset cascade.
    #[serde(flatten)]
    pub media_inset: StyleMediaInset,
}

/// Base text sizing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleTypography {
    /// Base text size, in mm.
    pub font_size_mm: f64,
}

impl Default for StyleTypography {
    fn default() -> Self {
        Self { font_size_mm: 2.0 }
    }
}

/// QR code visual defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleQr {
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
}

impl Default for StyleQr {
    fn default() -> Self {
        Self {
            qr_size_mm: 15.0,
            qr_error_correction: "M".into(),
            qr_margin: 0,
            qr_dark: "#000000".into(),
            qr_light: "#ffffff".into(),
        }
    }
}

/// 1D barcode visual defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleBarcode {
    /// Barcode bar height, in mm.
    pub barcode_height_mm: f64,
    /// Barcode single-module (narrowest bar) width, in mm.
    pub barcode_module_width_mm: f64,
}

impl Default for StyleBarcode {
    fn default() -> Self {
        Self {
            barcode_height_mm: 12.0,
            barcode_module_width_mm: 0.33,
        }
    }
}

/// Cascading inner padding between the label edge and its content (mm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StylePadding {
    /// Uniform base padding on all sides.
    pub padding_mm: f64,
    /// Padding on both horizontal sides (left + right).
    pub padding_horizontal_mm: Option<f64>,
    /// Padding on both vertical sides (top + bottom).
    pub padding_vertical_mm: Option<f64>,
    /// Top-side padding.
    pub padding_top_mm: Option<f64>,
    /// Right-side padding.
    pub padding_right_mm: Option<f64>,
    /// Bottom-side padding.
    pub padding_bottom_mm: Option<f64>,
    /// Left-side padding.
    pub padding_left_mm: Option<f64>,
}

impl Default for StylePadding {
    fn default() -> Self {
        Self {
            padding_mm: 2.0,
            padding_horizontal_mm: None,
            padding_vertical_mm: None,
            padding_top_mm: None,
            padding_right_mm: None,
            padding_bottom_mm: None,
            padding_left_mm: None,
        }
    }
}

/// Label chrome (gap, border, corners).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleChrome {
    /// Gap between sibling elements inside `.lbl-label` / flex rows, in mm.
    pub element_gap_mm: f64,
    /// Border drawn around the label, in mm (0 = no border).
    pub border_width_mm: f64,
    /// Corner radius on fixed die-cut labels, in mm (preview only).
    pub corner_radius_mm: f64,
}

impl Default for StyleChrome {
    fn default() -> Self {
        Self {
            element_gap_mm: 4.0,
            border_width_mm: 0.0,
            corner_radius_mm: 2.0,
        }
    }
}

/// How `.lbl-label` fills and aligns inside the render viewport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleFit {
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
    /// Multiplier applied to auto-fit text size in fill mode (`1.0` = comfortable
    /// max fill with width safety margin and line-height 1.1; `0.8` = 80% of that;
    /// values above `1.0` spend width margin and tighten line-height so glyph ink
    /// can grow into the box without overflowing the line box).
    pub font_fit_scale: f64,
}

impl Default for StyleFit {
    fn default() -> Self {
        Self {
            label_fit: "auto".into(),
            label_align: "center".into(),
            label_valign: "center".into(),
            label_fit_scale: 1.0,
            font_fit_scale: 1.0,
        }
    }
}

/// Cascading inset from the physical media edge (mm).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleMediaInset {
    /// Inset from the physical media edge, uniform (mm).
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

impl Default for StyleMediaInset {
    fn default() -> Self {
        Self {
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
    /// Protocol-specific defaults (`[print.driver.<protocol>]`).
    pub driver: DriverPrintConfig,
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
    /// Opaque protocol-specific driver variant (firmware/task profile).
    ///
    /// Interpreted by the selected protocol's driver; unused by protocols with
    /// no variants. Accepts the legacy TOML/env key `niimbot_task`.
    #[serde(default, alias = "niimbot_task")]
    pub driver_variant: Option<String>,
    /// Virtual-printer output format (`png`, `bmp`, …).
    pub media_type: Option<String>,
    /// Virtual-printer export mode: `raster` (default) or `vector` (PDF).
    pub export_mode: Option<String>,
}

/// Protocol-specific print defaults (`[print.driver]`).
///
/// Mirrors [`lbl_core::DriverOptions`]: each driver reads only its own bag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DriverPrintConfig {
    /// DYMO LabelWriter 550-series defaults (`[print.driver.dymo]`).
    pub dymo: DymoPrintConfig,
}

/// DYMO LabelWriter 550-series print defaults (`[print.driver.dymo]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct DymoPrintConfig {
    /// Text vs graphics engine mode (`text` | `graphics`).
    pub output_mode: Option<String>,
    /// Feed speed (`normal` | `high`).
    pub speed: Option<String>,
}

impl DriverPrintConfig {
    /// Overlay CLI `--driver-opt KEY=VALUE` entries (dotted paths under `driver`).
    ///
    /// Unknown bags or fields are rejected (`deny_unknown_fields` on the
    /// round-trip deserialize). Later specs win when they set the same path.
    pub fn with_opt_overrides<'a, I>(&self, specs: I) -> crate::Result<Self>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut value =
            serde_json::to_value(self).map_err(|e| crate::ConfigError::Load(e.to_string()))?;
        for spec in specs {
            apply_driver_opt(&mut value, spec)?;
        }
        serde_json::from_value(value).map_err(|e| {
            crate::ConfigError::Load(format!("invalid --driver-opt (unknown key or type): {e}"))
        })
    }
}

/// Apply one `KEY=VALUE` overlay onto a JSON object tree.
fn apply_driver_opt(root: &mut serde_json::Value, spec: &str) -> crate::Result<()> {
    let (key, raw_val) = spec.split_once('=').ok_or_else(|| {
        crate::ConfigError::Load(format!(
            "invalid --driver-opt '{spec}' (expected KEY=VALUE, e.g. dymo.output_mode=graphics)"
        ))
    })?;
    let key = key.trim();
    let raw_val = raw_val.trim();
    if key.is_empty() {
        return Err(crate::ConfigError::Load(format!(
            "invalid --driver-opt '{spec}' (empty key)"
        )));
    }
    let parts: Vec<&str> = key.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err(crate::ConfigError::Load(format!(
            "invalid --driver-opt '{spec}' (empty key)"
        )));
    }

    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        let is_leaf = i + 1 == parts.len();
        if is_leaf {
            let map = cur.as_object_mut().ok_or_else(|| {
                crate::ConfigError::Load(format!(
                    "invalid --driver-opt '{spec}' (path prefix is not an object)"
                ))
            })?;
            map.insert(
                (*part).to_string(),
                serde_json::Value::String(raw_val.to_string()),
            );
            return Ok(());
        }
        if !cur.is_object() {
            return Err(crate::ConfigError::Load(format!(
                "invalid --driver-opt '{spec}' (path prefix is not an object)"
            )));
        }
        let map = cur.as_object_mut().expect("checked is_object");
        if !map.contains_key(*part) {
            map.insert((*part).to_string(), serde_json::json!({}));
        } else if !map.get(*part).is_some_and(|v| v.is_object()) {
            return Err(crate::ConfigError::Load(format!(
                "invalid --driver-opt '{spec}' (path prefix is not an object)"
            )));
        }
        cur = map.get_mut(*part).expect("inserted or existing object");
    }
    Err(crate::ConfigError::Load(format!(
        "invalid --driver-opt '{spec}' (empty key)"
    )))
}

#[cfg(test)]
mod driver_opt_tests {
    use super::*;

    #[test]
    fn applies_known_dymo_fields() {
        let cfg = DriverPrintConfig::default()
            .with_opt_overrides(["dymo.output_mode=graphics", "dymo.speed=high"])
            .unwrap();
        assert_eq!(cfg.dymo.output_mode.as_deref(), Some("graphics"));
        assert_eq!(cfg.dymo.speed.as_deref(), Some("high"));
    }

    #[test]
    fn rejects_unknown_field() {
        let err = DriverPrintConfig::default()
            .with_opt_overrides(["dymo.bogus=1"])
            .unwrap_err();
        assert!(err.to_string().contains("invalid --driver-opt"), "{err}");
    }

    #[test]
    fn rejects_unknown_bag() {
        let err = DriverPrintConfig::default()
            .with_opt_overrides(["niimbot.task=b1"])
            .unwrap_err();
        assert!(err.to_string().contains("invalid --driver-opt"), "{err}");
    }

    #[test]
    fn rejects_malformed_spec() {
        let err = DriverPrintConfig::default()
            .with_opt_overrides(["no-equals"])
            .unwrap_err();
        assert!(err.to_string().contains("KEY=VALUE"), "{err}");
    }

    #[test]
    fn later_opt_wins() {
        let cfg = DriverPrintConfig::default()
            .with_opt_overrides(["dymo.speed=high", "dymo.speed=normal"])
            .unwrap();
        assert_eq!(cfg.dymo.speed.as_deref(), Some("normal"));
    }
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
            driver: DriverPrintConfig::default(),
            dither: "auto".into(),
            protocol: None,
            backend: "chromium".into(),
            bluetooth: None,
            serial: None,
            usb: None,
            network: None,
            driver_variant: None,
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
