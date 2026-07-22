//! Zebra ZPL `~HS` (host status) USB query.
//!
//! Reply parsing and the `~HS` command bytes live in [`lbl_status::zpl`]; this
//! module only drives the USB bulk transfer.

use crate::transport::{open_usb_bulk_session, UsbTransport};
use crate::DeviceError;

pub use lbl_status::{parse_zpl_host_status as parse_host_status, ZplHostStatus, HOST_STATUS_CMD};

/// Query Zebra `~HS` host status over USB.
pub fn query_status(usb: &UsbTransport) -> Result<ZplHostStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    session.transfer_out(HOST_STATUS_CMD)?;
    let buf = session.transfer_in(256)?;
    Ok(parse_host_status(&buf)?)
}
