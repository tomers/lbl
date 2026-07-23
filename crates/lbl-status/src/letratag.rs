//! DYMO LetraTag LT-200B advertising-data status (passive BLE scan).

use crate::StatusError;

/// 3-byte BLE advertising manufacturer payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LetraTagAdStatus {
    pub revision: u8,
    pub cassette_id: u8,
    pub carbon_type: bool,
    pub busy_locked: bool,
    pub tape_jam: bool,
    pub cutter_jam: bool,
    pub battery_too_low: bool,
    pub battery_low: bool,
    pub battery_level: u8,
    pub charging: bool,
}

impl LetraTagAdStatus {
    pub fn state(self) -> &'static str {
        if self.tape_jam {
            "tape_jam"
        } else if self.cutter_jam {
            "cutter_jam"
        } else if self.battery_too_low {
            "battery_too_low"
        } else if self.busy_locked {
            "busy"
        } else if self.cassette_id == 0 {
            "no_media"
        } else {
            "ready"
        }
    }
}

/// Parse the 3-byte LetraTag advertising manufacturer payload.
pub fn parse_advertising_status(bytes: &[u8]) -> Result<LetraTagAdStatus, StatusError> {
    if bytes.len() < 3 {
        return Err(StatusError::Parse(format!(
            "short LetraTag advertising status ({} bytes, expected 3)",
            bytes.len()
        )));
    }
    let b0 = bytes[0];
    let b1 = bytes[1];
    let b2 = bytes[2];
    Ok(LetraTagAdStatus {
        revision: b0 >> 4,
        cassette_id: b1 & 0x0f,
        carbon_type: b1 & 0x10 != 0,
        busy_locked: b1 & 0x20 != 0,
        tape_jam: b2 & 0x01 != 0,
        cutter_jam: b2 & 0x02 != 0,
        battery_too_low: b2 & 0x04 != 0,
        battery_low: b2 & 0x08 != 0,
        battery_level: (b2 >> 4) & 0x03,
        charging: b2 & 0x40 != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_12mm_ready() {
        // revision 1, cassette 3 (12 mm), battery level 2
        let s = parse_advertising_status(&[0x10, 0x03, 0x20]).unwrap();
        assert_eq!(s.revision, 1);
        assert_eq!(s.cassette_id, 3);
        assert_eq!(s.battery_level, 2);
        assert_eq!(s.state(), "ready");
    }
}
