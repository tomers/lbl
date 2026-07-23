//! Brother QL-series status reply (`ESC i S`) parsing.
//!
//! The printer returns a fixed 32-byte status block. See Brother's *Raster
//! Command Reference* for the QL-800 / QL-810W / QL-820NWB family.
//!
//! The wire shape exposes machine codes only. Consumers map codes and error
//! bitmasks to display copy.

use crate::StatusError;

/// Length of a Brother QL status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// `ESC i S` status information request.
pub const STATUS_REQUEST: [u8; 3] = [0x1B, b'i', b'S'];

/// Parsed fields from the 32-byte status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrotherQlStatus {
    pub status_type_code: u8,
    pub phase_type_code: u8,
    pub media_type_code: u8,
    pub media_width_mm: u8,
    pub media_length_mm: u8,
    /// Model identity derived from the firmware model byte (e.g. `QL-820NWB`).
    pub model_code: String,
    pub error_info_1: u8,
    pub error_info_2: u8,
}

const MODEL_CODES: &[(u8, &str)] = &[(0x38, "QL-800"), (0x39, "QL-810W"), (0x41, "QL-820NWB")];

/// Parse a 32-byte Brother QL status reply.
pub fn parse_status(status: &[u8]) -> Result<BrotherQlStatus, StatusError> {
    if status.len() < STATUS_REPLY_LEN {
        return Err(StatusError::Parse(format!(
            "short Brother QL status reply ({} bytes, expected {STATUS_REPLY_LEN})",
            status.len()
        )));
    }
    if status[0] != 0x80 || status[1] != 0x20 || status[2] != 0x42 {
        return Err(StatusError::Parse(format!(
            "unexpected Brother QL status header {:02x}:{:02x}:{:02x}",
            status[0], status[1], status[2]
        )));
    }

    let error1 = status[8];
    let error2 = status[9];
    let media_type_code = status[11];
    let status_type_code = status[18];
    let phase_type_code = status[19];
    let model_byte = status[4];

    Ok(BrotherQlStatus {
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
        assert_eq!(status.media_type_code, 0x4A);
        assert_eq!(status.error_info_1, 0);
        assert_eq!(status.error_info_2, 0);
        assert_eq!(media_key_hint(&status).as_deref(), Some("62"));
    }

    #[test]
    fn parses_cover_open_error() {
        let mut s = sample_ready_62mm();
        s[9] = 1 << 4; // cover open
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.error_info_2 & (1 << 4), 1 << 4);
        assert_eq!(status.status_type_code, 0x02);
    }

    #[test]
    fn rejects_bad_header() {
        let mut s = sample_ready_62mm();
        s[0] = 0x00;
        assert!(parse_status(&s).is_err());
    }
}
