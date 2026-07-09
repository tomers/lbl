//! Data model for catalog entries.

use lbl_core::media::{Adhesive, Material, Media, MediaColor, MediaLength};
use lbl_core::printer::{PrinterCapabilities, PrinterModel, Protocol};
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
    /// Manufacturer-reported product identifiers (e.g. NIIMBOT RFID oneCodes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub product_ids: Vec<String>,
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

    /// Whether this entry matches a manufacturer product id (exact match).
    pub fn matches_product_id(&self, id: &str) -> bool {
        self.product_ids.iter().any(|p| p == id)
    }
}

/// How to reach a printer model. The first entry is the preferred default when
/// no explicit transport flag is passed on the CLI. USB entries also identify
/// the model during device discovery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectionHint {
    /// Bluetooth Low Energy (NIIMBOT D-series and similar).
    Ble {
        /// Advertised name or address substring for `--bluetooth`.
        name: String,
    },
    /// USB serial / CDC-ACM (NIIMBOT B-series and similar).
    Serial {
        /// Optional fixed device path; when omitted, discovery picks the first
        /// matching serial port at print time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// USB bulk transfer.
    Usb {
        /// USB vendor id.
        vendor_id: u16,
        /// USB product id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        product_id: Option<u16>,
    },
    /// Raw TCP socket (typically port 9100).
    Network {
        /// Host or IP address.
        host: String,
        /// TCP port.
        port: u16,
    },
}

/// Resolved transport targets for dispatch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedTransport {
    /// Bluetooth LE target (`--bluetooth`).
    pub bluetooth: Option<String>,
    /// Serial port target (`--serial`).
    pub serial: Option<String>,
    /// USB bulk target (`vid:pid` hex, `--usb`).
    pub usb: Option<String>,
    /// Network target (`host:port`, `--network`).
    pub network: Option<String>,
}

impl ConnectionHint {
    /// Apply this hint to a [`ResolvedTransport`], overwriting any field it sets.
    pub fn apply_to(&self, out: &mut ResolvedTransport) {
        match self {
            Self::Ble { name } => out.bluetooth = Some(name.clone()),
            Self::Serial { path } => out.serial = path.clone(),
            Self::Usb {
                vendor_id,
                product_id,
            } => {
                out.usb = Some(match product_id {
                    Some(pid) => format!("{vendor_id:04x}:{pid:04x}"),
                    None => format!("{vendor_id:04x}"),
                });
            }
            Self::Network { host, port } => out.network = Some(format!("{host}:{port}")),
        }
    }

    /// Whether this is an exact USB product match (not a vendor wildcard).
    pub fn is_exact_usb_match(&self, vendor_id: u16, product_id: u16) -> bool {
        matches!(
            self,
            Self::Usb {
                vendor_id: vid,
                product_id: Some(pid),
            } if *vid == vendor_id && *pid == product_id
        )
    }
}

/// A known printer model in the catalog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterEntry {
    /// Manufacturer brand, e.g. "DYMO".
    pub brand: String,
    /// One or more keys/aliases that resolve to this entry (the first is
    /// canonical).
    pub keys: Vec<String>,
    /// Human-friendly display name.
    pub name: String,
    /// Protocol the model speaks.
    pub protocol: Protocol,
    /// Native print resolution in dots per inch.
    pub dpi: f64,
    /// Maximum printable width across the head, in millimeters.
    pub max_width_mm: f64,
    /// Whether the printer can cut between jobs/items.
    #[serde(default)]
    pub supports_cut: bool,
    /// Whether the printer outputs more than one ink color. Thermal 1-bit heads
    /// (DYMO, NIIMBOT, ESC/POS, …) leave this false. Preview sinks
    /// (`virtual`, `html`, `console`) are treated as color-capable in the UI.
    #[serde(default)]
    pub supports_color: bool,
    /// Whether the printer reports loaded media for auto-detection.
    #[serde(default)]
    pub reports_media: bool,
    /// Media catalog keys this printer can use.
    #[serde(default)]
    pub supported_media: Vec<String>,
    /// Default media catalog key, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_media: Option<String>,
    /// How to connect to this model (and, for USB, how to recognize it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionHint>,
    /// Blank feed before raster content (DYMO tape: lead margin along feed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_lead_mm: Option<f64>,
    /// Head-to-cutter gap along the feed (DYMO tape: ~8.1 mm on LabelManager).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_trail_mm: Option<f64>,
    /// Mirror content along the feed axis when encoding (DYMO tape).
    #[serde(default)]
    pub feed_reverse: bool,
}

