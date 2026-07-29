//! Data model for catalog entries.

use lbl_core::media::{Adhesive, Material, Media, MediaColor, MediaLength, MediaSense};
use lbl_core::printer::{DeviceCapabilities, DeviceModel, Protocol};
use lbl_core::units::Dpi;
use serde::{Deserialize, Serialize};

/// The physical specification of a media SKU, independent of any printer's
/// resolution. Combine with a [`Dpi`] to produce a device-ready [`Media`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaSpec {
    /// Physical stock width across the head, in millimeters.
    ///
    /// This is the marketed / die-cut / cassette width — not the printer's
    /// inkable band. When stock is wider than a printer's `max_width_mm` (or
    /// `head_printable_height_mm`), layout/encode clamp to the printable band
    /// and preview pads the unprintable margins.
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
    /// Dual-ink / two-color consumable (primary + secondary plane at encode).
    #[serde(default)]
    pub two_color: bool,
    /// Explicit gap / black-mark / continuous sensing for industrial dialects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sense: Option<MediaSense>,
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
            two_color: self.two_color,
            sense: Some(
                self.sense
                    .unwrap_or_else(|| MediaSense::inferred_from_length(self.length)),
            ),
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

/// USB presentation mode for dual-PID chassis (printer vs mass-storage / Editor Lite).
///
/// Some Brother / DYMO units re-enumerate under a sibling product ID that exposes
/// a removable disk (bundled software / Editor Lite), not a printable interface.
/// Catalog both PIDs so discovery can identify the device, and gate pairing on
/// [`UsbConnectionMode::MassStorage`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbConnectionMode {
    /// Printable USB interface (default when omitted in catalog TOML).
    #[default]
    Printer,
    /// Mass-storage / Editor Lite / modeswitch identity — not printable.
    MassStorage,
}

fn usb_mode_is_printer(mode: &UsbConnectionMode) -> bool {
    matches!(mode, UsbConnectionMode::Printer)
}

/// How to reach a printer model. The first **printable** entry is the preferred
/// default when no explicit transport flag is passed on the CLI (mass-storage
/// USB siblings are skipped). USB entries also identify the model during device
/// discovery.
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
        /// Printer vs mass-storage sibling identity (defaults to printer).
        #[serde(default, skip_serializing_if = "usb_mode_is_printer")]
        mode: UsbConnectionMode,
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
                ..
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
                ..
            } if *vid == vendor_id && *pid == product_id
        )
    }

    /// True when this USB hint is a mass-storage / Editor Lite sibling PID.
    pub fn is_mass_storage_usb(&self) -> bool {
        matches!(
            self,
            Self::Usb {
                mode: UsbConnectionMode::MassStorage,
                ..
            }
        )
    }

    /// USB connection mode when this hint is USB; `None` for other kinds.
    pub fn usb_mode(&self) -> Option<UsbConnectionMode> {
        match self {
            Self::Usb { mode, .. } => Some(*mode),
            _ => None,
        }
    }
}

/// How thoroughly a catalog printer model has been validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Maturity {
    /// Encode path and discovery exercised on real hardware we have on hand.
    Verified,
    /// Shares a protocol we have verified on hand; model itself not yet
    /// hardware-exercised by us.
    Supported,
    /// Protocol has not been exercised on real hardware by us yet.
    #[default]
    Experimental,
}

impl Maturity {
    /// Short label for UI / marketing tables.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Supported => "supported",
            Self::Experimental => "experimental",
        }
    }
}

/// Manufacturer support links for a printer model.
///
/// Prefer [`product_url`] (model-specific drivers / manuals / FAQs) at display
/// time; fall back to [`brand_url`] (brand support hub) when the manufacturer
/// has no stable per-model support page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DeviceSupport {
    /// Model-specific support page (drivers, manuals, FAQs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_url: Option<String>,
    /// Brand-level support hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand_url: Option<String>,
}

impl DeviceSupport {
    /// Whether both URL fields are unset.
    pub fn is_empty(&self) -> bool {
        self.product_url.is_none() && self.brand_url.is_none()
    }

    /// Preferred URL for UI: product page when set, otherwise brand hub.
    pub fn primary_url(&self) -> Option<&str> {
        self.product_url.as_deref().or(self.brand_url.as_deref())
    }
}

