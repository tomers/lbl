//! DYMO LabelWriter classic (450-series) 1-byte status (`ESC A`) parsing.

use crate::readiness::PrintReadiness;
use crate::StatusError;

/// `ESC A` status request.
pub const STATUS_REQUEST: [u8; 2] = [0x1B, b'A'];

const READY: u8 = 0x01;
const TOP_OF_FORM: u8 = 0x02;
const NO_PAPER: u8 = 0x20;
const PAPER_JAM: u8 = 0x40;
const PRINTER_ERROR: u8 = 0x80;

/// Parsed classic LabelWriter status byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DymoLwClassicStatus {
    pub raw: u8,
    pub ready: bool,
    pub top_of_form: bool,
    pub no_paper: bool,
    pub paper_jam: bool,
    pub printer_error: bool,
}

impl DymoLwClassicStatus {
    pub fn state(self) -> &'static str {
        if self.paper_jam {
            "paper_jam"
        } else if self.no_paper {
            "no_paper"
        } else if self.printer_error {
            "error"
        } else if self.ready {
            "ready"
        } else {
            "not_ready"
        }
    }

    /// Whether the device can accept a new print job.
    pub fn readiness(self) -> PrintReadiness {
        if self.ready && !self.no_paper && !self.paper_jam && !self.printer_error {
            PrintReadiness::ready()
        } else {
            PrintReadiness::not_ready(self.state())
        }
    }
}

/// Parse a classic LW status reply (first byte).
pub fn parse_status(bytes: &[u8]) -> Result<DymoLwClassicStatus, StatusError> {
    let Some(&raw) = bytes.first() else {
        return Err(StatusError::Parse("empty classic LW status reply".into()));
    };
    Ok(DymoLwClassicStatus {
        raw,
        ready: raw & READY != 0,
        top_of_form: raw & TOP_OF_FORM != 0,
        no_paper: raw & NO_PAPER != 0,
        paper_jam: raw & PAPER_JAM != 0,
        printer_error: raw & PRINTER_ERROR != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_healthy_idle() {
        let s = parse_status(&[0x03]).unwrap();
        assert!(s.ready && s.top_of_form);
        assert_eq!(s.state(), "ready");
    }

    #[test]
    fn parses_no_paper() {
        let s = parse_status(&[0xA1]).unwrap();
        assert!(s.no_paper);
        assert_eq!(s.state(), "no_paper");
    }
}
