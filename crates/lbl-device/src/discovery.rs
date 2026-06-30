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
    /// CDC-ACM / serial port, `"ble"` for Bluetooth Low Energy).
    pub connection: String,
    /// Identifier to hand to the matching transport flag: a serial device path
    /// (e.g. `/dev/ttyACM0` or `COM3`) for `--serial`, or the advertised
    /// name/address for `--bluetooth`. `None` for USB bulk devices.
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

/// How long [`discover_ble`] scans for advertising peripherals.
#[cfg(feature = "ble")]
const BLE_DISCOVERY_SCAN_SECS: u64 = 10;

/// Scan for nearby Bluetooth Low Energy label printers (NIIMBOT D-series).
///
/// Unlike USB/serial enumeration this performs a short radio scan (a few
/// seconds) and reports peripherals whose advertised name looks like a NIIMBOT
/// printer. The advertised name is returned in `path` to pass to `--bluetooth`.
///
/// Returns an empty list when the `ble` feature is disabled, no adapter is
/// present, or the scan fails.
#[cfg(feature = "ble")]
pub fn discover_ble() -> Vec<DiscoveredPrinter> {
    use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
    use btleplug::platform::Manager;
    use std::time::Duration;

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!("ble runtime init failed: {e}");
            return Vec::new();
        }
    };

    rt.block_on(async {
        let manager = match Manager::new().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("ble manager init failed: {e}");
                return Vec::new();
            }
        };
        let adapter = match manager.adapters().await {
            Ok(a) => a.into_iter().next(),
            Err(e) => {
                tracing::warn!("ble adapter enumeration failed: {e}");
                return Vec::new();
            }
        };
        let Some(adapter) = adapter else {
            return Vec::new();
        };
        if let Err(e) = adapter.start_scan(ScanFilter::default()).await {
            tracing::warn!("ble scan failed: {e}");
            return Vec::new();
        }
        tokio::time::sleep(Duration::from_secs(BLE_DISCOVERY_SCAN_SECS)).await;
        let peripherals = adapter.peripherals().await.unwrap_or_default();
        adapter.stop_scan().await.ok();

        let mut out = Vec::new();
        for p in peripherals {
            let addr = p.address().to_string();
            let props = match p.properties().await {
                Ok(Some(props)) => props,
                _ => continue,
            };
            if !crate::ble::props_look_like_niimbot(&props, &addr) {
                continue;
            }
            let name = props
                .local_name
                .clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| addr.clone());
            out.push(DiscoveredPrinter {
                vendor_id: None,
                product_id: None,
                serial: None,
                brand: Some("NIIMBOT".to_string()),
                model: Some(name.clone()),
                protocol: Some(Protocol::Niimbot),
                connection: "ble".to_string(),
                path: Some(name),
            });
        }
        out
    })
}

/// Stub when BLE support is compiled out.
#[cfg(not(feature = "ble"))]
pub fn discover_ble() -> Vec<DiscoveredPrinter> {
    Vec::new()
}

/// Enumerate every connected printer candidate: USB bulk devices
/// ([`discover_usb`]), USB serial ports ([`discover_serial`]), and — when the
/// `ble` feature is enabled — nearby Bluetooth LE printers ([`discover_ble`]).
///
/// Recognized devices come first (those with a suggested `protocol`), so a
/// caller picking the first entry favors a known printer.
///
/// Note: with the `ble` feature enabled this performs a short Bluetooth scan
/// (a few seconds) on each call.
pub fn discover() -> Vec<DiscoveredPrinter> {
    let mut all = discover_usb();
    all.extend(discover_serial());
    all.extend(discover_ble());
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
