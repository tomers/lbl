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
    /// How the device is connected.
    pub connection: String,
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
            })
        })
        .collect()
}

/// Stub when USB support is compiled out.
#[cfg(not(feature = "usb"))]
pub fn discover_usb() -> Vec<DiscoveredPrinter> {
    Vec::new()
}
