//! Brother QL-series raster driver.
//!
//! Implements the raster command language from Brother's *Raster Command
//! Reference* for the QL-800 / QL-810W / QL-820NWB family (including the
//! QL-820NWBc). The print head is horizontal (720 dots / 90 bytes at 300 dpi)
//! and consumes one raster line per row of a row-major [`MonoBitmap`].
//!
//! ## Print job structure
//!
//! ```text
//! 400 × 0x00              invalidate (QL-8xx)
//! ESC @                   initialize
//! ESC i a 0x01            switch to raster mode
//! per page / copy:
//!   ESC i S               status information request
//!   ESC i z …             print information (media + raster line count)
//!   ESC i M …             various mode (auto-cut)
//!   ESC i A …             cut every N labels
//!   ESC i K …             expanded mode (cut-at-end)
//!   ESC i d …             margin / feed amount
//!   M 0x00                compression off (uncompressed 90-byte rows)
//!   per row:
//!     g 0x00 90 <90 bytes>   raster graphics transfer (row mirrored)
//!   0x1A / 0x0C           print with feed (last) / print (more pages follow)
//! ```
//!
//! Bit polarity matches [`MonoBitmap`]: `1` = ink. Each row is mirrored
//! left-to-right before packing, matching the head wiring used by
//! `brother_ql` and Brother's own driver.
//!
//! `lbl` is not affiliated with Brother; see the repository disclaimer.

use lbl_core::media::MediaLength;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;

/// Dots across the QL-8xx print head (90 bytes × 8).
const HEAD_DOTS: u32 = 720;
const BYTES_PER_ROW: usize = 90;
/// Invalidate buffer size for QL-800 / QL-810W / QL-820NWB(c).
const INVALIDATE_BYTES: usize = 400;

/// Media geometry for a DK label size at 300 dpi.
///
/// Values follow the `brother_ql` label table (printable width, right offset on
/// the 720-dot head, and feed margin).
#[derive(Debug, Clone, Copy)]
struct MediaGeometry {
    /// Tape / label width in whole millimeters (print-info `mwidth`).
    tape_width_mm: u8,
    /// Die-cut length in whole millimeters, or `0` for continuous (`mlength`).
    tape_length_mm: u8,
    /// Printable dots across the head for this media.
    printable_dots: u32,
    /// Right-side offset when placing the printable band on the 720-dot head.
    offset_r: u32,
    /// Feed margin in dots (`ESC i d`).
    feed_margin: u16,
    /// `0x0A` continuous, `0x0B` die-cut.
    media_type: u8,
}

const MEDIA_CONTINUOUS: u8 = 0x0A;
const MEDIA_DIE_CUT: u8 = 0x0B;

/// Known DK sizes that fit the QL-820 (≤ 62 mm).
const MEDIA_TABLE: &[MediaGeometry] = &[
    // Continuous rolls.
    MediaGeometry {
        tape_width_mm: 12,
        tape_length_mm: 0,
        printable_dots: 106,
        offset_r: 29,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 0,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
    },
    MediaGeometry {
        tape_width_mm: 38,
        tape_length_mm: 0,
        printable_dots: 413,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
    },
    MediaGeometry {
        tape_width_mm: 50,
        tape_length_mm: 0,
        printable_dots: 554,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
    },
    MediaGeometry {
        tape_width_mm: 54,
        tape_length_mm: 0,
        printable_dots: 590,
        offset_r: 0,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
    },
    MediaGeometry {
        tape_width_mm: 62,
        tape_length_mm: 0,
        printable_dots: 696,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
    },
    // Die-cut labels.
    MediaGeometry {
        tape_width_mm: 17,
        tape_length_mm: 54,
        printable_dots: 165,
        offset_r: 0,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 17,
        tape_length_mm: 87,
        printable_dots: 165,
        offset_r: 0,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 23,
        tape_length_mm: 23,
        printable_dots: 202,
        offset_r: 42,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 42,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 90,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 38,
        tape_length_mm: 90,
        printable_dots: 413,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 39,
        tape_length_mm: 48,
        printable_dots: 425,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 52,
        tape_length_mm: 29,
        printable_dots: 578,
        offset_r: 0,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 62,
        tape_length_mm: 29,
        printable_dots: 696,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
    MediaGeometry {
        tape_width_mm: 62,
        tape_length_mm: 100,
        printable_dots: 696,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
    },
];

/// Driver for Brother QL-800 / QL-810W / QL-820NWB(c) raster printers.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrotherQlDriver {
    /// Prefer quality over speed (`pquality` flag in print information).
    pub quality_priority: bool,
}

impl BrotherQlDriver {
    /// Create a driver with quality-priority printing enabled.
    pub fn new() -> Self {
        Self {
            quality_priority: true,
        }
    }

