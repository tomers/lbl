//! ESC/POS raster driver.
//!
//! Encodes the bitmap with the `GS v 0` raster bit-image command, which matches
//! the [`MonoBitmap`] layout (rows packed MSB-first, `1` = printed dot). The
//! stream is: `ESC @` (init), `GS v 0` raster, a short feed, then an optional
//! `GS V` cut.
//!
//! Also hosts Phomemo drivers: [`PhomemoDriver`] (M02-class),
//! [`PhomemoM02xDriver`] (M02X), [`PhomemoM110Driver`] (M110/M120/M220), and
//! [`PhomemoD30Driver`] (D30/Q30).

pub mod phomemo;
pub mod phomemo_d30;
pub mod phomemo_m02x;
pub mod phomemo_m110;

pub use phomemo::PhomemoDriver;
pub use phomemo_d30::PhomemoD30Driver;
pub use phomemo_m02x::PhomemoM02xDriver;
pub use phomemo_m110::PhomemoM110Driver;

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;

/// The ESC/POS driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct EscPosDriver;

impl EscPosDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }
}

impl Driver for EscPosDriver {
    fn protocol(&self) -> Protocol {
        Protocol::EscPos
    }

    fn name(&self) -> &'static str {
        "escpos-raster"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let stride = bitmap.stride();
        if stride > 0xFFFF {
            return Err(DriverError::Unsupported("raster too wide".into()));
        }
        if bitmap.height > 0xFFFF {
            return Err(DriverError::Unsupported("raster too tall".into()));
        }

        let mut out = Vec::new();
        out.extend_from_slice(&[ESC, b'@']); // initialize

        let copies = ctx.copies();
        for index in 0..copies {
            // GS v 0 m xL xH yL yH [data]
            out.extend_from_slice(&[GS, b'v', b'0', 0x00]);
            out.push((stride & 0xFF) as u8);
            out.push((stride >> 8) as u8);
            out.push((bitmap.height & 0xFF) as u8);
            out.push((bitmap.height >> 8) as u8);
            out.extend_from_slice(&bitmap.data);

            // Feed a few dots so the content clears the head/cutter.
            out.extend_from_slice(&[ESC, b'd', 0x03]);

            if ctx.should_cut_after_copy(index, copies) {
                out.extend_from_slice(&[GS, b'V', 0x00]); // full cut
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::{CutMode, JobSpec};
    use lbl_core::media::Media;
    use lbl_core::printer::PrinterCapabilities;
    use lbl_core::units::Dpi;

    #[test]
    fn emits_init_and_raster_header() {
        let bmp = MonoBitmap::new(16, 2); // stride 2
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::continuous(58.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = EscPosDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(&bytes[0..2], &[ESC, b'@']);
        assert_eq!(&bytes[2..6], &[GS, b'v', b'0', 0x00]);
        assert_eq!(&bytes[6..8], &[0x02, 0x00]); // xL,xH = stride 2
        assert_eq!(&bytes[8..10], &[0x02, 0x00]); // yL,yH = height 2
    }

    #[test]
    fn cut_only_when_supported_and_requested() {
        let bmp = MonoBitmap::new(8, 1);
        let mut caps = PrinterCapabilities::default();
        let mut job = JobSpec::new(Media::continuous(58.0, Dpi(203.0)));
        job.cut_mode = CutMode::Every;

        // Requested but printer can't cut.
        let ctx = EncodeContext::new(&job, &caps);
        let no_cut = EscPosDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(!contains_subsequence(&no_cut, &[GS, b'V', 0x00]));

        // Now the printer supports it.
        caps.supports_cut = true;
        let ctx = EncodeContext::new(&job, &caps);
        let cut = EscPosDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(contains_subsequence(&cut, &[GS, b'V', 0x00]));
    }

    fn contains_subsequence(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }
}
