//! Printer discovery and transport for `lbl`.
//!
//! - [`discovery`] enumerates connected printers — USB bulk devices via
//!   [nusb](https://docs.rs/nusb) and USB serial ports (NIIMBOT D-series and
//!   other CDC-ACM printers) via [serialport](https://docs.rs/serialport) — and
//!   maps them to known models/protocols.
//! - [`transport`] delivers encoded bytes to a printer over USB bulk transfer,
//!   a raw TCP socket (network printers, typically port 9100), or a
//!   bidirectional serial port (USB CDC-ACM, e.g. NIIMBOT D-series).
//! - [`media`] resolves the media loaded in a printer: auto-detection when the
//!   device reports it, otherwise an explicit override.
//!
//! USB support is behind the default `usb` feature so the crate still builds in
//! environments without USB access.

pub mod discovery;
pub mod known;
pub mod media;
pub mod transport;

pub use discovery::{discover, discover_serial, discover_usb, DiscoveredPrinter};
pub use media::{resolve_media, MediaSource};
pub use transport::{FileTransport, NetworkTransport, Transport};

#[cfg(feature = "usb")]
pub use transport::UsbTransport;

#[cfg(feature = "serial")]
pub use transport::{SerialTransport, DEFAULT_SERIAL_BAUD};

/// Errors produced by the device layer.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// The requested device could not be found.
    #[error("device not found: {0}")]
    NotFound(String),

    /// A transport-level failure (open/claim/write).
    #[error("transport error: {0}")]
    Transport(String),
}
