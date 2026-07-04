//! Protocol-specific print-engine status queries.

use lbl_core::printer::Protocol;

use crate::DeviceError;

#[cfg(feature = "usb")]
use crate::dymo_lw::{self, Lw550PrintStatusView};
#[cfg(feature = "usb")]
use crate::transport::UsbTransport;

/// Whether `protocol` supports print-engine status queries.
pub fn status_supported(protocol: Protocol) -> bool {
    matches!(protocol, Protocol::DymoLw)
}

/// Print-engine status from a connected printer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "protocol", rename_all = "kebab-case")]
pub enum PrintStatus {
    /// DYMO LabelWriter 550-series (`dymo-lw`).
    #[serde(rename = "dymo-lw")]
    DymoLw(Lw550PrintStatusView),
}

/// Query print-engine status for `protocol` over USB.
#[cfg(feature = "usb")]
pub fn query_print_status(
    protocol: Protocol,
    usb: &UsbTransport,
) -> Result<PrintStatus, DeviceError> {
    match protocol {
        Protocol::DymoLw => {
            let status = dymo_lw::query_status(usb)?;
            Ok(PrintStatus::DymoLw(status.to_view()))
        }
        other => Err(DeviceError::Transport(format!(
            "print-engine status not supported for protocol {other:?}"
        ))),
    }
}

/// Query the loaded media SKU when the protocol supports auto-detection over USB.
#[cfg(feature = "usb")]
pub fn query_loaded_media_sku(
    protocol: Protocol,
    usb: &UsbTransport,
) -> Result<Option<String>, DeviceError> {
    match protocol {
        Protocol::DymoLw => dymo_lw::query_loaded_media(usb),
        _ => Ok(None),
    }
}
