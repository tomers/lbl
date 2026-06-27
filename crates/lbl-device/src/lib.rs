//! Printer discovery and transport for `lbl`.
//!
//! - [`discovery`] enumerates connected printers (USB via
//!   [nusb](https://docs.rs/nusb)) and maps them to known models/protocols.
//! - [`transport`] delivers encoded bytes to a printer over USB bulk transfer
//!   or a raw TCP socket (network printers, typically port 9100).
//! - [`media`] resolves the media loaded in a printer: auto-detection when the
//!   device reports it, otherwise an explicit override.
//!
//! USB support is behind the default `usb` feature so the crate still builds in
//! environments without USB access.

pub mod discovery;
pub mod known;
pub mod media;
pub mod transport;

pub use discovery::{discover_usb, DiscoveredPrinter};
pub use media::{resolve_media, MediaSource};
pub use transport::{FileTransport, NetworkTransport, Transport};

#[cfg(feature = "usb")]
pub use transport::UsbTransport;

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
