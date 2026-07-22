//! Brother QL-series USB status queries (`ESC i S`).
//!
//! Reply parsing and the request/length constants live in
//! [`lbl_status::brother_ql`]; this module only drives the USB bulk transfer.

use crate::transport::{open_usb_bulk_session, UsbBulkSession, UsbTransport};
use crate::DeviceError;

pub use lbl_status::{
    brother_ql_media_key_hint as media_key_hint, parse_brother_ql_status as parse_status,
    BrotherQlStatus, BROTHER_QL_STATUS_REPLY_LEN as STATUS_REPLY_LEN,
    BROTHER_QL_STATUS_REQUEST as STATUS_REQUEST,
};

/// Query status over an open USB bulk session.
pub fn query_print_status(session: &mut UsbBulkSession) -> Result<BrotherQlStatus, DeviceError> {
    session.transfer_out(&STATUS_REQUEST)?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    Ok(parse_status(&status)?)
}

/// Query Brother QL status over USB.
pub fn query_status(usb: &UsbTransport) -> Result<BrotherQlStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_print_status(&mut session)
}
