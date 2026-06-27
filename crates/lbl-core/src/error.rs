//! Core error type shared across the toolchain.

use std::result::Result as StdResult;

/// Errors originating from core types and conversions.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// A value was outside its valid range (e.g. a negative dimension).
    #[error("invalid value for {field}: {reason}")]
    InvalidValue {
        /// The field that failed validation.
        field: &'static str,
        /// A human-readable reason.
        reason: String,
    },

    /// A unit conversion could not be represented.
    #[error("conversion error: {0}")]
    Conversion(String),
}

/// Convenience alias used throughout `lbl-core`.
pub type Result<T> = StdResult<T, CoreError>;
