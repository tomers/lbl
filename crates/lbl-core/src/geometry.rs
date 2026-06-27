//! Simple geometry helpers for label layout.

use serde::{Deserialize, Serialize};

/// A width/height pair, generic over the unit type (e.g. `Size<Dots>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size<T> {
    /// Width.
    pub width: T,
    /// Height.
    pub height: T,
}

impl<T> Size<T> {
    /// Create a new size.
    pub fn new(width: T, height: T) -> Self {
        Self { width, height }
    }
}

/// Margins around the printable area, in millimeters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Margins {
    /// Top margin (mm).
    pub top: f64,
    /// Right margin (mm).
    pub right: f64,
    /// Bottom margin (mm).
    pub bottom: f64,
    /// Left margin (mm).
    pub left: f64,
}

impl Margins {
    /// Uniform margins on all four sides.
    pub fn uniform(mm: f64) -> Self {
        Self {
            top: mm,
            right: mm,
            bottom: mm,
            left: mm,
        }
    }
}
