//! Shared Brother raster-status tokens (QL and PT).
//!
//! Machine-stable snake_case ids only — consumers map tokens to display copy.

use serde::{Deserialize, Serialize};

/// Status-type byte (offset 18) of the 32-byte `ESC i S` reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherStatusType {
    Reply,
    PrintComplete,
    Error,
    TurnedOff,
    Notification,
    PhaseChange,
    Unknown,
}

impl BrotherStatusType {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0x00 => Self::Reply,
            0x01 => Self::PrintComplete,
            0x02 => Self::Error,
            0x04 => Self::TurnedOff,
            0x05 => Self::Notification,
            0x06 => Self::PhaseChange,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::PrintComplete => "print_complete",
            Self::Error => "error",
            Self::TurnedOff => "turned_off",
            Self::Notification => "notification",
            Self::PhaseChange => "phase_change",
            Self::Unknown => "unknown",
        }
    }
}

/// Phase-type byte (offset 19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherPhaseType {
    Waiting,
    Printing,
    Unknown,
}

impl BrotherPhaseType {
    pub fn from_byte(value: u8) -> Self {
        match value {
            0x00 => Self::Waiting,
            0x01 => Self::Printing,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Printing => "printing",
            Self::Unknown => "unknown",
        }
    }
}

/// UI severity for a derived readiness summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrotherSeverity {
    Success,
    Warning,
    Error,
    Neutral,
    Primary,
}

/// Compact readiness summary so hosts/UI do not re-encode status bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrotherStatusSummary {
    /// Stable state token (`ready`, `cover_open`, `printing`, …).
    pub state: String,
    pub severity: BrotherSeverity,
}

pub(crate) fn collect_error_bits<E: Copy>(defs: &[(u8, E)], bits: u8) -> Vec<E> {
    defs.iter()
        .filter(|(bit, _)| bits & (1 << bit) != 0)
        .map(|(_, e)| *e)
        .collect()
}

pub(crate) fn summary_from_parts(
    errors: &[impl AsRef<str>],
    status_type: BrotherStatusType,
    phase_type: BrotherPhaseType,
    media_present: bool,
) -> BrotherStatusSummary {
    if let Some(err) = errors.first() {
        return BrotherStatusSummary {
            state: err.as_ref().to_string(),
            severity: BrotherSeverity::Error,
        };
    }
    match status_type {
        BrotherStatusType::Error => BrotherStatusSummary {
            state: "error".into(),
            severity: BrotherSeverity::Error,
        },
        BrotherStatusType::TurnedOff => BrotherStatusSummary {
            state: "turned_off".into(),
            severity: BrotherSeverity::Neutral,
        },
        BrotherStatusType::PrintComplete => BrotherStatusSummary {
            state: "print_complete".into(),
            severity: BrotherSeverity::Primary,
        },
        _ if phase_type == BrotherPhaseType::Printing => BrotherStatusSummary {
            state: "printing".into(),
            severity: BrotherSeverity::Primary,
        },
        _ if !media_present => BrotherStatusSummary {
            state: "no_media".into(),
            severity: BrotherSeverity::Warning,
        },
        _ => BrotherStatusSummary {
            state: "ready".into(),
            severity: BrotherSeverity::Success,
        },
    }
}
