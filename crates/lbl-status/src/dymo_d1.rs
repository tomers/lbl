//! DYMO LabelManager / D1 tape status (`ESC A`) parsing.
//!
//! The printer replies with a single status byte on bulk IN. Semantic bits
//! follow the LabelWriter 400/450 Series Technical Reference / thermal-label
//! d1-core protocol docs. Hosts commonly read a full 64-byte max-packet block;
//! only byte 0 is meaningful.

use crate::readiness::PrintReadiness;
use crate::StatusError;

/// Bare `ESC A` status query.
pub const STATUS_REQUEST: [u8; 2] = [0x1B, b'A'];

/// Bulk-IN drain length commonly used for the one-byte reply.
pub const STATUS_READ_LEN: usize = 64;

const CASSETTE: u8 = 0x40;
const CUTTER_JAM: u8 = 0x10;
const GENERAL_ERROR: u8 = 0x04;

/// Parsed D1 status byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DymoD1Status {
    /// Raw status byte (only low-relevant bits are documented).
    pub raw: u8,
    /// Cassette inserted (bit 6).
    pub cassette_inserted: bool,
    /// Cutter jammed (bit 4; no-op on manual-cutter chassis).
    pub cutter_jammed: bool,
    /// General error (bit 2).
    pub general_error: bool,
}

impl DymoD1Status {
    /// Whether the chassis looks idle with media and no error/jam.
    pub fn ready(self) -> bool {
        self.cassette_inserted && !self.cutter_jammed && !self.general_error
    }

    /// Machine-stable state token for UI summaries.
    pub fn state(self) -> &'static str {
        if self.general_error {
            "error"
        } else if self.cutter_jammed {
            "cutter_jam"
        } else if !self.cassette_inserted {
            "no_media"
        } else {
            "ready"
        }
    }

    /// Whether the device can accept a new print job.
    pub fn readiness(self) -> PrintReadiness {
        if self.ready() {
            PrintReadiness::ready()
        } else {
            PrintReadiness::not_ready(self.state())
        }
    }
}

/// Parse a D1 status reply (uses the first byte when longer buffers are drained).
pub fn parse_status(bytes: &[u8]) -> Result<DymoD1Status, StatusError> {
    let Some(&raw) = bytes.first() else {
        return Err(StatusError::Parse("empty D1 status reply".into()));
    };
    Ok(DymoD1Status {
        raw,
        cassette_inserted: raw & CASSETTE != 0,
        cutter_jammed: raw & CUTTER_JAM != 0,
        general_error: raw & GENERAL_ERROR != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ready_cassette() {
        let s = parse_status(&[0x40]).unwrap();
        assert!(s.cassette_inserted);
        assert!(s.ready());
        assert_eq!(s.state(), "ready");
    }

    #[test]
    fn parses_jam_and_error() {
        let jam = parse_status(&[0x50]).unwrap();
        assert!(jam.cassette_inserted && jam.cutter_jammed);
        assert_eq!(jam.state(), "cutter_jam");
        let err = parse_status(&[0x44]).unwrap();
        assert!(err.general_error);
        assert_eq!(err.state(), "error");
    }

    #[test]
    fn uses_first_byte_of_max_packet() {
        let mut buf = [0u8; 64];
        buf[0] = 0x40;
        assert!(parse_status(&buf).unwrap().ready());
    }
}
