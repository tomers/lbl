//! Cross-protocol print/cut dispatch readiness.
//!
//! Hosts must not re-derive "can we send a job?" from UI severity or badge
//! colour. Each status type fills [`PrintReadiness`] itself (see
//! `BrotherStatusSummary::readiness`, `DymoD1Status::readiness`, …); UIs map
//! `reason` to display copy.

use serde::{Deserialize, Serialize};

use crate::error::NotReadyError;
use crate::PrintStatus;

/// Whether the device can accept a new print or cut job from this snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintReadiness {
    /// `true` when idle/healthy or actively printing/busy without a blocking fault.
    pub ready_to_print: bool,
    /// Machine-stable token when not ready (`cover_open`, `no_media`, `paper_out`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PrintReadiness {
    /// Device can accept a job (or is already printing without a fault).
    pub fn ready() -> Self {
        Self {
            ready_to_print: true,
            reason: None,
        }
    }

    /// Blocking fault — do not dispatch. `reason` is a snake_case engine token.
    pub fn not_ready(reason: impl Into<String>) -> Self {
        Self {
            ready_to_print: false,
            reason: Some(reason.into()),
        }
    }
}

impl Default for PrintReadiness {
    fn default() -> Self {
        Self::ready()
    }
}

/// Refuse a job when readiness says the device cannot accept one.
///
/// - `force == true` — always allow (operator override).
/// - `ready_to_print == false` — return [`NotReadyError`] with the engine reason.
pub fn ensure_ready(readiness: &PrintReadiness, force: bool) -> Result<(), NotReadyError> {
    if force || readiness.ready_to_print {
        Ok(())
    } else {
        Err(NotReadyError::from_readiness(readiness))
    }
}

/// Refuse a job when live status says the device cannot accept one.
///
/// - `force == true` — always allow (operator override).
/// - `status.readiness()` is `None` — allow (incomplete / unknown snapshot).
/// - `ready_to_print == false` — return [`NotReadyError`] with the engine reason.
pub fn ensure_ready_to_print(status: &PrintStatus, force: bool) -> Result<(), NotReadyError> {
    match status.readiness() {
        Some(r) => ensure_ready(&r, force),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GpglHostStatus;

    #[test]
    fn ensure_ready_respects_force() {
        let status = PrintStatus::Gpgl(GpglHostStatus::Unloaded.into());
        let err = ensure_ready_to_print(&status, false).unwrap_err();
        assert_eq!(err.reason.as_deref(), Some("unloaded"));
        assert!(ensure_ready_to_print(&status, true).is_ok());
    }
}
