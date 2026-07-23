//! Brother QL-series status reply (`ESC i S`) parsing.
//!
//! The printer returns a fixed 32-byte status block. See Brother's *Raster
//! Command Reference* for the QL-800 / QL-810W / QL-820NWB family.
//!
//! The wire shape exposes machine-stable tokens only. Consumers map tokens to
//! display copy.

use crate::brother::{
    collect_error_bits, summary_from_parts, BrotherPhaseType, BrotherStatusSummary,
    BrotherStatusType,
};
use crate::StatusError;

/// Length of a Brother QL status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// `ESC i S` status information request.
pub const STATUS_REQUEST: [u8; 3] = [0x1B, b'i', b'S'];

/// QL media-type byte (offset 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherQlMediaType {
    NoMedia,
    Continuous,
    DieCut,
    Unknown,
}

impl BrotherQlMediaType {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0x00 => Self::NoMedia,
            0x0a | 0x4a => Self::Continuous,
            0x0b | 0x4b => Self::DieCut,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoMedia => "no_media",
            Self::Continuous => "continuous",
            Self::DieCut => "die_cut",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_present(self) -> bool {
        !matches!(self, Self::NoMedia)
    }
}

/// Decoded QL error-info bitmask flags (bytes 8–9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherQlError {
    NoMedia,
    EndOfMedia,
    CutterJam,
    PrinterInUse,
    PrinterTurnedOff,
    ReplaceMedia,
    ExpansionBufferFull,
    CommunicationError,
    CoverOpen,
    MediaCannotBeFed,
    SystemError,
}

impl BrotherQlError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoMedia => "no_media",
            Self::EndOfMedia => "end_of_media",
            Self::CutterJam => "cutter_jam",
            Self::PrinterInUse => "printer_in_use",
            Self::PrinterTurnedOff => "printer_turned_off",
            Self::ReplaceMedia => "replace_media",
            Self::ExpansionBufferFull => "expansion_buffer_full",
            Self::CommunicationError => "communication_error",
            Self::CoverOpen => "cover_open",
            Self::MediaCannotBeFed => "media_cannot_be_fed",
            Self::SystemError => "system_error",
        }
    }
}

impl AsRef<str> for BrotherQlError {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

const ERROR1: &[(u8, BrotherQlError)] = &[
    (0, BrotherQlError::NoMedia),
    (1, BrotherQlError::EndOfMedia),
    (2, BrotherQlError::CutterJam),
    (4, BrotherQlError::PrinterInUse),
    (5, BrotherQlError::PrinterTurnedOff),
];

const ERROR2: &[(u8, BrotherQlError)] = &[
    (0, BrotherQlError::ReplaceMedia),
    (1, BrotherQlError::ExpansionBufferFull),
    (2, BrotherQlError::CommunicationError),
    (4, BrotherQlError::CoverOpen),
    (6, BrotherQlError::MediaCannotBeFed),
    (7, BrotherQlError::SystemError),
];

/// Parsed fields from the 32-byte status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrotherQlStatus {
    pub status_type: BrotherStatusType,
    pub phase_type: BrotherPhaseType,
    pub media_type: BrotherQlMediaType,
    pub media_width_mm: u8,
    pub media_length_mm: u8,
    /// Model identity derived from the firmware model byte (e.g. `QL-820NWB`).
    pub model_code: String,
    /// Bit 7 of reserved byte 25: two-colour roll loaded (DK-22251 class).
    pub two_color_roll: bool,
    /// Decoded error flags from error-info bitmasks.
    pub errors: Vec<BrotherQlError>,
    /// Derived readiness summary (state token + severity).
    pub summary: BrotherStatusSummary,
}

const MODEL_CODES: &[(u8, &str)] = &[
    (0x31, "QL-500"),
    (0x32, "QL-550"),
    (0x33, "QL-560"),
    (0x34, "QL-570"),
    (0x35, "QL-580N"),
    (0x36, "QL-650TD"),
    (0x37, "QL-700"),
    (0x38, "QL-800"),
    (0x39, "QL-810W"),
    (0x41, "QL-820NWB"),
    (0x43, "QL-1100"),
    (0x44, "QL-1110NWB"),
    (0x47, "QL-1115NWB"),
];

fn decode_errors(error1: u8, error2: u8) -> Vec<BrotherQlError> {
    let mut out = collect_error_bits(ERROR1, error1);
    out.extend(collect_error_bits(ERROR2, error2));
    out
}

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

    let media_type = BrotherQlMediaType::from_byte(status[11]);
    let status_type = BrotherStatusType::from_byte(status[18]);
    let phase_type = BrotherPhaseType::from_byte(status[19]);
    let model_byte = status[4];
    let errors = decode_errors(status[8], status[9]);
    let media_width_mm = status[10];
    let media_present = media_width_mm > 0 && media_type.is_present();
    let summary = summary_from_parts(&errors, status_type, phase_type, media_present);

    Ok(BrotherQlStatus {
        status_type,
        phase_type,
        media_type,
        media_width_mm,
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
        two_color_roll: status[25] & 0x80 != 0,
        errors,
        summary,
    })
}

/// Best-effort media key from status width/length (e.g. `62` or `29x90`).
pub fn media_key_hint(status: &BrotherQlStatus) -> Option<String> {
    if status.media_width_mm == 0 || !status.media_type.is_present() {
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

/// Readiness summary so UI hosts do not re-encode status bytes.
pub fn status_summary(status: &BrotherQlStatus) -> BrotherStatusSummary {
    let media_present = status.media_width_mm > 0 && status.media_type.is_present();
    summary_from_parts(
        &status.errors,
        status.status_type,
        status.phase_type,
        media_present,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BrotherSeverity;

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
        assert_eq!(status.media_type, BrotherQlMediaType::Continuous);
        assert!(status.errors.is_empty());
        assert_eq!(media_key_hint(&status).as_deref(), Some("62"));
        let summary = status_summary(&status);
        assert_eq!(summary.state, "ready");
        assert_eq!(summary.severity, BrotherSeverity::Success);
    }

    #[test]
    fn parses_cover_open_error() {
        let mut s = sample_ready_62mm();
        s[9] = 1 << 4; // cover open
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.errors, vec![BrotherQlError::CoverOpen]);
        assert_eq!(status.status_type, BrotherStatusType::Error);
        let summary = status_summary(&status);
        assert_eq!(summary.state, "cover_open");
        assert_eq!(summary.severity, BrotherSeverity::Error);
    }

    #[test]
    fn rejects_bad_header() {
        let mut s = sample_ready_62mm();
        s[0] = 0x00;
        assert!(parse_status(&s).is_err());
    }
}