fn support_is_empty(support: &DeviceSupport) -> bool {
    support.is_empty()
}

/// Whether a catalog device is a label printer or a craft cutter/plotter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeviceRole {
    /// Thermal / inkjet label printer (default for existing catalog rows).
    #[default]
    Printer,
    /// Desktop cutting machine (vector GPGL / similar).
    Cutter,
}

/// A known device model in the catalog (printer or cutter).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceEntry {
    /// Manufacturer brand, e.g. "DYMO".
    pub brand: String,
    /// One or more keys/aliases that resolve to this entry (the first is
    /// canonical).
    pub keys: Vec<String>,
    /// Human-friendly display name.
    pub name: String,
    /// Protocol the model speaks.
    pub protocol: Protocol,
    /// Printer vs cutter. Defaults to printer for thermal catalog rows.
    #[serde(default)]
    pub role: DeviceRole,
    /// How thoroughly this model has been validated.
    #[serde(default)]
    pub maturity: Maturity,
    /// Encode-time capabilities (DPI, cut, feed, Brother dialect flags, …).
    ///
    /// Flattened so catalog TOML / device API JSON keep top-level keys
    /// (`dpi`, `max_width_mm`, …) matching historical `[[devices]]` rows.
    #[serde(flatten)]
    pub capabilities: DeviceCapabilities,
    /// Whether the printer supports a host-initiated soft reboot of the print
    /// engine (recovery when wedged / lock stuck). Catalog/UI gate only — not
    /// an encode-time capability.
    #[serde(default)]
    pub supports_soft_reboot: bool,
    /// Media catalog keys this printer can use.
    #[serde(default)]
    pub supported_media: Vec<String>,
    /// Default media catalog key, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_media: Option<String>,
    /// How to connect to this model (and, for USB, how to recognize it).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionHint>,
    /// Optional manufacturer support links (product page and/or brand hub).
    #[serde(default, skip_serializing_if = "support_is_empty")]
    pub support: DeviceSupport,
}

impl DeviceEntry {
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

    /// Convert to a [`DeviceModel`] for profiles and drivers.
    pub fn to_printer_model(&self) -> DeviceModel {
        DeviceModel {
            brand: self.brand.clone(),
            model: self.canonical_key().to_string(),
            protocol: self.protocol,
            capabilities: self.capabilities.clone(),
        }
    }

    /// Static capabilities for encode/dispatch (protocol padding, DPI, etc.).
    pub fn encode_capabilities(&self) -> DeviceCapabilities {
        self.capabilities.clone()
    }

    /// Build transport targets from the first printable catalog connection hint.
    ///
    /// Mass-storage USB siblings are skipped so CLI defaults prefer a printable
    /// PID when both are catalogued.
    pub fn default_transport(&self) -> ResolvedTransport {
        let mut out = ResolvedTransport::default();
        let preferred = self
            .connections
            .iter()
            .find(|c| !c.is_mass_storage_usb())
            .or_else(|| self.connections.first());
        if let Some(conn) = preferred {
            conn.apply_to(&mut out);
        }
        out
    }

    /// USB connection mode for an exact VID/PID match on this device, if any.
    pub fn usb_connection_mode(
        &self,
        vendor_id: u16,
        product_id: u16,
    ) -> Option<UsbConnectionMode> {
        self.connections.iter().find_map(|c| match c {
            ConnectionHint::Usb {
                vendor_id: vid,
                product_id: Some(pid),
                mode,
            } if *vid == vendor_id && *pid == product_id => Some(*mode),
            _ => None,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct CatalogFile {
    #[serde(default)]
    pub entries: Vec<CatalogEntry>,
    #[serde(default)]
    pub devices: Vec<DeviceEntry>,
}

/// Resolve encode-time capabilities from an optional catalog printer and media.
pub fn encode_capabilities_for(
    printer: Option<&DeviceEntry>,
    media: &Media,
    supports_cut: bool,
) -> DeviceCapabilities {
    match printer {
        Some(entry) => {
            let mut caps = entry.encode_capabilities();
            caps.supports_cut |= supports_cut;
            caps
        }
        None => DeviceCapabilities {
            dpi: media.dpi,
            max_width_mm: media.width_mm,
            supports_cut,
            ..Default::default()
        },
    }
}
