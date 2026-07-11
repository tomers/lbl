//! Protocol-specific print-engine status queries.

use lbl_core::printer::Protocol;

use crate::DeviceError;

#[cfg(feature = "usb")]
use crate::brother_pt::{self, BrotherPtStatus};
#[cfg(feature = "usb")]
use crate::brother_ql::{self, BrotherQlStatus};
#[cfg(feature = "usb")]
use crate::dymo_lw::{self, Lw550PrintStatusView};
#[cfg(feature = "usb")]
use crate::transport::UsbTransport;
#[cfg(feature = "usb")]
use crate::zpl::{self, ZplHostStatus};

/// Whether `protocol` supports print-engine status queries.
pub fn status_supported(protocol: Protocol) -> bool {
    matches!(
        protocol,
        Protocol::DymoLw | Protocol::BrotherQl | Protocol::BrotherPt | Protocol::Zpl
    )
}

/// Print-engine status from a connected printer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "protocol", rename_all = "kebab-case")]
pub enum PrintStatus {
    /// DYMO LabelWriter 550-series (`dymo-lw`).
    #[serde(rename = "dymo-lw")]
    DymoLw(Lw550PrintStatusView),
    /// Brother QL-series (`brother-ql`).
    #[serde(rename = "brother-ql")]
    BrotherQl(BrotherQlStatus),
    /// Brother P-touch / TZe (`brother-pt`).
    #[serde(rename = "brother-pt")]
    BrotherPt(BrotherPtStatus),
    /// Zebra ZPL (`zpl`).
    #[serde(rename = "zpl")]
    Zpl(ZplHostStatus),
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
        Protocol::BrotherQl => {
            let status = brother_ql::query_status(usb)?;
            Ok(PrintStatus::BrotherQl(status))
        }
        Protocol::BrotherPt => {
            let status = brother_pt::query_status(usb)?;
            Ok(PrintStatus::BrotherPt(status))
        }
        Protocol::Zpl => {
            let status = zpl::query_status(usb)?;
            Ok(PrintStatus::Zpl(status))
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
        Protocol::BrotherQl => {
            let status = brother_ql::query_status(usb)?;
            Ok(brother_ql::media_key_hint(&status))
        }
        Protocol::BrotherPt => {
            let status = brother_pt::query_status(usb)?;
            Ok(brother_pt::media_key_hint(&status))
        }
        _ => Ok(None),
    }
}
