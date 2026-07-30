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

#[cfg(feature = "usb")]
pub mod brother_pt;
#[cfg(feature = "usb")]
pub mod brother_ql;
pub mod discovery;
#[cfg(feature = "usb")]
pub mod dymo_lw;
#[cfg(feature = "usb")]
pub mod gpgl;
pub mod media;
pub mod status;
pub mod transport;
pub mod troubleshoot;
#[cfg(feature = "usb")]
pub mod usb_printer_id;
#[cfg(feature = "usb")]
pub mod zpl;

#[cfg(feature = "ble")]
pub mod ble;

pub use discovery::{discover, discover_ble, discover_serial, discover_usb, DiscoveredDevice};
pub use media::{resolve_media, MediaSource};
pub use transport::{FileTransport, NetworkTransport, Transport};

#[cfg(feature = "usb")]
pub use brother_pt::{
    media_key_hint as brother_pt_media_key_hint, parse_status as parse_brother_pt_status,
    query_status as query_brother_pt_status, BrotherPtStatus,
    STATUS_REPLY_LEN as BROTHER_PT_STATUS_REPLY_LEN, STATUS_REQUEST as BROTHER_PT_STATUS_REQUEST,
};
#[cfg(feature = "usb")]
pub use brother_ql::{
    media_key_hint as brother_ql_media_key_hint, parse_status as parse_brother_ql_status,
    query_status as query_brother_ql_status, BrotherQlStatus,
    STATUS_REPLY_LEN as BROTHER_QL_STATUS_REPLY_LEN, STATUS_REQUEST as BROTHER_QL_STATUS_REQUEST,
};
#[cfg(feature = "usb")]
pub use dymo_lw::{
    parse_engine_version, parse_print_status, parse_sku_info, query_loaded_media, query_status,
    soft_reboot_usb, DymoLwUsbTransport, Lw550EngineVersion, Lw550MainBayStatus,
    Lw550PrintEngineStatus, Lw550PrintHeadStatus, Lw550PrintHeadVoltage, Lw550PrintStatus,
    Lw550SkuInfo, ENGINE_VERSION_REPLY_LEN, SKU_INFO_REPLY_LEN, STATUS_REPLY_LEN,
};
#[cfg(feature = "usb")]
pub use gpgl::{send_cut_job, wait_until_ready, CAMEO4_PID, CAMEO4_VID};
#[cfg(feature = "usb")]
pub use status::{
    query_loaded_media_sku, query_print_status, soft_reboot_print_engine, soft_reboot_supported,
    status_supported, PrintStatus,
};
#[cfg(feature = "usb")]
pub use transport::{open_usb_bulk_session, UsbBulkSession, UsbTransport};
#[cfg(feature = "usb")]
pub use usb_printer_id::{query_identity as query_usb_printer_identity, UsbPrinterIdentity};
#[cfg(feature = "usb")]
pub use zpl::{
    parse_host_status as parse_zpl_host_status, query_status as query_zpl_status, ZplHostStatus,
};

#[cfg(feature = "serial")]
pub use transport::{SerialTransport, DEFAULT_SERIAL_BAUD};

#[cfg(feature = "ble")]
pub use transport::{BleFrameMode, BleTransport, BLE_DEFAULT_CHUNK, BLE_DEFAULT_SCAN_SECS};

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

impl From<lbl_status::StatusError> for DeviceError {
    /// Status parsing runs behind a transport; a parse failure surfaces as a
    /// transport-level error to the caller.
    fn from(err: lbl_status::StatusError) -> Self {
        DeviceError::Transport(err.to_string())
    }
}
