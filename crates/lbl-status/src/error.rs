//! Errors produced while building status queries, parsing replies, or gating
//! dispatch on readiness.
//!
//! This layer is transport-agnostic: it never opens a device. Callers that own
//! a transport (USB, BLE, network) map these errors onto their own error type
//! at the I/O boundary.

use crate::PrintReadiness;

/// Errors produced while building status queries or parsing replies.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatusError {
    /// A reply could not be parsed, or a query is unsupported for the protocol.
    #[error("{0}")]
    Parse(String),
}

/// The device is not ready to accept a print/cut job.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "printer not ready to print{}",
    .reason.as_ref().map(|r| format!(": {r}")).unwrap_or_default()
)]
pub struct NotReadyError {
    /// Machine-stable reason token from [`PrintReadiness::reason`], when known.
    pub reason: Option<String>,
}

impl NotReadyError {
    pub fn from_readiness(readiness: &PrintReadiness) -> Self {
        Self {
            reason: readiness.reason.clone(),
        }
    }
}
