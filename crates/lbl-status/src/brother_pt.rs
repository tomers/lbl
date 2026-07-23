//! Brother P-touch / TZe status reply (`ESC i S`) parsing.
//!
//! The printer returns a fixed 32-byte status block. See Brother's *Raster
//! Command Reference* for the PT-H500 / PT-P700 / PT-E500 family.
//!
//! The wire shape exposes machine codes only. Consumers map codes and error
//! bitmasks to display copy.

use crate::StatusError;

/// Length of a Brother PT status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// `ESC i S` status information request.
pub const STATUS_REQUEST: [u8; 3] = [0x1B, b'i', b'S'];

/// Parsed fields from the 32-byte status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrotherPtStatus {
    pub status_type_code: u8,
    pub phase_type_code: u8,
    pub media_type_code: u8,
    pub media_width_mm: u8,
    /// Always `0` for continuous TZe tape.
    pub media_length_mm: u8,
    /// Model identity derived from the firmware model byte (e.g. `PT-P700`).
    pub model_code: String,
    pub error_info_1: u8,
    pub error_info_2: u8,
}

const MODEL_CODES: &[(u8, &str)] = &[
    (b'd', "PT-H500"),
    (b'e', "PT-E500"),
    (b'g', "PT-P700"),
    (b'q', "PT-P900"),
    (b'o', "PT-P900W"),
    (b'p', "PT-P950NW"),
    (b'x', "PT-P910BT"),
];

/// Parse a 32-byte Brother PT status reply.
pub fn parse_status(status: &[u8]) -> Result<BrotherPtStatus, StatusError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(StatusError::Parse(format!(
            "short Brother PT status reply ({} bytes, expected {STATUS_REPLY_LEN})",
            status.len()
        )));
    }
    // Header: 80h, 20h, 'B'. Series code is '0' for the PT-P700 family.
    if status[0] != 0x80 || status[1] != 0x20 || status[2] != 0x42 {
        return Err(StatusError::Parse(format!(
            "unexpected Brother PT status header {:02x}:{:02x}:{:02x}",
            status[0], status[1], status[2]
        )));
    }

    let error1 = status[8];
    let error2 = status[9];
    let media_type_code = status[11];
    let status_type_code = status[18];
    let phase_type_code = status[19];
    let model_byte = status[4];

    Ok(BrotherPtStatus {
        status_type_code,
        phase_type_code,
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
        error_info_1: error1,
        error_info_2: error2,
    })
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
        assert_eq!(status.media_type_code, 0x01);
        assert_eq!(status.error_info_1, 0);
        assert_eq!(status.error_info_2, 0);
        assert_eq!(media_key_hint(&status).as_deref(), Some("12"));
    }

    #[test]
    fn parses_cover_open_error() {
        let mut s = sample_ready_12mm();
        s[9] = 1 << 4;
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.error_info_2 & (1 << 4), 1 << 4);
        assert_eq!(status.status_type_code, 0x02);
    }

    #[test]
    fn rejects_bad_header() {
        let mut s = sample_ready_12mm();
        s[0] = 0x00;
        assert!(parse_status(&s).is_err());
    }
}
