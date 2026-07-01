//! DYMO LabelWriter 550-series raster driver.
//!
//! Implements the structured print-job protocol from DYMO's *LabelWriter 550
//! Series Technical Reference* (covering the LabelWriter 550, 550 Turbo, and
//! 5XL). Unlike the LabelManager tape protocol, the LabelWriter has a horizontal
//! print head (672 dots at 57 mm; 1248 dots on the 5XL) and prints row-by-row as
//! the label feeds, which maps directly onto a row-major [`MonoBitmap`].
//!
//! ## Print job structure
//!
//! ```text
//! ESC s <job-id:u32>      start of print job
//! ESC i                   select graphics output mode (300×600 dpi feed; use only
//!                         when the raster was rendered for that mode)
//! ESC h                   select text output mode (300×300 dpi; default for lbl)
//! [ESC L <lines:u32>]     set length to continuous stock (continuous media only)
//! per label:
//!   ESC n <index:u16>     set label index
//!   ESC D <bpp:u8> <align:u8> <width:u32> <height:u32> <data...>
//!                         start of label print data (width = number of lines,
//!                         height = dots across the head); data is width lines of
//!                         roundup(height/8) bytes, MSB-first, 1 = dot printed
//!   ESC G | ESC E         short form feed (between labels) | feed to tear (last)
//! ESC Q                   end of print job
//! ```
//!
//! Multi-byte fields are encoded little-endian. The label data layout (row-major,
//! `bytes_per_line = roundup(width_dots/8)`, `1` = ink) is exactly the
//! [`MonoBitmap`] layout, so the dithered bitmap is emitted without conversion.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

use crate::ESC;

// Command bytes.
const START_JOB: u8 = b's';
const SET_MAX_LENGTH: u8 = b'L';
const SET_DENSITY: u8 = b'C';
const TEXT_MODE: u8 = b'h';
const SET_LABEL_INDEX: u8 = b'n';
const LABEL_DATA: u8 = b'D';
const SHORT_FORM_FEED: u8 = b'G'; // feed next label into print position
const FORM_FEED_TEAR: u8 = b'E'; // feed to tear position
const END_JOB: u8 = b'Q';

// `ESC D` fixed parameters.
const BITS_PER_PIXEL: u8 = 0x01;
const ALIGN_BOTTOM: u8 = 0x02;

/// Default print density (100 %) for `ESC C`.
const DEFAULT_DENSITY: u8 = 100;

/// Driver for the DYMO LabelWriter 550 series (550 / 550 Turbo / 5XL).
#[derive(Debug, Clone, Copy)]
pub struct LabelWriter550Driver {
    /// Print job id placed in the `ESC s` header.
    pub job_id: u32,
}

impl Default for LabelWriter550Driver {
    fn default() -> Self {
        Self { job_id: 1 }
    }
}

impl LabelWriter550Driver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Driver for LabelWriter550Driver {
    fn protocol(&self) -> Protocol {
        Protocol::DymoLw
    }

    fn name(&self) -> &'static str {
        "dymo-labelwriter-550"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        // ESC D fields: the head spans the bitmap's width in dots; the label is
        // printed as `lines` raster lines (the feed direction = bitmap height).
        let dots_across_head = bitmap.width;
        let lines = bitmap.height;
        let continuous = matches!(
            ctx.job.media.length,
            lbl_core::media::MediaLength::Continuous
        );

        let mut out = Vec::with_capacity(bitmap.data.len() + 64);

        // --- Print job header ---
        out.extend_from_slice(&[ESC, START_JOB]);
        out.extend_from_slice(&self.job_id.to_le_bytes());
        out.extend_from_slice(&[ESC, SET_DENSITY, DEFAULT_DENSITY]);
        out.extend_from_slice(&[ESC, TEXT_MODE]);
        if continuous {
            // Set length to continuous stock; pass the line count as the length.
            out.extend_from_slice(&[ESC, SET_MAX_LENGTH]);
            out.extend_from_slice(&lines.to_le_bytes());
        }

        // --- One label per copy ---
        let copies = ctx.copies();
        for index in 0..copies {
            out.extend_from_slice(&[ESC, SET_LABEL_INDEX]);
            out.extend_from_slice(&(index as u16).to_le_bytes());

            out.extend_from_slice(&[ESC, LABEL_DATA, BITS_PER_PIXEL, ALIGN_BOTTOM]);
            out.extend_from_slice(&lines.to_le_bytes()); // Width = number of lines
            out.extend_from_slice(&dots_across_head.to_le_bytes()); // Height = dots
            out.extend_from_slice(&bitmap.data);

            // Short form feed between labels; feed to tear after the last so the
            // finished label can be removed.
            if index + 1 < copies {
                out.extend_from_slice(&[ESC, SHORT_FORM_FEED]);
            } else {
                out.extend_from_slice(&[ESC, FORM_FEED_TEAR]);
            }
        }

        // --- Print job trailer ---
        out.extend_from_slice(&[ESC, END_JOB]);
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
    fn emits_structured_print_job() {
        // 8 dots across head (1 byte/line), 2 lines.
        let mut bmp = MonoBitmap::new(8, 2);
        bmp.set(0, 0, true); // first line, MSB set
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 1);
        let ctx = EncodeContext::new(&job, &caps);

        let bytes = LabelWriter550Driver::new().encode(&bmp, &ctx).unwrap();

        // ESC s <jobid=1 le>
        assert_eq!(&bytes[0..2], &[ESC, b's']);
        assert_eq!(&bytes[2..6], &[1, 0, 0, 0]);
        // ESC C 100 (density) + ESC h (300×300 text mode)
        assert_eq!(&bytes[6..10], &[ESC, b'C', 100, ESC]);
        assert_eq!(&bytes[10..11], b"h");
        // ESC n <index=0 le16>
        assert_eq!(&bytes[11..13], &[ESC, b'n']);
        assert_eq!(&bytes[13..15], &[0, 0]);
        // ESC D BPP Align Width(=2 lines) Height(=8 dots)
        assert_eq!(&bytes[15..19], &[ESC, b'D', 0x01, 0x02]);
        assert_eq!(&bytes[19..23], &[2, 0, 0, 0]); // width = lines
        assert_eq!(&bytes[23..27], &[8, 0, 0, 0]); // height = dots
                                                   // data: 2 bytes (first line 0x80, second 0x00)
        assert_eq!(&bytes[27..29], &[0x80, 0x00]);
        // trailer: ESC E (feed to tear, last label) then ESC Q
        assert_eq!(&bytes[29..33], &[ESC, b'E', ESC, b'Q']);
    }

    #[test]
    fn continuous_media_sets_length() {
        let bmp = MonoBitmap::new(8, 3);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::continuous(57.0, Dpi(300.0)), 1);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = LabelWriter550Driver::new().encode(&bmp, &ctx).unwrap();
        // ESC L <lines=3 le32> appears in the header.
        let needle = [ESC, b'L', 3, 0, 0, 0];
        assert!(bytes.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn copies_use_short_feed_between_and_tear_at_end() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 3);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = LabelWriter550Driver::new().encode(&bmp, &ctx).unwrap();
        // Two short form feeds (between 3 labels) + one tear feed.
        let short = bytes.windows(2).filter(|w| w == &[ESC, b'G']).count();
        let tear = bytes.windows(2).filter(|w| w == &[ESC, b'E']).count();
        assert_eq!(short, 2);
        assert_eq!(tear, 1);
        // Three label indices 0,1,2.
        assert_eq!(bytes.windows(2).filter(|w| w == &[ESC, b'n']).count(), 3);
    }
}
