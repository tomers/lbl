//! Brother P-touch / TZe USB status queries (`ESC i S`).
//!
//! The printer returns a fixed 32-byte status block. See Brother's *Raster
//! Command Reference* for the PT-H500 / PT-P700 / PT-E500 family.

use crate::DeviceError;

#[cfg(feature = "usb")]
use crate::transport::{open_usb_bulk_session, UsbBulkSession, UsbTransport};

/// Length of a Brother PT status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// `ESC i S` status information request.
pub const STATUS_REQUEST: [u8; 3] = [0x1B, b'i', b'S'];

/// Parsed fields from the 32-byte status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrotherPtStatus {
    pub status_type: String,
    pub status_type_code: u8,
    pub phase_type: String,
    pub phase_type_code: u8,
    pub media_type: String,
    pub media_type_code: u8,
    pub media_width_mm: u8,
    /// Always `0` for continuous TZe tape.
    pub media_length_mm: u8,
    pub model_code: String,
    pub errors: Vec<String>,
    pub error_info_1: u8,
    pub error_info_2: u8,
}

fn lookup(map: &[(u8, &str)], code: u8, fallback: &str) -> String {
    map.iter()
        .find(|(c, _)| *c == code)
        .map(|(_, s)| (*s).to_string())
        .unwrap_or_else(|| format!("{fallback} ({code})"))
}

fn collect_errors(defs: &[(u8, &str)], bits: u8) -> Vec<String> {
    defs.iter()
        .filter(|(bit, _)| bits & (1 << bit) != 0)
        .map(|(_, label)| (*label).to_string())
        .collect()
}

/// Parse a 32-byte Brother PT status reply.
pub fn parse_status(status: &[u8]) -> Result<BrotherPtStatus, DeviceError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "short Brother PT status reply ({} bytes, expected {STATUS_REPLY_LEN})",
            status.len()
        )));
    }
    // Header: 80h, 20h, 'B'. Series code is '0' for the PT-P700 family.
    if status[0] != 0x80 || status[1] != 0x20 || status[2] != 0x42 {
        return Err(DeviceError::Transport(format!(
            "unexpected Brother PT status header {:02x}:{:02x}:{:02x}",
            status[0], status[1], status[2]
        )));
    }

    const STATUS_TYPES: &[(u8, &str)] = &[
        (0x00, "Reply to status request"),
        (0x01, "Printing completed"),
        (0x02, "Error occurred"),
        (0x04, "Turned off"),
        (0x05, "Notification"),
        (0x06, "Phase change"),
    ];
    const PHASE_TYPES: &[(u8, &str)] = &[(0x00, "Waiting to receive"), (0x01, "Printing")];
    const MEDIA_TYPES: &[(u8, &str)] = &[
        (0x00, "No media"),
        (0x01, "Laminated tape"),
        (0x03, "Non-laminated tape"),
        (0x11, "Heat-shrink tube 2:1"),
        (0x17, "Heat-shrink tube 3:1"),
        (0xFF, "Incompatible tape"),
    ];
    const MODEL_CODES: &[(u8, &str)] = &[
        (b'd', "PT-H500"),
        (b'e', "PT-E500"),
        (b'g', "PT-P700"),
        (b'q', "PT-P900"),
        (b'o', "PT-P900W"),
        (b'p', "PT-P950NW"),
        (b'x', "PT-P910BT"),
    ];
    // Raster Command Reference — Error information 1 / 2 (PT-H500 / P700 / E500).
    const ERROR1: &[(u8, &str)] = &[
        (0, "No media"),
        (2, "Cutter jam"),
        (3, "Weak batteries"),
        (6, "High-voltage adapter"),
    ];
    const ERROR2: &[(u8, &str)] = &[(0, "Replace media"), (4, "Cover open"), (5, "Overheating")];

    let error1 = status[8];
    let error2 = status[9];
    let media_type_code = status[11];
    let status_type_code = status[18];
    let phase_type_code = status[19];
    let model_byte = status[4];

    let mut errors = collect_errors(ERROR1, error1);
    errors.extend(collect_errors(ERROR2, error2));

    Ok(BrotherPtStatus {
        status_type: lookup(STATUS_TYPES, status_type_code, "unknown status"),
        status_type_code,
        phase_type: lookup(PHASE_TYPES, phase_type_code, "unknown phase"),
        phase_type_code,
        media_type: lookup(MEDIA_TYPES, media_type_code, "unknown media"),
        media_type_code,
        media_width_mm: status[10],
        media_length_mm: status[17],
        model_code: MODEL_CODES
            .iter()
            .find(|(c, _)| *c == model_byte)
            .map(|(_, s)| (*s).to_string())
            .unwrap_or_else(|| {
                if model_byte.is_ascii_graphic() {
                    (model_byte as char).to_string()
                } else {
                    format!("0x{model_byte:02X}")
                }
            }),
        errors,
        error_info_1: error1,
        error_info_2: error2,
    })
}

/// Query status over an open USB bulk session.
#[cfg(feature = "usb")]
pub fn query_print_status(session: &mut UsbBulkSession) -> Result<BrotherPtStatus, DeviceError> {
    session.transfer_out(&STATUS_REQUEST)?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    parse_status(&status)
}

/// Query Brother PT status over USB.
#[cfg(feature = "usb")]
pub fn query_status(usb: &UsbTransport) -> Result<BrotherPtStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_print_status(&mut session)
}

/// Best-effort media width key from status (e.g. `12` or `4` for 3.5 mm TZe).
pub fn media_key_hint(status: &BrotherPtStatus) -> Option<String> {
    if status.media_width_mm == 0 || status.media_type_code == 0 {
        return None;
    }
    Some(status.media_width_mm.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ready_12mm() -> [u8; 32] {
        let mut s = [0u8; 32];
        s[0] = 0x80;
        s[1] = 0x20;
        s[2] = b'B';
        s[3] = b'0';
        s[4] = b'g'; // PT-P700
        s[5] = b'0';
        s[10] = 12;
        s[11] = 0x01; // laminated
        s[17] = 0;
        s[18] = 0x00;
        s[19] = 0x00;
        s
    }

    #[test]
    fn parses_ready_laminated_12mm() {
        let status = parse_status(&sample_ready_12mm()).unwrap();
        assert_eq!(status.model_code, "PT-P700");
        assert_eq!(status.media_width_mm, 12);
        assert_eq!(status.media_type, "Laminated tape");
        assert!(status.errors.is_empty());
        assert_eq!(media_key_hint(&status).as_deref(), Some("12"));
    }

    #[test]
    fn parses_cover_open_error() {
        let mut s = sample_ready_12mm();
        s[9] = 1 << 4;
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert!(status.errors.iter().any(|e| e == "Cover open"));
        assert_eq!(status.status_type_code, 0x02);
    }

    #[test]
    fn rejects_bad_header() {
        let mut s = sample_ready_12mm();
        s[0] = 0x00;
        assert!(parse_status(&s).is_err());
    }
}
