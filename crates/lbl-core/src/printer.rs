//! Printer models, transports, capabilities, and persisted profiles.

use serde::{Deserialize, Serialize};

use crate::units::Dpi;

/// A recognized printing protocol/language. Proprietary protocols (e.g. DYMO)
/// and industry-standard ones (ESC/POS, ZPL, TSPL) are both first-class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// DYMO LabelManager proprietary tape protocol (vertical print head).
    Dymo,
    /// DYMO LabelWriter 550-series raster protocol (structured print job).
    DymoLw,
    /// DYMO LabelWriter 450-series classic raster (SYN rows; not LW550 job framing).
    #[serde(rename = "dymolwclassic")]
    DymoLwClassic,
    /// ESC/POS thermal protocol.
    EscPos,
    /// Epson ESC/Label (ColorWorks; ZPL II–compatible with Epson media layout).
    #[serde(rename = "esclabel")]
    EscLabel,
    /// Phomemo M02-class (ESC/POS raster with vendor `1F 11` framing).
    Phomemo,
    /// Phomemo M02X proprietary framing (not M02 ESC/POS).
    #[serde(rename = "phomemom02x")]
    PhomemoM02x,
    /// Phomemo M110/M120/M220 business labelers (speed/density/`1F 11` media + `GS v 0`).
    #[serde(rename = "phomemom110")]
    PhomemoM110,
    /// Phomemo D30/Q30 mini label makers (vendor bring-up + `GS v 0`).
    #[serde(rename = "phomemod30")]
    PhomemoD30,
    /// Zebra Programming Language.
    Zpl,
    /// TSC Printer Language.
    Tspl,
    /// Bixolon SLCS (Samsung Label Command Set) label language.
    Slcs,
    /// Godex EZPL (EZ Printer Language).
    Ezpl,
    /// SATO Barcode Printer Language (SBPL).
    Sbpl,
    /// Honeywell / Datamax-O'Neil / Citizen DPL (Datamax Programming Language).
    Dpl,
    /// Toshiba TEC TPCL (TEC Printer Command Language).
    Tpcl,
    /// NIIMBOT thermal label protocol (packet-framed; D11/D110 family).
    Niimbot,
    /// DYMO LetraTag LT-200B Bluetooth LE chunked-GATT protocol (not LabelManager).
    LetraTag,
    /// Brother QL-series raster protocol (QL-800 / QL-810W / QL-820NWB(c), …).
    BrotherQl,
    /// Brother P-touch / TZe tape raster protocol (PT-P700 / H500 / E500 family).
    BrotherPt,
    /// Graphtec / Silhouette GPGL plotter language (craft cutters).
    #[serde(rename = "gpgl")]
    Gpgl,
    /// A virtual printer that "prints" to an image file instead of hardware.
    ///
    /// The concrete output format (PNG, BMP, ...) is the printer's selected
    /// *media type*, configured on the driver rather than encoded in the
    /// protocol itself.
    Virtual,
    /// A virtual printer that "prints" the raster to the terminal as text.
    ///
    /// Like [`Virtual`](Protocol::Virtual) it targets a human rather than
    /// hardware; the dithered bitmap is rendered as Unicode half-block art.
    Console,
    /// A virtual printer that writes a browser-viewable HTML gallery of the
    /// dithered rasters (PNG images alongside an `index.html` page).
    ///
    /// Like [`Console`](Protocol::Console), this is for human preview rather
    /// than hardware; it avoids downsampling large labels to terminal columns.
    Html,
}

impl Protocol {
    /// Whether this protocol drives a physical print head with a fixed width
    /// (so landscape content must be turned a quarter-turn onto the head).
    ///
    /// On-screen sinks ([`Virtual`](Protocol::Virtual) image files and the
    /// [`Console`](Protocol::Console) / [`Html`](Protocol::Html) previews)
    /// target a human instead, so they show the label in its reading
    /// orientation rather than the head's.
    pub fn targets_print_head(self) -> bool {
        !matches!(
            self,
            Protocol::Virtual | Protocol::Console | Protocol::Html | Protocol::Gpgl
        )
    }

    /// Whether [`MonoBitmap::width`] runs along the feed direction.
    ///
    /// LabelManager tape ([`Dymo`](Protocol::Dymo)) and LetraTag consume the
    /// bitmap with width = feed and height = head (column-major / sample-pattern
    /// layout). LabelWriter raster ([`DymoLw`](Protocol::DymoLw) /
    /// [`DymoLwClassic`](Protocol::DymoLwClassic)) and other row-oriented
    /// drivers (NIIMBOT, ESC/POS, ZPL, …) use width = head and height = feed.
    pub fn bitmap_width_is_feed(self) -> bool {
        matches!(self, Protocol::Dymo | Protocol::LetraTag)
    }
}