    fn resolve_media(ctx: &EncodeContext<'_>) -> MediaGeometry {
        let width_mm = ctx.job.media.width_mm.round().clamp(1.0, 255.0) as u8;
        match ctx.job.media.length {
            MediaLength::Continuous => MEDIA_TABLE
                .iter()
                .copied()
                .filter(|g| g.media_type == MEDIA_CONTINUOUS)
                .min_by_key(|g| (g.tape_width_mm as i16 - width_mm as i16).unsigned_abs())
                .unwrap_or(MediaGeometry {
                    tape_width_mm: width_mm,
                    tape_length_mm: 0,
                    printable_dots: HEAD_DOTS.saturating_sub(24),
                    offset_r: 12,
                    feed_margin: 35,
                    media_type: MEDIA_CONTINUOUS,
                }),
            MediaLength::Fixed(len_mm) => {
                let length_mm = len_mm.round().clamp(1.0, 255.0) as u8;
                MEDIA_TABLE
                    .iter()
                    .copied()
                    .filter(|g| g.media_type == MEDIA_DIE_CUT)
                    .min_by_key(|g| {
                        let dw = (g.tape_width_mm as i16 - width_mm as i16).unsigned_abs();
                        let dl = (g.tape_length_mm as i16 - length_mm as i16).unsigned_abs();
                        (dw as u32) * 1000 + dl as u32
                    })
                    .unwrap_or(MediaGeometry {
                        tape_width_mm: width_mm,
                        tape_length_mm: length_mm,
                        printable_dots: HEAD_DOTS.saturating_sub(24),
                        offset_r: 12,
                        feed_margin: 0,
                        media_type: MEDIA_DIE_CUT,
                    })
            }
        }
    }

    /// Place `bitmap` onto a full-width head row, clipped to the media's
    /// printable band and right-offset like Brother's driver.
    ///
    /// Bitmaps wider than the printable band (common when the catalog lists
    /// marketed tape width, e.g. 62 mm → 732 dots) are center-cropped so the
    /// non-printable edge margins are dropped.
    fn pad_to_head(bitmap: &MonoBitmap, geom: MediaGeometry) -> Result<MonoBitmap, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let use_w = bitmap.width.min(geom.printable_dots).min(HEAD_DOTS);
        let src_x0 = bitmap.width.saturating_sub(use_w) / 2;
        let max_start = HEAD_DOTS.saturating_sub(use_w);
        let start_x = max_start.saturating_sub(geom.offset_r).min(max_start);

        let mut out = MonoBitmap::new(HEAD_DOTS, bitmap.height);
        for y in 0..bitmap.height {
            for x in 0..use_w {
                if bitmap.get(src_x0 + x, y) {
                    out.set(start_x + x, y, true);
                }
            }
        }
        Ok(out)
    }

    /// Pack one mirrored head row (720 dots → 90 bytes, MSB-first, 1 = ink).
    fn pack_mirrored_row(bitmap: &MonoBitmap, y: u32) -> [u8; BYTES_PER_ROW] {
        let mut row = [0u8; BYTES_PER_ROW];
        for x in 0..HEAD_DOTS {
            if bitmap.get(x, y) {
                let mx = HEAD_DOTS - 1 - x;
                let byte = (mx / 8) as usize;
                let bit = 7 - (mx % 8);
                row[byte] |= 1 << bit;
            }
        }
        row
    }

    fn push_page(
        out: &mut Vec<u8>,
        bitmap: &MonoBitmap,
        geom: MediaGeometry,
        cut: bool,
        quality: bool,
        first_page: bool,
        last_page: bool,
    ) {
        // Status information request.
        out.extend_from_slice(&[ESC, b'i', b'S']);

        // Print information: ESC i z <flags> <type> <width> <length> <lines:u32le> <page> 0x00
        let mut flags: u8 = 0x80 | (1 << 1) | (1 << 2) | (1 << 3);
        if quality {
            flags |= 1 << 6;
        }
        out.extend_from_slice(&[ESC, b'i', b'z', flags]);
        out.push(geom.media_type);
        out.push(geom.tape_width_mm);
        out.push(geom.tape_length_mm);
        out.extend_from_slice(&bitmap.height.to_le_bytes());
        out.push(if first_page { 0 } else { 1 });
        out.push(0x00);

        // Various mode: auto-cut bit 6.
        out.extend_from_slice(&[ESC, b'i', b'M', if cut { 1 << 6 } else { 0 }]);
        // Cut every 1 label when cutting.
        if cut {
            out.extend_from_slice(&[ESC, b'i', b'A', 0x01]);
        }
        // Expanded mode: cut-at-end bit 3.
        out.extend_from_slice(&[ESC, b'i', b'K', if cut { 1 << 3 } else { 0 }]);
        // Margins.
        out.extend_from_slice(&[ESC, b'i', b'd']);
        out.extend_from_slice(&geom.feed_margin.to_le_bytes());
        // No compression.
        out.extend_from_slice(&[b'M', 0x00]);

        for y in 0..bitmap.height {
            let row = Self::pack_mirrored_row(bitmap, y);
            out.extend_from_slice(&[b'g', 0x00, BYTES_PER_ROW as u8]);
            out.extend_from_slice(&row);
        }

        // 0x1A = print with feeding (last page); 0x0C = print (more follow).
        out.push(if last_page { 0x1A } else { 0x0C });
    }
}

