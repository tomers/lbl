//! USB Printer Class identity via IEEE 1284 `GET_DEVICE_ID`.
//!
//! Reply parsing lives in [`lbl_status::usb_printer_id`]; this module issues the
//! control transfer and attaches USB string descriptors.

use std::time::Duration;

use nusb::transfer::{ControlIn, ControlType, Recipient};
use nusb::MaybeFuture;

use crate::transport::UsbTransport;
use crate::DeviceError;

pub use lbl_status::{
    parse_usb_printer_device_id, UsbPrinterIdentity, GET_DEVICE_ID_LENGTH, GET_DEVICE_ID_REQUEST,
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

/// Query USB printer identity (Device ID + string descriptors) for `usb`.
pub fn query_identity(usb: &UsbTransport) -> Result<UsbPrinterIdentity, DeviceError> {
    let device_info = nusb::list_devices()
        .wait()
        .map_err(|e| DeviceError::Transport(format!("listing usb devices: {e}")))?
        .find(|d| {
            d.vendor_id() == usb.vendor_id
                && d.product_id() == usb.product_id
                && usb
                    .serial
                    .as_deref()
                    .map(|s| d.serial_number() == Some(s))
                    .unwrap_or(true)
        })
        .ok_or_else(|| {
            DeviceError::NotFound(format!("usb {:04x}:{:04x}", usb.vendor_id, usb.product_id))
        })?;

    let product = device_info.product_string().map(str::to_string);
    let usb_manufacturer = device_info.manufacturer_string().map(str::to_string);
    let serial_number = device_info.serial_number().map(str::to_string);
    let vendor_id = Some(device_info.vendor_id());
    let product_id = Some(device_info.product_id());

    let device = device_info
        .open()
        .wait()
        .map_err(|e| DeviceError::Transport(format!("opening device: {e}")))?;
    let interface = device
        .claim_interface(usb.interface)
        .wait()
        .map_err(|e| DeviceError::Transport(format!("claiming interface: {e}")))?;

    // USB Printer Class GET_DEVICE_ID: bmRequestType=0xA1, bRequest=0.
    // wValue = (config << 8) | alternate — 0 is valid for single-config devices.
    // wIndex = claimed interface number.
    let raw = interface
        .control_in(
            ControlIn {
                control_type: ControlType::Class,
                recipient: Recipient::Interface,
                request: GET_DEVICE_ID_REQUEST,
                value: 0,
                index: u16::from(usb.interface),
                length: GET_DEVICE_ID_LENGTH,
            },
            CONTROL_TIMEOUT,
        )
        .wait()
        .map_err(|e| DeviceError::Transport(format!("GET_DEVICE_ID: {e}")))?;

    let identity = parse_usb_printer_device_id(&raw)
        .unwrap_or_else(|_| UsbPrinterIdentity::empty())
        .with_usb_strings(
            product,
            usb_manufacturer,
            serial_number,
            vendor_id,
            product_id,
        );

    if identity.has_displayable_fields() {
        Ok(identity)
    } else {
        Err(DeviceError::Transport(
            "USB printer returned no Device ID or string identity".into(),
        ))
    }
}
