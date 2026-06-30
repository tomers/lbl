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
    /// ESC/POS thermal protocol.
    EscPos,
    /// Zebra Programming Language.
    Zpl,
    /// TSC Printer Language.
    Tspl,
    /// NIIMBOT thermal label protocol (packet-framed; D11/D110 family).
    Niimbot,
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
}

impl Protocol {
    /// Whether this protocol drives a physical print head with a fixed width
    /// (so landscape content must be turned a quarter-turn onto the head).
    ///
    /// On-screen sinks ([`Virtual`](Protocol::Virtual) image files and the
    /// [`Console`](Protocol::Console) preview) target a human instead, so they
    /// show the label in its reading orientation rather than the head's.
    pub fn targets_print_head(self) -> bool {
        !matches!(self, Protocol::Virtual | Protocol::Console)
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
}

/// Default baud rate for serial printers (NIIMBOT family).
fn default_serial_baud() -> u32 {
    115_200
}

/// A stable identifier for a printer the user owns, used as the config key so
/// configuration persists across disconnects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrinterId(pub String);

/// A known printer model definition (independent of any physical instance).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterModel {
    /// Manufacturer, e.g. "DYMO".
    pub brand: String,
    /// Model name, e.g. "LabelWriter 550".
    pub model: String,
    /// Protocol the model speaks.
    pub protocol: Protocol,
    /// Static capabilities of the model.
    pub capabilities: PrinterCapabilities,
}

/// What a printer can do; used by drivers and the spooler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterCapabilities {
    /// Native print resolution.
    pub dpi: Dpi,
    /// Maximum printable width across the head, in millimeters.
    pub max_width_mm: f64,
    /// Whether the printer can cut between jobs/items.
    pub supports_cut: bool,
    /// Whether the printer reports loaded media for auto-detection.
    pub reports_media: bool,
}

impl Default for PrinterCapabilities {
    fn default() -> Self {
        Self {
            dpi: Dpi(300.0),
            max_width_mm: 56.0,
            supports_cut: false,
            reports_media: false,
        }
    }
}

/// A user-owned printer instance plus its desired (persisted) configuration.
///
/// This is what `lbl-config` stores so that a disconnected printer keeps its
/// settings and is restored on reconnect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrinterProfile {
    /// Stable identifier (config key).
    pub id: PrinterId,
    /// Human-friendly name.
    pub name: String,
    /// Which known model this instance is.
    pub model: PrinterModel,
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
            Protocol::EscPos,
            Protocol::Zpl,
            Protocol::Tspl,
            Protocol::Niimbot,
        ] {
            assert!(p.targets_print_head(), "{p:?} should target a head");
        }
        assert!(!Protocol::Virtual.targets_print_head());
        assert!(!Protocol::Console.targets_print_head());
    }
}
