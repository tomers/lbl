//! USB Printer Class IEEE 1284 Device ID (`GET_DEVICE_ID`) identity.
//!
//! Identity for printers that expose it only through the USB Printer Class
//! control request (no usable bulk dialect status). The Device ID string is
//! enough for a device-information panel; it never carries readiness.
//!
//! This is a USB-class path, not a print-language dialect. Profile protocols
//! that use it are selected by [`crate::status_uses_usb_device_id`].
//!
//! ## Wire format
//!
//! ```text
//! [u16 BE length][ASCII key:value; key:value; …]
//! ```
//!
//! Common keys: `MFG` / `MANUFACTURER`, `MDL` / `MODEL`, `DES` / `DESCRIPTION`,
//! `CMD` / `COMMAND SET`.

use crate::StatusError;

/// USB Printer Class `GET_DEVICE_ID` request (`bRequest = 0`).
pub const GET_DEVICE_ID_REQUEST: u8 = 0;

/// Suggested control-IN length for Device ID (includes the 2-byte length prefix).
pub const GET_DEVICE_ID_LENGTH: u16 = 1024;

/// Parsed USB printer identity (IEEE 1284 Device ID + optional USB strings).
///
/// Carries no readiness — Device ID cannot assert Ready. Always expose values
/// after [`UsbPrinterIdentity::normalize`] (or [`Self::with_usb_strings`]): OEM
/// placeholders such as `MFG:Printer` and all-zero serials are stripped at this
/// boundary so hosts do not re-implement that filter.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UsbPrinterIdentity {
    /// IEEE 1284 `MFG` / `MANUFACTURER` after normalize.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// IEEE 1284 `MDL` / `MODEL`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// IEEE 1284 `DES` / `DESCRIPTION`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// IEEE 1284 `CMD` / `COMMAND SET`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_set: Option<String>,
    /// USB product string descriptor (when available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// USB manufacturer string descriptor (when available).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usb_manufacturer: Option<String>,
    /// USB serial string after [`meaningful_serial`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// USB vendor id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<u16>,
    /// USB product id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_id: Option<u16>,
    /// Raw IEEE 1284 key/value body (without the length prefix).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_device_id: Option<String>,
}

impl UsbPrinterIdentity {
    /// Empty identity (all fields unset).
    pub fn empty() -> Self {
        Self {
            manufacturer: None,
            model: None,
            description: None,
            command_set: None,
            product: None,
            usb_manufacturer: None,
            serial_number: None,
            vendor_id: None,
            product_id: None,
            raw_device_id: None,
        }
    }

    /// Strip OEM placeholder and empty identity fields.
    ///
    /// - IEEE `MFG:Printer` (generic USB-class filler) → cleared
    /// - All-zero USB serial strings → cleared
    /// - Empty / whitespace-only strings → cleared
    pub fn normalize(mut self) -> Self {
        self.manufacturer =
            nonempty(self.manufacturer).filter(|m| !m.eq_ignore_ascii_case("printer"));
        self.model = nonempty(self.model);
        self.description = nonempty(self.description);
        self.command_set = nonempty(self.command_set);
        self.product = nonempty(self.product);
        self.usb_manufacturer = nonempty(self.usb_manufacturer);
        self.serial_number = meaningful_serial(self.serial_number);
        self.raw_device_id = nonempty(self.raw_device_id);
        self
    }

    /// Attach USB string / id metadata, then [`normalize`].
    pub fn with_usb_strings(
        mut self,
        product: Option<String>,
        usb_manufacturer: Option<String>,
        serial_number: Option<String>,
        vendor_id: Option<u16>,
        product_id: Option<u16>,
    ) -> Self {
        self.product = product;
        self.usb_manufacturer = usb_manufacturer;
        self.serial_number = serial_number;
        self.vendor_id = vendor_id;
        self.product_id = product_id;
        self.normalize()
    }

    /// True when at least one displayable identity field is set.
    pub fn has_displayable_fields(&self) -> bool {
        self.model.is_some()
            || self.product.is_some()
            || self.manufacturer.is_some()
            || self.usb_manufacturer.is_some()
            || self.description.is_some()
            || self.command_set.is_some()
            || self.serial_number.is_some()
            || (self.vendor_id.is_some() && self.product_id.is_some())
    }
}

