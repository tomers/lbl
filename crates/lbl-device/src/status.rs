//! Protocol-specific print-engine status queries over USB.
//!
//! The tagged [`PrintStatus`] type and the `status_supported` /
//! `soft_reboot_supported` predicates live in [`lbl_status`]; this module adds
//! the USB transport wrappers for the protocols with a bulk-transfer query.

pub use lbl_status::{soft_reboot_supported, status_supported, PrintStatus};

#[cfg(feature = "usb")]
use lbl_core::printer::Protocol;

#[cfg(feature = "usb")]
use crate::transport::{open_usb_bulk_session, UsbTransport};
#[cfg(feature = "usb")]
use crate::DeviceError;
#[cfg(feature = "usb")]
use crate::{brother_pt, brother_ql, dymo_lw, zpl};

/// Query print-engine status for `protocol` over USB.
///
/// Only the protocols with a USB bulk-transfer status query are handled here;
/// serial/BLE-only protocols (e.g. NIIMBOT, GPGL) report `status_supported`
/// but are queried through their own transports.
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
        Protocol::Dymo => {
            let mut session = open_usb_bulk_session(usb)?;
            session.transfer_out(&lbl_status::DYMO_D1_STATUS_REQUEST)?;
            let status = session.transfer_in(lbl_status::DYMO_D1_STATUS_READ_LEN)?;
            Ok(PrintStatus::Dymo(lbl_status::parse_dymo_d1_status(
                &status,
            )?))
        }
        Protocol::DymoLwClassic => {
            let mut session = open_usb_bulk_session(usb)?;
            session.transfer_out(&lbl_status::DYMO_LW_CLASSIC_STATUS_REQUEST)?;
            let status = session.transfer_in(64)?;
            Ok(PrintStatus::DymoLwClassic(
                lbl_status::parse_dymo_lw_classic_status(&status)?,
            ))
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
            "print-engine status over USB not supported for protocol {other:?}"
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

/// Soft-reboot the print engine when the protocol supports it.
#[cfg(feature = "usb")]
pub fn soft_reboot_print_engine(protocol: Protocol, usb: &UsbTransport) -> Result<(), DeviceError> {
    match protocol {
        Protocol::DymoLw => dymo_lw::soft_reboot_usb(usb),
        other => Err(DeviceError::Transport(format!(
            "soft reboot not supported for protocol {other:?}"
        ))),
    }
}
