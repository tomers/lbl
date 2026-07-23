//! Brother P-touch / TZe status reply (`ESC i S`) parsing.
//!
//! The printer returns a fixed 32-byte status block. See Brother's *Raster
//! Command Reference* for the PT-H500 / PT-P700 / PT-E500 / P900 family.
//!
//! The wire shape exposes machine-stable tokens only. Consumers map tokens to
//! display copy.

use crate::brother::{
    collect_error_bits, summary_from_parts, BrotherPhaseType, BrotherStatusSummary,
    BrotherStatusType,
};
use crate::StatusError;

/// Length of a Brother PT status reply on bulk IN.
pub const STATUS_REPLY_LEN: usize = 32;

/// `ESC i S` status information request.
pub const STATUS_REQUEST: [u8; 3] = [0x1B, b'i', b'S'];

/// PT media-type byte (offset 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherPtMediaType {
    NoMedia,
    LaminatedTape,
    NonLaminatedTape,
    FabricTape,
    HeatShrinkTube2To1,
    FleTape,
    FlexibleIdTape,
    SatinTape,
    HeatShrinkTube3To1,
    IncompatibleTape,
    Unknown,
}

impl BrotherPtMediaType {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0x00 => Self::NoMedia,
            0x01 => Self::LaminatedTape,
            0x03 => Self::NonLaminatedTape,
            0x04 => Self::FabricTape,
            0x11 => Self::HeatShrinkTube2To1,
            0x13 => Self::FleTape,
            0x14 => Self::FlexibleIdTape,
            0x15 => Self::SatinTape,
            0x17 => Self::HeatShrinkTube3To1,
            0xff => Self::IncompatibleTape,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoMedia => "no_media",
            Self::LaminatedTape => "laminated_tape",
            Self::NonLaminatedTape => "non_laminated_tape",
            Self::FabricTape => "fabric_tape",
            Self::HeatShrinkTube2To1 => "heat_shrink_tube_2_to_1",
            Self::FleTape => "fle_tape",
            Self::FlexibleIdTape => "flexible_id_tape",
            Self::SatinTape => "satin_tape",
            Self::HeatShrinkTube3To1 => "heat_shrink_tube_3_to_1",
            Self::IncompatibleTape => "incompatible_tape",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_present(self) -> bool {
        !matches!(self, Self::NoMedia | Self::IncompatibleTape)
    }
}

/// Decoded PT fault tokens from error-info bitmasks, extended-error codes,
/// notification number, and media/phase cues.
///
/// Bit meanings follow the PT-P900 / P900W / P950NW / P910BT Raster Command
/// Reference (cross-checked via thermal-label PT protocol docs). Extended-error
/// codes and notification numbers are documented for the 560-pin family; the
/// 128-pin PT-P710BT family shares the same notification table and may report
/// `status_type = error` with empty bitmasks when the fault is elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherPtError {
    NoMedia,
    EndOfMedia,
    CutterJam,
    WeakBatteries,
    HighVoltageAdapter,
    ReplaceMedia,
    ExpansionBufferFull,
    CommunicationError,
    CommunicationBufferFull,
    CoverOpen,
    Overheating,
    BlackMarkingNotDetected,
    SystemError,
    /// Extended error `0x10` — Fle tape end.
    FleTapeEnd,
    /// Extended error `0x1D` — high-resolution / draft mode rejected.
    HighResDraftError,
    /// Extended error `0x1E` — AC adapter insert/remove fault.
    AdapterInsertError,
    /// Extended error `0x21`, or media-type `0xFF`.
    IncompatibleMedia,
}

impl BrotherPtError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoMedia => "no_media",
            Self::EndOfMedia => "end_of_media",
            Self::CutterJam => "cutter_jam",
            Self::WeakBatteries => "weak_batteries",
            Self::HighVoltageAdapter => "high_voltage_adapter",
            Self::ReplaceMedia => "replace_media",
            Self::ExpansionBufferFull => "expansion_buffer_full",
            Self::CommunicationError => "communication_error",
            Self::CommunicationBufferFull => "communication_buffer_full",
            Self::CoverOpen => "cover_open",
            Self::Overheating => "overheating",
            Self::BlackMarkingNotDetected => "black_marking_not_detected",
            Self::SystemError => "system_error",
            Self::FleTapeEnd => "fle_tape_end",
            Self::HighResDraftError => "high_res_draft_error",
            Self::AdapterInsertError => "adapter_insert_error",
            Self::IncompatibleMedia => "incompatible_media",
        }
    }
}

