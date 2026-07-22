//! Error type for status queries and reply parsing.

/// Errors produced while building status queries or parsing replies.
///
/// This layer is transport-agnostic: it never opens a device. Callers that own
/// a transport (USB, BLE, network) map [`StatusError`] onto their own error
/// type at the I/O boundary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatusError {
    /// A reply could not be parsed, or a query is unsupported for the protocol.
    #[error("{0}")]
    Parse(String),
}