/// Keep a USB serial only when it is non-empty and not an all-zero filler.
pub fn meaningful_serial(serial: Option<String>) -> Option<String> {
    let s = nonempty(serial)?;
    if s.chars().all(|c| c == '0') {
        return None;
    }
    Some(s)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

/// Parse a USB Printer Class `GET_DEVICE_ID` reply into [`UsbPrinterIdentity`].
///
/// Result is [`UsbPrinterIdentity::normalize`]d.
pub fn parse_device_id(buf: &[u8]) -> Result<UsbPrinterIdentity, StatusError> {
    if buf.len() < 2 {
        return Err(StatusError::Parse(
            "USB Device ID reply shorter than 2-byte length prefix".into(),
        ));
    }
    let declared = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    // Some stacks return only the body; tolerate missing prefix when the buffer
    // looks like `KEY:…` ASCII.
    let body = if declared > 2 && declared <= buf.len() {
        &buf[2..declared.min(buf.len())]
    } else if buf[2..].contains(&b':') {
        &buf[2..]
    } else if buf.contains(&b':') {
        buf
    } else {
        return Err(StatusError::Parse(
            "USB Device ID reply has no IEEE 1284 key:value body".into(),
        ));
    };

    let text = String::from_utf8_lossy(body);
    let raw = text.trim().to_string();
    if raw.is_empty() {
        return Err(StatusError::Parse("USB Device ID body is empty".into()));
    }

    let mut identity = UsbPrinterIdentity::empty();
    identity.raw_device_id = Some(raw.clone());

    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((key, value)) = part.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.to_ascii_uppercase().as_str() {
            "MFG" | "MANUFACTURER" => identity.manufacturer = Some(value.to_string()),
            "MDL" | "MODEL" => identity.model = Some(value.to_string()),
            "DES" | "DESCRIPTION" => identity.description = Some(value.to_string()),
            "CMD" | "COMMAND SET" | "COMMANDSET" => identity.command_set = Some(value.to_string()),
            _ => {}
        }
    }

    if identity.manufacturer.is_none()
        && identity.model.is_none()
        && identity.description.is_none()
        && identity.command_set.is_none()
    {
        return Err(StatusError::Parse(
            "USB Device ID body has no recognized MFG/MODEL/DES/CMD keys".into(),
        ));
    }

    Ok(identity.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn munbyn_fixture() -> Vec<u8> {
        let body = b"MFG:Printer ;CMD: ;MODEL:ITPP130;DES:LabelPrinter;";
        let mut buf = Vec::with_capacity(2 + body.len());
        let len = (2 + body.len()) as u16;
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(body);
        buf
    }

    #[test]
    fn parses_and_strips_placeholder_mfg() {
        let id = parse_device_id(&munbyn_fixture()).unwrap();
        assert!(id.manufacturer.is_none());
        assert_eq!(id.model.as_deref(), Some("ITPP130"));
        assert_eq!(id.description.as_deref(), Some("LabelPrinter"));
        assert!(id.command_set.is_none());
        assert!(id
            .raw_device_id
            .as_deref()
            .unwrap()
            .contains("MODEL:ITPP130"));
    }

    #[test]
    fn rejects_empty_and_short() {
        assert!(parse_device_id(&[]).is_err());
        assert!(parse_device_id(&[0x00]).is_err());
        assert!(parse_device_id(&[0x00, 0x02]).is_err());
        assert!(parse_device_id(b"\x00\x05nope").is_err());
    }

    #[test]
    fn accepts_alternate_key_names() {
        let body = b"MANUFACTURER:Acme;MDL:X1;DESCRIPTION:Label;COMMAND SET:TSPL;";
        let len = (2 + body.len()) as u16;
        let mut buf = Vec::with_capacity(2 + body.len());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(body);
        let id = parse_device_id(&buf).unwrap();
        assert_eq!(id.manufacturer.as_deref(), Some("Acme"));
        assert_eq!(id.model.as_deref(), Some("X1"));
        assert_eq!(id.description.as_deref(), Some("Label"));
        assert_eq!(id.command_set.as_deref(), Some("TSPL"));
    }

    #[test]
    fn drops_placeholder_serial() {
        assert!(meaningful_serial(Some("0000000".into())).is_none());
        assert!(meaningful_serial(Some("0000".into())).is_none());
        assert!(meaningful_serial(Some("".into())).is_none());
        assert_eq!(
            meaningful_serial(Some("130B2604027284".into())).as_deref(),
            Some("130B2604027284")
        );
    }

    #[test]
    fn with_usb_strings_normalizes() {
        let id = parse_device_id(&munbyn_fixture())
            .unwrap()
            .with_usb_strings(
                Some("ITPP130B".into()),
                Some("YXWL".into()),
                Some("0000000".into()),
                Some(0x5958),
                Some(0x0130),
            );
        assert_eq!(id.product.as_deref(), Some("ITPP130B"));
        assert_eq!(id.usb_manufacturer.as_deref(), Some("YXWL"));
        assert!(id.serial_number.is_none());
        assert!(id.manufacturer.is_none());
        assert_eq!(id.vendor_id, Some(0x5958));
        assert_eq!(id.product_id, Some(0x0130));
        assert_eq!(id.model.as_deref(), Some("ITPP130"));
        assert!(id.has_displayable_fields());
    }

    #[test]
    fn status_uses_usb_device_id_for_tspl_only() {
        use lbl_core::printer::Protocol;
        assert!(crate::status_uses_usb_device_id(Protocol::Tspl));
        assert!(!crate::status_uses_usb_device_id(Protocol::Zpl));
        assert!(!crate::status_uses_usb_device_id(Protocol::DymoLw));
    }
}
