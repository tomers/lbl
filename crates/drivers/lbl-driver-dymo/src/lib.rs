//! DYMO drivers for `lbl`.
//!
//! This crate hosts two DYMO drivers, because DYMO uses two very different
//! protocols:
//!
//! - [`DymoDriver`] — the **LabelManager** tape protocol (this module). DYMO
//!   tape printers have a vertical print head: each transmitted "line" is one
//!   **column** of dots across the tape, and the tape feeds horizontally, so the
//!   encoder transposes the bitmap into columns. Modeled on the command set used
//!   by [labelle](https://github.com/labelle-org/labelle) (derived from
//!   dymoprint). Its command stream is `ESC C 0`, `ESC D n`, a `SYN`-prefixed
//!   line per column, then `ESC E`.
//! - [`LabelWriter550Driver`] — the **LabelWriter 550 series** raster protocol
//!   (see [`lw550`]), per DYMO's LW 550 Technical Reference.
//!
//! `lbl` is not affiliated with DYMO; see the repository disclaimer.

pub mod lw550;

pub use lw550::LabelWriter550Driver;

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const SYN: u8 = 0x16;

/// The DYMO LabelManager driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct DymoDriver;

impl DymoDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    /// Extract column `x` of the bitmap as `bytes_per_line` packed bytes
    /// (MSB-first over the tape's vertical axis).
    fn column_bytes(bitmap: &MonoBitmap, x: u32, bytes_per_line: usize) -> Vec<u8> {
        let mut col = vec![0u8; bytes_per_line];
        for y in 0..bitmap.height {
            if bitmap.get(x, y) {
                col[(y / 8) as usize] |= 0x80 >> (y % 8);
            }
        }
        col
    }
}

impl Driver for DymoDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Dymo
    }

    fn name(&self) -> &'static str {
        "dymo-labelmanager"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.height == 0 || bitmap.width == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        let bytes_per_line = ((bitmap.height + 7) / 8) as usize;
        if bytes_per_line > 0xFF {
            return Err(DriverError::Unsupported(format!(
                "tape too tall: {} dots (max 2040)",
                bitmap.height
            )));
        }

        let mut out = Vec::new();
        for _ in 0..ctx.copies() {
            // Tape color (0 = black on white).
            out.extend_from_slice(&[ESC, b'C', 0x00]);
            // Bytes per line.
            out.extend_from_slice(&[ESC, b'D', bytes_per_line as u8]);
            // One SYN-prefixed line per column.
            for x in 0..bitmap.width {
                out.push(SYN);
                out.extend_from_slice(&Self::column_bytes(bitmap, x, bytes_per_line));
            }
            // Form feed (advances and cuts on cutter models).
            out.extend_from_slice(&[ESC, b'E']);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::JobSpec;
    use lbl_core::media::Media;
    use lbl_core::printer::PrinterCapabilities;
    use lbl_core::units::Dpi;

    fn ctx_job(copies: u32) -> JobSpec {
        let mut job = JobSpec::new(Media::continuous(12.0, Dpi(180.0)));
        job.copies = copies;
        job
    }

    #[test]
    fn encodes_columns_with_syn() {
        let mut bmp = MonoBitmap::new(3, 8);
        bmp.set(0, 0, true);
        bmp.set(2, 7, true);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(1);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();

        // ESC C 0, ESC D 1, then 3 columns each: SYN + 1 byte.
        assert_eq!(&bytes[0..3], &[ESC, b'C', 0x00]);
        assert_eq!(&bytes[3..6], &[ESC, b'D', 0x01]);
        assert_eq!(bytes[6], SYN);
        assert_eq!(bytes[7], 0x80); // column 0, y=0 set
        // ends with form feed
        assert_eq!(&bytes[bytes.len() - 2..], &[ESC, b'E']);
    }

    #[test]
    fn copies_repeat_stream() {
        let bmp = MonoBitmap::new(2, 8);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(3);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(bytes.iter().filter(|&&b| b == b'E').count(), 3);
    }
}
