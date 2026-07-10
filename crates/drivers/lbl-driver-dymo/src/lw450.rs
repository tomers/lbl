//! DYMO LabelWriter 450-series classic raster driver.
//!
//! The LabelWriter 450 family (450, 450 Turbo, 450 Twin Turbo, 4XL) uses a
//! row-oriented SYN-based protocol that differs from the structured job framing
//! introduced on the LW550. Each raster row is sent as a `SYN` byte followed by
//! the packed row data (MSB-first, 1 = ink), left-aligned on the physical head.
//!
//! ## Print job structure
//!
//! ```text
//! ESC @                   reset printer
//! ESC h                   select text output mode (300×300 dpi)
//! ESC D n                 set bytes per line (head_dots / 8)
//! ESC B 0                 set dot-tab to 0
//! per row y (top → bottom):
//!   SYN <row_bytes...>    one raster row, padded to head width
//! ESC G                   short form feed (between copies)
//! ESC E                   feed to tear position (after last copy)
//! ```
//!
//! Multi-copy jobs emit the full row stream for each copy with `ESC G` between
//! copies and a single `ESC E` at the end.
//!
//! Reference: DYMO LabelWriter 400/450 series host commands; community
//! documentation at <https://sbkb.dymo.com/>.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

use crate::ESC;
use crate::SYN;

const RESET: u8 = b'@';
const TEXT_MODE: u8 = b'h';
const BYTES_PER_LINE: u8 = b'D';
const DOT_TAB: u8 = b'B';
const SHORT_FORM_FEED: u8 = b'G';
const FORM_FEED_TEAR: u8 = b'E';

/// Dots across the 57 mm print head (450 / 450 Turbo / 450 Twin Turbo).
const HEAD_DOTS_57MM: u32 = 672;

/// Dots across the 101 mm print head (4XL).
const HEAD_DOTS_4XL: u32 = 1248;

/// Driver for the DYMO LabelWriter 450 series (450 / 450 Turbo / 450 Twin Turbo / 4XL).
#[derive(Debug, Default, Clone, Copy)]
pub struct LabelWriter450Driver;

impl LabelWriter450Driver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn head_dots(ctx: &EncodeContext<'_>) -> u32 {
        let from_caps = lbl_core::units::Millimeters(ctx.capabilities.max_width_mm)
            .to_dots(ctx.capabilities.dpi);
        if from_caps.0 > 900 {
            HEAD_DOTS_4XL
        } else {
            HEAD_DOTS_57MM
        }
    }

    fn pad_to_head(bitmap: &MonoBitmap, head_dots: u32) -> Result<MonoBitmap, DriverError> {
        if bitmap.width > head_dots {
            return Err(DriverError::Unsupported(format!(
                "bitmap width {} exceeds print head width {head_dots} dots",
                bitmap.width
            )));
        }
        if bitmap.width == head_dots {
            return Ok(bitmap.clone());
        }
        let mut out = MonoBitmap::new(head_dots, bitmap.height);
        for y in 0..bitmap.height {
            for x in 0..bitmap.width {
                if bitmap.get(x, y) {
                    out.set(x, y, true);
                }
            }
        }
        Ok(out)
    }

    fn emit_rows(out: &mut Vec<u8>, bitmap: &MonoBitmap) {
        for y in 0..bitmap.height {
            out.push(SYN);
            out.extend_from_slice(bitmap.row(y));
        }
    }
}

impl Driver for LabelWriter450Driver {
    fn protocol(&self) -> Protocol {
        Protocol::DymoLwClassic
    }

    fn name(&self) -> &'static str {
        "dymo-labelwriter-450"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let head_dots = Self::head_dots(ctx);
        let bitmap = Self::pad_to_head(bitmap, head_dots)?;
        let bytes_per_line = (head_dots / 8) as u8;
        let copies = ctx.copies();

        let mut out = Vec::with_capacity(bitmap.data.len() * copies as usize + 16);

        out.extend_from_slice(&[ESC, RESET]);
        out.extend_from_slice(&[ESC, TEXT_MODE]);
        out.extend_from_slice(&[ESC, BYTES_PER_LINE, bytes_per_line]);
        out.extend_from_slice(&[ESC, DOT_TAB, 0x00]);

        for i in 0..copies {
            Self::emit_rows(&mut out, &bitmap);
            if i + 1 < copies {
                out.extend_from_slice(&[ESC, SHORT_FORM_FEED]);
            }
        }

        out.extend_from_slice(&[ESC, FORM_FEED_TEAR]);
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

    fn ctx_job(media: Media, copies: u32) -> JobSpec {
        let mut job = JobSpec::new(media);
        job.copies = copies;
        job
    }

    #[test]
    fn emits_reset_and_mode_header() {
        let bmp = MonoBitmap::new(8, 2);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        assert_eq!(&bytes[0..2], &[ESC, b'@']);
        assert_eq!(&bytes[2..4], &[ESC, b'h']);
        assert_eq!(&bytes[4..7], &[ESC, b'D', 84]); // 672/8 = 84
        assert_eq!(&bytes[7..10], &[ESC, b'B', 0x00]);
    }

    #[test]
    fn syn_row_per_line() {
        let mut bmp = MonoBitmap::new(8, 2);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        let syn_count = bytes.iter().filter(|&&b| b == SYN).count();
        assert_eq!(syn_count, 2);

        let header_len = 10usize;
        assert_eq!(bytes[header_len], SYN);
        assert_eq!(bytes[header_len + 1], 0x80); // first dot set in MSB
        assert_eq!(bytes[header_len + 85], SYN); // second row: 1 + 84 bytes
        assert_eq!(bytes[header_len + 86], 0x00); // blank second row
    }

    #[test]
    fn copies_use_short_feed_between_and_tear_at_end() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 3);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        let short_ff = bytes.windows(2).filter(|w| w == &[ESC, b'G']).count();
        let tear = bytes.windows(2).filter(|w| w == &[ESC, b'E']).count();
        assert_eq!(short_ff, 2); // between copies only
        assert_eq!(tear, 1);
    }

    #[test]
    fn wide_media_uses_4xl_head() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities {
            dpi: Dpi(300.0),
            max_width_mm: 104.0,
            ..Default::default()
        };
        let job = ctx_job(Media::fixed(104.0, 159.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        // 4XL head: 1248 dots / 8 = 156 bytes per line
        assert_eq!(&bytes[4..7], &[ESC, b'D', 156]);
    }

    #[test]
    fn rejects_bitmap_wider_than_head() {
        let bmp = MonoBitmap::new(700, 1); // > 672 dots
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::fixed(57.0, 25.0, Dpi(300.0)), 1);
        let err = LabelWriter450Driver::new().encode(&bmp, &EncodeContext::new(&job, &caps));
        assert!(matches!(err, Err(DriverError::Unsupported(_))));
    }
}
