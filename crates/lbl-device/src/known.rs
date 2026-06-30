//! A small table mapping known USB vendor/product ids to printer models and
//! protocols, used to suggest a driver for a discovered device.

use lbl_core::printer::Protocol;

/// A known USB printer entry.
#[derive(Debug, Clone, Copy)]
pub struct KnownUsbPrinter {
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id (`None` matches any product for the vendor).
    pub product_id: Option<u16>,
    /// Manufacturer brand.
    pub brand: &'static str,
    /// Model name.
    pub model: &'static str,
    /// Suggested protocol.
    pub protocol: Protocol,
}

/// The built-in known-printer table. Intentionally small; extend as devices are
/// verified.
pub const KNOWN_USB_PRINTERS: &[KnownUsbPrinter] = &[
    KnownUsbPrinter {
        vendor_id: 0x0922,
        product_id: Some(0x1001),
        brand: "DYMO",
        model: "LabelManager PnP",
        protocol: Protocol::Dymo,
    },
    // LabelWriter 550 series (raster protocol). VID/PIDs per the LW 550
    // Technical Reference.
    KnownUsbPrinter {
        vendor_id: 0x0922,
        product_id: Some(0x0028),
        brand: "DYMO",
        model: "LabelWriter 550",
        protocol: Protocol::DymoLw,
    },
    KnownUsbPrinter {
        vendor_id: 0x0922,
        product_id: Some(0x0029),
        brand: "DYMO",
        model: "LabelWriter 550 Turbo",
        protocol: Protocol::DymoLw,
    },
    KnownUsbPrinter {
        vendor_id: 0x0922,
        product_id: Some(0x002A),
        brand: "DYMO",
        model: "LabelWriter 5XL",
        protocol: Protocol::DymoLw,
    },
    KnownUsbPrinter {
        vendor_id: 0x0922,
        product_id: None,
        brand: "DYMO",
        model: "LabelWriter/LabelManager",
        protocol: Protocol::Dymo,
    },
    // NIIMBOT D11/D110-family printers (Protocol::Niimbot) connect over a USB
    // CDC-ACM serial port (e.g. /dev/ttyACM0), not the USB bulk-out path this
    // table drives, and expose no stable bulk VID/PID. They are reached via the
    // bidirectional `SerialTransport` (pick "niimbot" as the protocol and pass a
    // serial path) rather than auto-suggested here; their media lives in the
    // bundled catalog.
];

/// Find the most specific known entry for a vendor/product id.
pub fn match_usb(vendor_id: u16, product_id: u16) -> Option<&'static KnownUsbPrinter> {
    // Prefer an exact product match, then a vendor-wildcard match.
    KNOWN_USB_PRINTERS
        .iter()
        .find(|k| k.vendor_id == vendor_id && k.product_id == Some(product_id))
        .or_else(|| {
            KNOWN_USB_PRINTERS
                .iter()
                .find(|k| k.vendor_id == vendor_id && k.product_id.is_none())
        })
}