impl AsRef<str> for BrotherPtError {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

const ERROR1: &[(u8, BrotherPtError)] = &[
    (0, BrotherPtError::NoMedia),
    (1, BrotherPtError::EndOfMedia),
    (2, BrotherPtError::CutterJam),
    (3, BrotherPtError::WeakBatteries),
    (6, BrotherPtError::HighVoltageAdapter),
];

const ERROR2: &[(u8, BrotherPtError)] = &[
    (0, BrotherPtError::ReplaceMedia),
    (1, BrotherPtError::ExpansionBufferFull),
    (2, BrotherPtError::CommunicationError),
    (3, BrotherPtError::CommunicationBufferFull),
    (4, BrotherPtError::CoverOpen),
    (5, BrotherPtError::Overheating),
    (6, BrotherPtError::BlackMarkingNotDetected),
    (7, BrotherPtError::SystemError),
];

/// Parsed fields from the 32-byte status reply.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BrotherPtStatus {
    pub status_type: BrotherStatusType,
    pub phase_type: BrotherPhaseType,
    pub media_type: BrotherPtMediaType,
    pub media_width_mm: u8,
    /// Always `0` for continuous TZe tape.
    pub media_length_mm: u8,
    /// Model identity derived from the firmware model byte (e.g. `PT-P700`).
    pub model_code: String,
    /// Battery / power byte (offset 6); meaning is model-family specific.
    pub battery_level: u8,
    /// Extended error code (offset 7), or `0` when none.
    pub extended_error: u8,
    /// Notification number (offset 22); cover open/closed on many PT models.
    pub notification_number: u8,
    /// Phase number (offsets 20–21, big-endian).
    pub phase_number: u16,
    /// Cassette tape colour ID (offset 24).
    pub tape_color_id: u8,
    /// Cassette ink / text colour ID (offset 25).
    pub text_color_id: u8,
    /// Decoded fault tokens (bitmasks + extended/notification/media cues).
    pub errors: Vec<BrotherPtError>,
    /// Derived readiness summary (state token + severity).
    pub summary: BrotherStatusSummary,
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

fn decode_error_bits(error1: u8, error2: u8) -> Vec<BrotherPtError> {
    let mut out = collect_error_bits(ERROR1, error1);
    out.extend(collect_error_bits(ERROR2, error2));
    out
}

fn decode_extended_error(code: u8) -> Option<BrotherPtError> {
    match code {
        0x00 => None,
        0x10 => Some(BrotherPtError::FleTapeEnd),
        0x1d => Some(BrotherPtError::HighResDraftError),
        0x1e => Some(BrotherPtError::AdapterInsertError),
        0x21 => Some(BrotherPtError::IncompatibleMedia),
        _ => None,
    }
}

/// Cover-open while receiving (editing phase number 20) per PT raster manuals.
const PHASE_COVER_OPEN_WHILE_RECEIVING: u16 = 20;

fn push_unique(out: &mut Vec<BrotherPtError>, err: BrotherPtError) {
    if !out.contains(&err) {
        out.push(err);
    }
}

struct ErrorCues {
    error1: u8,
    error2: u8,
    extended_error: u8,
    notification_number: u8,
    phase_number: u16,
    media_type: BrotherPtMediaType,
    media_width_mm: u8,
    status_type: BrotherStatusType,
}

/// Fold bitmask, extended-error, notification, phase, and media cues into one
/// fault list so hosts never see a bare `status_type = error` with no tokens.
fn collect_errors(cues: ErrorCues) -> Vec<BrotherPtError> {
    let mut out = decode_error_bits(cues.error1, cues.error2);
    if let Some(err) = decode_extended_error(cues.extended_error) {
        push_unique(&mut out, err);
    }
    // Notification 01h = cover open (PT-E550W / P750W / P710BT / P900 family).
    if cues.notification_number == 0x01 {
        push_unique(&mut out, BrotherPtError::CoverOpen);
    }
    if cues.phase_number == PHASE_COVER_OPEN_WHILE_RECEIVING {
        push_unique(&mut out, BrotherPtError::CoverOpen);
    }
    if cues.media_type == BrotherPtMediaType::IncompatibleTape {
        push_unique(&mut out, BrotherPtError::IncompatibleMedia);
    }
    // Some chassis raise status_type=error with empty bitmasks when the cassette
    // is missing; treat that the same as the no-media bit.
    if cues.status_type == BrotherStatusType::Error
        && out.is_empty()
        && (cues.media_width_mm == 0 || cues.media_type == BrotherPtMediaType::NoMedia)
    {
        push_unique(&mut out, BrotherPtError::NoMedia);
    }
    out
}

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

    let media_type = BrotherPtMediaType::from_byte(status[11]);
    let status_type = BrotherStatusType::from_byte(status[18]);
    let phase_type = BrotherPhaseType::from_byte(status[19]);
    let model_byte = status[4];
    let extended_error = status[7];
    let notification_number = status[22];
    let phase_number = u16::from_be_bytes([status[20], status[21]]);
    let media_width_mm = status[10];
    let errors = collect_errors(ErrorCues {
        error1: status[8],
        error2: status[9],
        extended_error,
        notification_number,
        phase_number,
        media_type,
        media_width_mm,
        status_type,
    });
    let media_present = media_width_mm > 0 && media_type.is_present();
    let summary = summary_from_parts(&errors, status_type, phase_type, media_present);

