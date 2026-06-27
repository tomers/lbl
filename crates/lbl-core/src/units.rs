//! Physical and device units used to size labels.
//!
//! The toolchain works in three units and converts between them via the device
//! resolution ([`Dpi`]):
//! - [`Millimeters`]: physical, what humans and media specs use.
//! - [`Dots`]: device pixels, what the printer head and rasterizer use.
//! - [`Dpi`]: dots per inch, the bridge between the two.

use serde::{Deserialize, Serialize};

const MM_PER_INCH: f64 = 25.4;

/// A length in millimeters.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Millimeters(pub f64);

/// A count of device dots (pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dots(pub u32);

/// Device resolution in dots per inch.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Dpi(pub f64);

impl Millimeters {
    /// Convert this physical length to device [`Dots`] at the given [`Dpi`],
    /// rounding to the nearest whole dot.
    pub fn to_dots(self, dpi: Dpi) -> Dots {
        let dots = (self.0 / MM_PER_INCH) * dpi.0;
        Dots(dots.round().max(0.0) as u32)
    }
}

impl Dots {
    /// Convert device dots back to physical [`Millimeters`] at the given [`Dpi`].
    pub fn to_mm(self, dpi: Dpi) -> Millimeters {
        Millimeters((self.0 as f64 / dpi.0) * MM_PER_INCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mm_to_dots_at_300dpi() {
        // 25.4 mm == 1 inch == 300 dots at 300 dpi.
        assert_eq!(Millimeters(25.4).to_dots(Dpi(300.0)), Dots(300));
    }

    #[test]
    fn roundtrip_is_close() {
        let mm = Millimeters(54.0);
        let back = mm.to_dots(Dpi(300.0)).to_mm(Dpi(300.0));
        assert!((back.0 - mm.0).abs() < 0.1);
    }
}
