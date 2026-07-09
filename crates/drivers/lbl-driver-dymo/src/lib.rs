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
//!   dymoprint). Its command stream is `ESC C 0`, `ESC B 0`, `ESC D n`, a
//!   `SYN`-prefixed line per column, then `ESC A` (status) and `ESC E` (feed/cut).
//!   Column order matches [labelle](https://github.com/labelle-org/labelle)'s
//!   `ROTATE_270` transpose when [`PrinterCapabilities::feed_reverse`] is set.
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

    fn mm_to_feed_dots(mm: f64, dpi: f64) -> u32 {
        if mm <= f64::EPSILON {
            return 0;
        }
        ((mm / 25.4) * dpi).round().max(0.0) as u32
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

    fn append_job(
        out: &mut Vec<u8>,
        bitmap: &MonoBitmap,
        lead_cols: u32,
        trail_cols: u32,
        feed_reverse: bool,
    ) -> Result<(), DriverError> {
        let bytes_per_line = bitmap.height.div_ceil(8) as usize;
        if bytes_per_line > 0xFF {
            return Err(DriverError::Unsupported(format!(
                "tape too tall: {} dots (max 2040)",
                bitmap.height
            )));
        }

        // Tape color (0 = black on white).
        out.extend_from_slice(&[ESC, b'C', 0x00]);
        // Reset dot-tab bias; firmware can carry a non-zero margin across jobs.
        out.extend_from_slice(&[ESC, b'B', 0x00]);
        // Bytes per line.
        out.extend_from_slice(&[ESC, b'D', bytes_per_line as u8]);

        let total_cols = lead_cols + bitmap.width + trail_cols;
        for i in 0..total_cols {
            out.push(SYN);
            if i < lead_cols || i >= lead_cols + bitmap.width {
                out.extend_from_slice(&vec![0u8; bytes_per_line]);
                continue;
            }
            let src_x = i - lead_cols;
            let x = if feed_reverse {
                bitmap.width - 1 - src_x
            } else {
                src_x
            };
            out.extend_from_slice(&Self::column_bytes(bitmap, x, bytes_per_line));
        }

        // Status query (conventional job terminator; host should read IN).
        out.extend_from_slice(&[ESC, b'A']);
        // Form feed (advances and cuts on cutter models).
        out.extend_from_slice(&[ESC, b'E']);
        Ok(())
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

        let dpi = ctx.capabilities.dpi.0;
        let lead_cols = ctx
            .capabilities
            .feed_lead_mm
            .map(|mm| Self::mm_to_feed_dots(mm, dpi))
            .unwrap_or(0);
        let trail_cols = ctx
            .capabilities
            .feed_trail_mm
            .map(|mm| Self::mm_to_feed_dots(mm * 2.0, dpi))
            .unwrap_or(0);
        let feed_reverse = ctx.capabilities.feed_reverse;

        let mut out = Vec::new();
        for _ in 0..ctx.copies() {
            Self::append_job(&mut out, bitmap, lead_cols, trail_cols, feed_reverse)?;
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

        // ESC C 0, ESC B 0, ESC D 1, then 3 columns each: SYN + 1 byte.
        assert_eq!(&bytes[0..3], &[ESC, b'C', 0x00]);
        assert_eq!(&bytes[3..6], &[ESC, b'B', 0x00]);
        assert_eq!(&bytes[6..9], &[ESC, b'D', 0x01]);
        assert_eq!(bytes[9], SYN);
        assert_eq!(bytes[10], 0x80); // column 0, y=0 set
                                     // ends with status + form feed
        assert_eq!(&bytes[bytes.len() - 4..], &[ESC, b'A', ESC, b'E']);
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

    #[test]
    fn feed_trail_adds_blank_columns() {
        let mut bmp = MonoBitmap::new(1, 8);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities {
            dpi: Dpi(180.0),
            feed_trail_mm: Some(12.7), // gap; driver feeds 2× → 25.4 mm ≈ 180 dots
            ..Default::default()
        };
        let job = ctx_job(1);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();
        let syn_count = bytes.iter().filter(|&&b| b == SYN).count();
        assert_eq!(syn_count, 1 + 180);
    }

    #[test]
    fn feed_reverse_swaps_column_order() {
        let mut bmp = MonoBitmap::new(2, 8);
        bmp.set(0, 0, true);
        bmp.set(1, 7, true);
        let caps = PrinterCapabilities {
            feed_reverse: true,
            ..Default::default()
        };
        let job = ctx_job(1);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();
        // First data column after headers should be right column (y=7).
        assert_eq!(bytes[9], SYN);
        assert_eq!(bytes[10], 0x01);
        assert_eq!(bytes[11], SYN);
        assert_eq!(bytes[12], 0x80);
    }
}