/// How the toolchain reaches a printer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Transport {
    /// A USB device, addressed by vendor/product id and optional serial.
    Usb {
        /// USB vendor id.
        vendor_id: u16,
        /// USB product id.
        product_id: u16,
        /// Optional serial number for disambiguation/persistence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        serial: Option<String>,
    },
    /// A network printer reachable over TCP.
    Network {
        /// Host or IP address.
        host: String,
        /// TCP port (commonly 9100 for raw printing).
        port: u16,
    },
    /// A bidirectional serial port (USB CDC-ACM, e.g. `/dev/ttyACM0`), used by
    /// printers that handshake — such as the NIIMBOT B-series.
    Serial {
        /// Serial device path (`/dev/ttyACM0`, `COM3`, ...).
        path: String,
        /// Baud rate (NIIMBOT printers use 115200).
        #[serde(default = "default_serial_baud")]
        baud: u32,
    },
    /// A bidirectional Bluetooth Low Energy (GATT) link, used by cable-less
    /// printers — such as the NIIMBOT pocket D-series (D11, D110, ...).
    Ble {
        /// Advertised local name or address substring used to find the device
        /// (e.g. `D110` or `D110-1A2B3C4D`).
        name: String,
    },
    /// Browser-side delivery (WebUSB, Web Serial, or Web Bluetooth). Device I/O
    /// runs in the user's browser; the server only encodes label bytes.
    Browser {
        /// User-facing connection: `usb` or `bluetooth`.
        connection: String,
        /// Internal API: `webusb`, `web_serial`, or `web_bluetooth`.
        api: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vendor_id: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        product_id: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        serial_number: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ble_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ble_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_label: Option<String>,
    },
}

/// Default baud rate for serial printers (NIIMBOT family).
fn default_serial_baud() -> u32 {
    115_200
}

/// A stable identifier for a printer the user owns, used as the config key so
/// configuration persists across disconnects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(pub String);

/// A known printer model definition (independent of any physical instance).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceModel {
    /// Manufacturer, e.g. "DYMO".
    pub brand: String,
    /// Model name, e.g. "LabelWriter 550".
    pub model: String,
    /// Protocol the model speaks.
    pub protocol: Protocol,
    /// Static capabilities of the model.
    pub capabilities: DeviceCapabilities,
}

fn default_true() -> bool {
    true
}

/// What a printer can do; used by drivers and the spooler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    /// Native print resolution.
    pub dpi: Dpi,
    /// Maximum printable width across the head, in millimeters.
    ///
    /// Physical media may be wider (`Media::width_mm`); layout/encode clamp to
    /// this value (and optional [`Self::head_printable_height_mm`]), and preview
    /// pads the leftover stock margins.
    pub max_width_mm: f64,
    /// Whether the printer can cut between jobs/items.
    #[serde(default)]
    pub supports_cut: bool,
    /// Whether the cutter can half-cut (laminate only, leave backing intact).
    ///
    /// Implies [`Self::supports_cut`]. Guillotine-only chassis leave this false.
    #[serde(default)]
    pub supports_half_cut: bool,
    /// Whether the printer deposits more than one ink color (full-color inkjet
    /// or dual-ink thermal). When set, the encode pipeline may supply a color
    /// PNG alongside the mono bitmap for drivers that register full-color
    /// graphics (e.g. ESC/Label `~DY`).
    #[serde(default)]
    pub supports_color: bool,
    /// Whether the printer reports loaded media for auto-detection.
    #[serde(default)]
    pub reports_media: bool,
    /// Blank feed before raster content when encoding. Some tape printers omit
    /// this because the head already sits past the last cut; preview may still
    /// show that offset using [`feed_trail_mm`] when lead is unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_lead_mm: Option<f64>,
    /// Head-to-cutter distance along the feed (e.g. ~8.1 mm on DYMO LabelManager).
    /// Preview can show symmetric head offset and content-boundary markers;
    /// drivers may feed extra blank columns so the cut lands with matching margins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_trail_mm: Option<f64>,
    /// Mirror content along the feed axis when encoding (mechanical/orientation).
    #[serde(default)]
    pub feed_reverse: bool,
    /// Inkable height across the head when narrower than the loaded tape
    /// (laminate / dead zones on each edge). Layout fits content to this band;
    /// drivers pad to the protocol column height as needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_printable_height_mm: Option<f64>,
    /// Brother QL: emit `ESC i K` expanded mode (cut-at-end / two-color / hi-res).
    /// Older chassis (QL-500 / QL-550) omit this opcode per Brother's raster ref.
    #[serde(default = "default_true")]
    pub supports_expanded_mode: bool,
    /// Brother QL: emit `ESC i A` (cut every N) when auto-cut is on.
    /// QL-500/550/650TD lack this command; QL-560+ support it.
    #[serde(default = "default_true")]
    pub supports_cut_every: bool,
    /// Brother QL: emit `ESC i a 01` raster-mode switch.
    /// Required only on QL-580N / 650TD / 1050 / 1060N (and later dual-mode bodies).
    #[serde(default = "default_true")]
    pub emit_raster_mode_switch: bool,
    /// Brother QL: override leading invalidate (`0x00`) length.
    /// When unset, the driver picks a family default (see brother-ql / brother-pt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidate_bytes: Option<u32>,
    /// Brother QL/PT: emit TIFF PackBits (`M 02`) raster rows when beneficial.
    /// QL-800 firmware lacks compression; leave false there.
    #[serde(default)]
    pub supports_packbits: bool,
    /// Brother QL/PT: high-resolution feed mode (`ESC i K` bit 4 on QL, bit 6 on PT).
    #[serde(default)]
    pub supports_high_resolution: bool,
    /// Device can emit a zero-content pre-cut prologue (eject ≈ [`Self::feed_trail_mm`]).
    ///
    /// Capability only — preference defaults via [`Self::precut_default`].
    #[serde(default)]
    pub supports_precut: bool,
    /// Initial job preference when `JobSpec::precut` is unset.
    /// Catalog devices that support pre-cut typically set this `true`.
    #[serde(default)]
    pub precut_default: bool,
    /// Minimum lead the chassis can honor after a pre-cut (protocol clamp), mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_lead_min_mm: Option<f64>,
}