impl PrinterEntry {
    /// The canonical key (first in `keys`).
    pub fn canonical_key(&self) -> &str {
        self.keys.first().map(String::as_str).unwrap_or("")
    }

    /// Whether any of this entry's keys matches `key` (case-insensitive).
    pub fn matches_key(&self, key: &str) -> bool {
        self.keys.iter().any(|k| k.eq_ignore_ascii_case(key))
    }

    /// Match strength for a free-form model string. Higher is better; `None` if
    /// no key matches. Exact key hits rank above substring matches; when the
    /// query appears inside a key, the matched key length wins (so
    /// "DYMO LabelWriter 550" prefers `LabelWriter 550` over `LabelWriter`).
    pub fn match_score(&self, printer_model: &str) -> Option<usize> {
        let needle = printer_model.to_ascii_lowercase();
        let mut best = None;
        for k in &self.keys {
            let key = k.to_ascii_lowercase();
            let score = if k.eq_ignore_ascii_case(printer_model) {
                key.len() + 1000
            } else if needle.contains(&key) {
                key.len()
            } else if key.contains(&needle) {
                needle.len()
            } else {
                continue;
            };
            best = Some(best.map_or(score, |b: usize| b.max(score)));
        }
        best
    }

    /// Whether this printer matches a free-form model string (case-insensitive
    /// substring match on keys, or exact key match).
    pub fn matches_model(&self, printer_model: &str) -> bool {
        self.match_score(printer_model).is_some()
    }

    /// Convert to a [`PrinterModel`] for profiles and drivers.
    pub fn to_printer_model(&self) -> PrinterModel {
        PrinterModel {
            brand: self.brand.clone(),
            model: self.canonical_key().to_string(),
            protocol: self.protocol,
            capabilities: self.encode_capabilities(),
        }
    }

    /// Static capabilities for encode/dispatch (protocol padding, DPI, etc.).
    pub fn encode_capabilities(&self) -> PrinterCapabilities {
        PrinterCapabilities {
            dpi: Dpi(self.dpi),
            max_width_mm: self.max_width_mm,
            supports_cut: self.supports_cut,
            reports_media: self.reports_media,
            feed_lead_mm: self.feed_lead_mm,
            feed_trail_mm: self.feed_trail_mm,
            feed_reverse: self.feed_reverse,
        }
    }

    /// Build transport targets from the first catalog connection hint.
    pub fn default_transport(&self) -> ResolvedTransport {
        let mut out = ResolvedTransport::default();
        if let Some(conn) = self.connections.first() {
            conn.apply_to(&mut out);
        }
        out
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CatalogFile {
    #[serde(default)]
    pub entries: Vec<CatalogEntry>,
    #[serde(default)]
    pub printers: Vec<PrinterEntry>,
}

/// Resolve encode-time capabilities from an optional catalog printer and media.
pub fn encode_capabilities_for(
    printer: Option<&PrinterEntry>,
    media: &Media,
    supports_cut: bool,
) -> PrinterCapabilities {
    match printer {
        Some(entry) => {
            let mut caps = entry.encode_capabilities();
            caps.supports_cut |= supports_cut;
            caps
        }
        None => PrinterCapabilities {
            dpi: media.dpi,
            max_width_mm: media.width_mm,
            supports_cut,
            ..Default::default()
        },
    }
}
