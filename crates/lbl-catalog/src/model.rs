//! Data model for catalog entries.

use lbl_core::media::{Adhesive, Material, Media, MediaColor, MediaLength};
use lbl_core::units::Dpi;
use serde::{Deserialize, Serialize};

/// The physical specification of a media SKU, independent of any printer's
/// resolution. Combine with a [`Dpi`] to produce a device-ready [`Media`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaSpec {
    /// Printable width in millimeters.
    pub width_mm: f64,
    /// Fixed (die-cut) or continuous length.
    pub length: MediaLength,
    /// Material.
    #[serde(default)]
    pub material: Material,
    /// Adhesive.
    #[serde(default)]
    pub adhesive: Adhesive,
    /// Base color.
    #[serde(default)]
    pub color: MediaColor,
}

impl MediaSpec {
    /// Resolve this spec into a device-ready [`Media`] at the given resolution.
    pub fn to_media(&self, dpi: Dpi) -> Media {
        Media {
            width_mm: self.width_mm,
            length: self.length,
            dpi,
            margins: Default::default(),
            material: self.material,
            adhesive: self.adhesive,
            color: self.color,
        }
    }
}

/// Image metadata for a catalog entry. Redistribution is gated on the license;
/// local caching is always allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Source URL of the product image.
    pub url: String,
    /// License/source descriptor (e.g. "manufacturer-press", "cc-by-4.0",
    /// "unknown").
    #[serde(default = "unknown_license")]
    pub license: String,
    /// Required attribution text, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// Whether the image may be downloaded *and redistributed* with the
    /// catalog. When false, the UI hotlinks the URL only.
    #[serde(default)]
    pub redistributable: bool,
}

fn unknown_license() -> String {
    "unknown".to_string()
}

/// A single catalog entry describing a media SKU.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// Manufacturer brand, e.g. "DYMO".
    pub brand: String,
    /// One or more keys/SKUs that resolve to this entry (aliases). The first is
    /// treated as canonical.
    pub keys: Vec<String>,
    /// Human-friendly display name.
    pub name: String,
    /// Physical media specification.
    pub media: MediaSpec,
    /// Optional product image (license-aware).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageInfo>,
    /// Optional purchase URL (an affiliate tag may be appended at display time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_url: Option<String>,
    /// Printer models this media is compatible with (matched case-insensitively
    /// as substrings, e.g. "LabelWriter 550", "LabelWriter").
    #[serde(default)]
    pub compatible: Vec<String>,
}

impl CatalogEntry {
    /// The canonical key (first in `keys`).
    pub fn canonical_key(&self) -> &str {
        self.keys.first().map(String::as_str).unwrap_or("")
    }

    /// Whether any of this entry's keys matches `key` (case-insensitive).
    pub fn matches_key(&self, key: &str) -> bool {
        self.keys.iter().any(|k| k.eq_ignore_ascii_case(key))
    }

    /// Whether this media is compatible with the given printer model string.
    pub fn is_compatible_with(&self, printer_model: &str) -> bool {
        let needle = printer_model.to_ascii_lowercase();
        self.compatible
            .iter()
            .any(|c| needle.contains(&c.to_ascii_lowercase()) || c.eq_ignore_ascii_case(printer_model))
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CatalogFile {
    #[serde(default)]
    pub entries: Vec<CatalogEntry>,
}