impl Default for DeviceCapabilities {
    fn default() -> Self {
        Self {
            dpi: Dpi(300.0),
            max_width_mm: 56.0,
            supports_cut: false,
            supports_half_cut: false,
            supports_color: false,
            reports_media: false,
            feed_lead_mm: None,
            feed_trail_mm: None,
            feed_reverse: false,
            head_printable_height_mm: None,
            supports_expanded_mode: true,
            supports_cut_every: true,
            emit_raster_mode_switch: true,
            invalidate_bytes: None,
            supports_packbits: false,
            supports_high_resolution: false,
            supports_precut: false,
            precut_default: false,
            feed_lead_min_mm: None,
        }
    }
}

/// A user-owned printer instance plus its desired (persisted) configuration.
///
/// This is what `lbl-config` stores so that a disconnected printer keeps its
/// settings and is restored on reconnect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    /// Stable identifier (config key).
    pub id: DeviceId,
    /// Human-friendly name.
    pub name: String,
    /// Which known model this instance is.
    pub model: DeviceModel,
    /// How to reach it.
    pub transport: Transport,
    /// Whether this is the user's default printer.
    #[serde(default)]
    pub default: bool,
    /// Default media SKU/key (resolved via `lbl-catalog`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_media: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_on_screen_sinks_lack_a_print_head() {
        for p in [
            Protocol::Dymo,
            Protocol::DymoLw,
            Protocol::DymoLwClassic,
            Protocol::EscPos,
            Protocol::EscLabel,
            Protocol::Phomemo,
            Protocol::PhomemoM02x,
            Protocol::PhomemoM110,
            Protocol::PhomemoD30,
            Protocol::Zpl,
            Protocol::Tspl,
            Protocol::Slcs,
            Protocol::Ezpl,
            Protocol::Sbpl,
            Protocol::Dpl,
            Protocol::Tpcl,
            Protocol::Niimbot,
            Protocol::LetraTag,
            Protocol::BrotherQl,
            Protocol::BrotherPt,
        ] {
            assert!(p.targets_print_head(), "{p:?} should target a head");
        }
        assert!(!Protocol::Virtual.targets_print_head());
        assert!(!Protocol::Console.targets_print_head());
        assert!(!Protocol::Html.targets_print_head());
        assert!(!Protocol::Gpgl.targets_print_head());
    }

    #[test]
    fn dymo_labelwriter_protocols_are_row_oriented() {
        assert!(!Protocol::DymoLw.bitmap_width_is_feed());
        assert!(!Protocol::DymoLwClassic.bitmap_width_is_feed());
    }

    #[test]
    fn tape_protocols_are_feed_oriented() {
        assert!(Protocol::Dymo.bitmap_width_is_feed());
        assert!(Protocol::LetraTag.bitmap_width_is_feed());
    }
}