    Ok(BrotherPtStatus {
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
        battery_level: status[6],
        extended_error,
        notification_number,
        phase_number,
        tape_color_id: status[24],
        text_color_id: status[25],
        errors,
        summary,
    })
}

/// Best-effort media width key from status (e.g. `12` or `4` for 3.5 mm TZe).
pub fn media_key_hint(status: &BrotherPtStatus) -> Option<String> {
    if status.media_width_mm == 0 || !status.media_type.is_present() {
        return None;
    }
    Some(status.media_width_mm.to_string())
}

/// Readiness summary so UI hosts do not re-encode status bytes.
pub fn status_summary(status: &BrotherPtStatus) -> BrotherStatusSummary {
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

    fn sample_ready_12mm() -> [u8; 32] {
        let mut s = [0u8; 32];
        s[0] = 0x80;
        s[1] = 0x20;
        s[2] = b'B';
        s[3] = b'0';
        s[4] = b'g'; // PT-P700
        s[5] = b'0';
        s[6] = 0x04; // AC adapter (560-pin table)
        s[10] = 12;
        s[11] = 0x01; // laminated
        s[17] = 0;
        s[18] = 0x00;
        s[19] = 0x00;
        s[24] = 0x01; // white
        s[25] = 0x01; // black
        s
    }

    #[test]
    fn parses_ready_laminated_12mm() {
        let status = parse_status(&sample_ready_12mm()).unwrap();
        assert_eq!(status.model_code, "PT-P700");
        assert_eq!(status.media_width_mm, 12);
        assert_eq!(status.media_type, BrotherPtMediaType::LaminatedTape);
        assert_eq!(status.battery_level, 0x04);
        assert_eq!(status.tape_color_id, 0x01);
        assert_eq!(status.text_color_id, 0x01);
        assert!(status.errors.is_empty());
        assert_eq!(media_key_hint(&status).as_deref(), Some("12"));
        assert_eq!(status_summary(&status).state, "ready");
    }

    #[test]
    fn parses_fabric_and_satin_media() {
        let mut s = sample_ready_12mm();
        s[11] = 0x04;
        assert_eq!(
            parse_status(&s).unwrap().media_type,
            BrotherPtMediaType::FabricTape
        );
        s[11] = 0x15;
        assert_eq!(
            parse_status(&s).unwrap().media_type,
            BrotherPtMediaType::SatinTape
        );
    }

    #[test]
    fn parses_cover_open_error() {
        let mut s = sample_ready_12mm();
        s[9] = 1 << 4;
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.errors, vec![BrotherPtError::CoverOpen]);
        assert_eq!(status.status_type, BrotherStatusType::Error);
        assert_eq!(status_summary(&status).state, "cover_open");
    }

    #[test]
    fn folds_notification_cover_open_into_errors() {
        let mut s = sample_ready_12mm();
        s[18] = 0x05; // notification
        s[22] = 0x01; // cover open
        let status = parse_status(&s).unwrap();
        assert_eq!(status.notification_number, 0x01);
        assert_eq!(status.errors, vec![BrotherPtError::CoverOpen]);
        assert_eq!(status_summary(&status).state, "cover_open");
    }

    #[test]
    fn folds_extended_incompatible_media() {
        let mut s = sample_ready_12mm();
        s[7] = 0x21;
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.extended_error, 0x21);
        assert_eq!(status.errors, vec![BrotherPtError::IncompatibleMedia]);
        assert_eq!(status_summary(&status).state, "incompatible_media");
    }

    #[test]
    fn folds_incompatible_tape_media_type() {
        let mut s = sample_ready_12mm();
        s[11] = 0xff;
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.media_type, BrotherPtMediaType::IncompatibleTape);
        assert_eq!(status.errors, vec![BrotherPtError::IncompatibleMedia]);
        assert_eq!(status_summary(&status).state, "incompatible_media");
    }

    #[test]
    fn folds_empty_bitmask_error_with_no_tape_as_no_media() {
        let mut s = sample_ready_12mm();
        s[10] = 0;
        s[11] = 0x00;
        s[18] = 0x02;
        let status = parse_status(&s).unwrap();
        assert_eq!(status.errors, vec![BrotherPtError::NoMedia]);
        assert_eq!(status_summary(&status).state, "no_media");
    }

    #[test]
    fn rejects_bad_header() {
        let mut s = sample_ready_12mm();
        s[0] = 0x00;
        assert!(parse_status(&s).is_err());
    }
}
