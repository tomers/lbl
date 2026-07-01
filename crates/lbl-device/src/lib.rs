//! Printer discovery and transport for `lbl`.
//!
//! - [`discovery`] enumerates connected printers — USB bulk devices via
//!   [nusb](https://docs.rs/nusb), USB serial ports (NIIMBOT B-series and other
//!   CDC-ACM printers) via [serialport](https://docs.rs/serialport), and — with
//!   the `ble` feature — nearby Bluetooth LE printers via [btleplug](https://docs.rs/btleplug)
//!   — and maps them to known models/protocols.
//! - [`transport`] delivers encoded bytes to a printer over USB bulk transfer,
//!   a raw TCP socket (network printers, typically port 9100), a bidirectional
//!   serial port (USB CDC-ACM, e.g. NIIMBOT B-series), or a bidirectional
//!   Bluetooth LE GATT link (NIIMBOT D-series).
//! - [`media`] resolves the media loaded in a printer: auto-detection when the
//!   device reports it, otherwise an explicit override.
//!
//! USB and serial support are behind the default `usb`/`serial` features so the
//! crate still builds in environments without that access. Bluetooth LE is
//! behind the opt-in `ble` feature (it pulls in `btleplug`, plus a vendored
//! `libdbus` on Linux).

pub mod discovery;
pub mod media;
pub mod transport;
pub mod troubleshoot;

#[cfg(feature = "ble")]
pub mod ble;

pub use discovery::{discover, discover_ble, discover_serial, discover_usb, DiscoveredPrinter};
pub use media::{resolve_media, MediaSource};
pub use transport::{FileTransport, NetworkTransport, Transport};

#[cfg(feature = "usb")]
pub use transport::UsbTransport;

#[cfg(feature = "serial")]
pub use transport::{SerialTransport, DEFAULT_SERIAL_BAUD};

#[cfg(feature = "ble")]
pub use transport::{BleTransport, BLE_DEFAULT_CHUNK, BLE_DEFAULT_SCAN_SECS};

pub use troubleshoot::{format_dispatch_failure, format_send_failure, TransportTarget};

/// Errors produced by the device layer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The requested device could not be found.
    #[error("device not found: {0}")]
    NotFound(String),

    /// A transport-level failure (open/claim/write).
    #[error("transport error: {0}")]
    Transport(String),
}
