//! Discovery of connected printers.

use lbl_core::printer::Protocol;
use serde::{Deserialize, Serialize};

/// A printer discovered on the system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPrinter {
    /// USB vendor id (USB devices only).
    pub vendor_id: Option<u16>,
    /// USB product id (USB devices only).
    pub product_id: Option<u16>,
    /// Serial number, if reported.
    pub serial: Option<String>,
    /// Brand, if recognized.
    pub brand: Option<String>,
    /// Model, if recognized.
    pub model: Option<String>,
    /// Suggested protocol, if recognized.
    pub protocol: Option<Protocol>,
    /// How the device is connected (`"usb"` for USB bulk, `"serial"` for a USB
    /// CDC-ACM / serial port).
    pub connection: String,
    /// Serial device path to pass to `--serial` (serial connections only), e.g.
    /// `/dev/ttyACM0` or `COM3`. `None` for USB bulk devices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Enumerate connected USB printers, matching against the known-printer table.
///
/// Returns an empty list when the `usb` feature is disabled or no devices are
/// present.
#[cfg(feature = "usb")]
pub fn discover_usb() -> Vec<DiscoveredPrinter> {
    let devices = match nusb::list_devices() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("usb enumeration failed: {e}");
            return Vec::new();
        }
    };

    devices
        .filter_map(|d| {
            let vid = d.vendor_id();
            let pid = d.product_id();
            let known = crate::known::match_usb(vid, pid)?;
            Some(DiscoveredPrinter {
                vendor_id: Some(vid),
                product_id: Some(pid),
                serial: d.serial_number().map(|s| s.to_string()),
                brand: Some(known.brand.to_string()),
                model: Some(known.model.to_string()),
                protocol: Some(known.protocol),
                connection: "usb".to_string(),
                path: None,
            })
        })
        .collect()
}

/// Stub when USB support is compiled out.
#[cfg(not(feature = "usb"))]
pub fn discover_usb() -> Vec<DiscoveredPrinter> {
    Vec::new()
}

/// Enumerate serial ports (USB CDC-ACM and friends) that could be a printer.
///
/// NIIMBOT D-series and other serial printers connect as a USB serial port
/// (e.g. `/dev/ttyACM0`, `/dev/ttyUSB0`, or `COM3`) and expose no stable bulk
/// VID/PID, so they don't show up in [`discover_usb`]. This lists every USB
/// serial port the OS reports, filling in `brand`/`model`/`protocol` when the
/// device's USB descriptor strings identify it (e.g. a "NIIMBOT" manufacturer),
/// and leaving them `None` for unrecognized ports so the user can still see the
/// candidate path to pass to `--serial`.
///
/// Returns an empty list when the `serial` feature is disabled or enumeration
/// fails.
#[cfg(feature = "serial")]
pub fn discover_serial() -> Vec<DiscoveredPrinter> {
    let ports = match serialport::available_ports() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("serial enumeration failed: {e}");
            return Vec::new();
        }
    };

    ports
        .into_iter()
        .filter_map(|port| {
            // Only USB serial ports are plausible printers; skip PCI/Bluetooth
            // and unknown port types.
            let usb = match port.port_type {
                serialport::SerialPortType::UsbPort(info) => info,
                _ => return None,
            };

            // Recognize known printers by their reported USB strings. NIIMBOT
            // firmwares report "NIIMBOT" (sometimes the model, e.g. "D110") in
            // the manufacturer/product descriptor.
            let descriptor = format!(
                "{} {}",
                usb.manufacturer.as_deref().unwrap_or(""),
                usb.product.as_deref().unwrap_or("")
            )
            .to_ascii_lowercase();
            let (brand, model, protocol) = if descriptor.contains("niimbot") {
                (
                    Some("NIIMBOT".to_string()),
                    usb.product.clone(),
                    Some(Protocol::Niimbot),
                )
            } else {
                (None, None, None)
            };

            Some(DiscoveredPrinter {
                vendor_id: Some(usb.vid),
                product_id: Some(usb.pid),
                serial: usb.serial_number,
                brand,
                model,
                protocol,
                connection: "serial".to_string(),
                path: Some(port.port_name),
            })
        })
        .collect()
}

/// Stub when serial support is compiled out.
#[cfg(not(feature = "serial"))]
pub fn discover_serial() -> Vec<DiscoveredPrinter> {
    Vec::new()
}

/// Enumerate every connected printer candidate, both USB bulk devices
/// ([`discover_usb`]) and USB serial ports ([`discover_serial`]).
///
/// Recognized devices come first (those with a suggested `protocol`), so a
/// caller picking the first entry favors a known printer.
pub fn discover() -> Vec<DiscoveredPrinter> {
    let mut all = discover_usb();
    all.extend(discover_serial());
    all.sort_by_key(|p| p.protocol.is_none());
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_printer_serializes_path_and_omits_it_for_usb() {
        let serial = DiscoveredPrinter {
            vendor_id: Some(0x1a86),
            product_id: Some(0x7523),
            serial: None,
            brand: Some("NIIMBOT".to_string()),
            model: Some("B1".to_string()),
            protocol: Some(Protocol::Niimbot),
            connection: "serial".to_string(),
            path: Some("/dev/ttyACM0".to_string()),
        };
        let json = serde_json::to_string(&serial).unwrap();
        assert!(json.contains("\"path\":\"/dev/ttyACM0\""));
        assert!(json.contains("\"connection\":\"serial\""));

        // USB bulk devices carry no serial path, which is omitted from the JSON.
        let usb = DiscoveredPrinter {
            path: None,
            connection: "usb".to_string(),
            ..serial
        };
        assert!(!serde_json::to_string(&usb).unwrap().contains("path"));
    }

    #[test]
    fn discover_does_not_panic() {
        // Enumeration may return nothing in CI/sandboxes, but must not panic and
        // must keep recognized printers (with a protocol) ahead of unknowns.
        let printers = discover();
        let first_unknown = printers.iter().position(|p| p.protocol.is_none());
        let last_known = printers.iter().rposition(|p| p.protocol.is_some());
        if let (Some(u), Some(k)) = (first_unknown, last_known) {
            assert!(u > k, "recognized printers should sort before unknown ones");
        }
    }
}
