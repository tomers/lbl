//! Brother QL-series USB status queries (`ESC i S`).
//!
//! The printer returns a fixed 32-byte status block. See Brother's *Raster
//! Command Reference* for the QL-800 / QL-810W / QL-820NWB family.

use crate::DeviceError;

#[cfg(feature = "usb")]
use crate::transport::{open_usb_bulk_session, UsbBulkSession, UsbTransport};

/// Length of a Brother QL status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// `ESC i S` status information request.
pub const STATUS_REQUEST: [u8; 3] = [0x1B, b'i', b'S'];

/// Parsed fields from the 32-byte status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrotherQlStatus {
    pub status_type: String,
    pub status_type_code: u8,
    pub phase_type: String,
    pub phase_type_code: u8,
    pub media_type: String,
    pub media_type_code: u8,
    pub media_width_mm: u8,
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

/// Parse a 32-byte Brother QL status reply.
pub fn parse_status(status: &[u8]) -> Result<BrotherQlStatus, DeviceError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(DeviceError::Transport(format!(
            "short Brother QL status reply ({} bytes, expected {STATUS_REPLY_LEN})",
            status.len()
        )));
    }
    if status[0] != 0x80 || status[1] != 0x20 || status[2] != 0x42 {
        return Err(DeviceError::Transport(format!(
            "unexpected Brother QL status header {:02x}:{:02x}:{:02x}",
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
        (0x0A, "Continuous"),
        (0x0B, "Die-cut"),
        (0x4A, "Continuous"),
        (0x4B, "Die-cut"),
    ];
    const MODEL_CODES: &[(u8, &str)] = &[(0x38, "QL-800"), (0x39, "QL-810W"), (0x41, "QL-820NWB")];
    const ERROR1: &[(u8, &str)] = &[
        (0, "No media"),
        (1, "End of media"),
        (2, "Cutter jam"),
        (4, "Printer in use"),
        (5, "Printer turned off"),
    ];
    const ERROR2: &[(u8, &str)] = &[
        (0, "Replace media"),
        (1, "Expansion buffer full"),
        (2, "Communication error"),
        (4, "Cover open"),
        (6, "Media cannot be fed"),
        (7, "System error"),
    ];

    let error1 = status[8];
    let error2 = status[9];
    let media_type_code = status[11];
    let status_type_code = status[18];
    let phase_type_code = status[19];
    let model_byte = status[4];

    let mut errors = collect_errors(ERROR1, error1);
    errors.extend(collect_errors(ERROR2, error2));

    Ok(BrotherQlStatus {
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
pub fn query_print_status(session: &mut UsbBulkSession) -> Result<BrotherQlStatus, DeviceError> {
    session.transfer_out(&STATUS_REQUEST)?;
    let status = session.transfer_in(STATUS_REPLY_LEN)?;
    parse_status(&status)
}

/// Query Brother QL status over USB.
#[cfg(feature = "usb")]
pub fn query_status(usb: &UsbTransport) -> Result<BrotherQlStatus, DeviceError> {
    let mut session = open_usb_bulk_session(usb)?;
    query_print_status(&mut session)
}

/// Best-effort media key from status width/length (e.g. `62` or `29x90`).
pub fn media_key_hint(status: &BrotherQlStatus) -> Option<String> {
    if status.media_width_mm == 0 || status.media_type_code == 0 {
        return None;
    }
    if status.media_length_mm == 0 {
        Some(status.media_width_mm.to_string())
    } else {
        Some(format!(
            "{}x{}",
            status.media_width_mm, status.media_length_mm
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ready_62mm() -> [u8; 32] {
        let mut s = [0u8; 32];
        s[0] = 0x80;
        s[1] = 0x20;
        s[2] = b'B';
        s[3] = b'4';
        s[4] = b'A'; // QL-820NWB
        s[5] = b'0';
        s[6] = b'0';
        s[10] = 62;
        s[11] = 0x4A; // continuous
        s[14] = 0x3F;
        s[17] = 0;
        s[18] = 0x00; // reply to status request
        s[19] = 0x00; // waiting
        s
    }

    #[test]
    fn parses_ready_continuous_62mm() {
        let status = parse_status(&sample_ready_62mm()).unwrap();
        assert_eq!(status.model_code, "QL-820NWB");
        assert_eq!(status.media_width_mm, 62);
        assert_eq!(status.media_length_mm, 0);
        assert_eq!(status.media_type, "Continuous");
        assert!(status.errors.is_empty());
        assert_eq!(media_key_hint(&status).as_deref(), Some("62"));
    }

    #[test]
    fn parses_cover_open_error() {
        let mut s = sample_ready_62mm();
        s[9] = 1 << 4; // cover open
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert!(status.errors.iter().any(|e| e == "Cover open"));
        assert_eq!(status.status_type_code, 0x02);
    }

    #[test]
    fn rejects_bad_header() {
        let mut s = sample_ready_62mm();
        s[0] = 0x00;
        assert!(parse_status(&s).is_err());
    }
}