impl Driver for BrotherQlDriver {
    fn protocol(&self) -> Protocol {
        Protocol::BrotherQl
    }

    fn name(&self) -> &'static str {
        "brother-ql"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let geom = Self::resolve_media(ctx);
        let bitmap = Self::pad_to_head(bitmap, geom)?;
        let copies = ctx.copies();
        let cut = ctx.should_cut();

        let mut out = Vec::with_capacity(
            INVALIDATE_BYTES
                + 64
                + copies as usize * (bitmap.data.len() + bitmap.height as usize * 3 + 48),
        );
        out.extend(std::iter::repeat_n(0u8, INVALIDATE_BYTES));
        out.extend_from_slice(&[ESC, b'@']); // initialize
        out.extend_from_slice(&[ESC, b'i', b'a', 0x01]); // raster mode

        for index in 0..copies {
            Self::push_page(
                &mut out,
                &bitmap,
                geom,
                cut,
                self.quality_priority,
                index == 0,
                index + 1 == copies,
            );
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

    fn ctx_job(media: Media, copies: u32, cut: bool) -> (JobSpec, PrinterCapabilities) {
        let mut job = JobSpec::new(media);
        job.copies = copies;
        job.cut = cut;
        let caps = PrinterCapabilities {
            supports_cut: true,
            dpi: Dpi(300.0),
            max_width_mm: 62.0,
            ..Default::default()
        };
        (job, caps)
    }

    #[test]
    fn emits_invalidate_init_and_raster_mode() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, true);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        assert!(bytes[..INVALIDATE_BYTES].iter().all(|&b| b == 0));
        assert_eq!(
            &bytes[INVALIDATE_BYTES..INVALIDATE_BYTES + 6],
            &[ESC, b'@', ESC, b'i', b'a', 0x01]
        );
        assert_eq!(*bytes.last().unwrap(), 0x1A);
    }

    #[test]
    fn continuous_62mm_sets_media_type_and_width() {
        let bmp = MonoBitmap::new(8, 2);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, true);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        let needle = [ESC, b'i', b'z'];
        let pos = bytes.windows(3).position(|w| w == needle).unwrap();
        assert_eq!(bytes[pos + 4], MEDIA_CONTINUOUS);
        assert_eq!(bytes[pos + 5], 62);
        assert_eq!(bytes[pos + 6], 0);
        // raster lines = 2 (le32)
        assert_eq!(&bytes[pos + 7..pos + 11], &[2, 0, 0, 0]);
    }

    #[test]
    fn die_cut_29x90_sets_length() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::fixed(29.0, 90.0, Dpi(300.0)), 1, true);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        let needle = [ESC, b'i', b'z'];
        let pos = bytes.windows(3).position(|w| w == needle).unwrap();
        assert_eq!(bytes[pos + 4], MEDIA_DIE_CUT);
        assert_eq!(bytes[pos + 5], 29);
        assert_eq!(bytes[pos + 6], 90);
    }

    #[test]
    fn cut_commands_when_supported() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, true);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));
    }

    #[test]
    fn no_cut_when_not_requested() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, false);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 0]));
        assert!(!bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
    }

    #[test]
    fn copies_use_form_feed_between_and_eof_at_end() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 3, true);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(bytes.iter().filter(|&&b| b == 0x0C).count(), 2);
        assert_eq!(bytes.iter().filter(|&&b| b == 0x1A).count(), 1);
    }

    #[test]
    fn raster_row_is_mirrored_and_full_head_width() {
        // Ink at x=0 should land in the last bit of the last byte after mirror.
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, false);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        let g = bytes
            .windows(3)
            .position(|w| w == [b'g', 0x00, 90])
            .unwrap();
        let row = &bytes[g + 3..g + 3 + 90];
        // start_x for 8-dot content on 62mm: printable 696, offset_r 12 → start = 720-8-12 = 700
        // After mirror, physical x=700 → mx = 19 → byte 2, bit 4 (7-3? 19%8=3 → bit 7-3=4)
        assert_eq!(row[2] & (1 << 4), 1 << 4);
        assert!(row.iter().sum::<u8>() == 1 << 4);
    }

    #[test]
    fn wide_62mm_bitmap_is_center_cropped() {
        // Marketed 62 mm at 300 dpi is 732 dots; printable band is 696.
        let mut bmp = MonoBitmap::new(732, 1);
        bmp.set(18, 0, true); // first printable column after 18-dot left margin
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, false);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(!bytes.is_empty());
        let g = bytes
            .windows(3)
            .position(|w| w == [b'g', 0x00, 90])
            .unwrap();
        let row = &bytes[g + 3..g + 3 + 90];
        // src col 18 → printable x=0 → head x=12 → mirror mx=707 → byte 88 bit 4
        assert_ne!(row[88] & (1 << 4), 0);
    }
}
