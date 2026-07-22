//! Brother P-touch / TZe USB status queries (`ESC i S`).
//!
//! Reply parsing and the request/length constants live in
//! [`lbl_status::brother_pt`]; this module only drives the USB bulk transfer.

use crate::transport::{open_usb_bulk_session, UsbBulkSession, UsbTransport};
use crate::DeviceError;

pub use lbl_status::{
    brother_pt_media_key_hint as media_key_hint, parse_brother_pt_status as parse_status,
    BrotherPtStatus, BROTHER_PT_STATUS_REPLY_LEN as STATUS_REPLY_LEN,
    BROTHER_PT_STATUS_REQUEST as STATUS_REQUEST,
};

/// Query status over an open USB bulk session.
pub fn query_print_status(session: &mut UsbBulkSession) -> Result<BrotherPtStatus, DeviceError> {
    session.transfer_out(&STATUS_REQUEST)?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    Ok(parse_status(&status)?)
}

/// Query Brother PT status over USB.
pub fn query_status(usb: &UsbTransport) -> Result<BrotherPtStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_print_status(&mut session)
}
