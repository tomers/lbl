//! Brother QL-series raster driver.
//!
//! Implements the raster command language from Brother's *Raster Command
//! Reference* for the QL-800 / QL-810W / QL-820NWB family (720-dot head) and the
//! QL-1100 / QL-1110NWB / QL-1115NWB family (1296-dot head). The print head is
//! horizontal and consumes one raster line per row of a row-major [`MonoBitmap`].
//!
//! Head geometry is selected from [`PrinterCapabilities::max_width_mm`]: printers
//! wider than 70 mm use the QL-11xx 1296-dot / 162-byte row layout.
//!
//! ## Print job structure
//!
//! ```text
//! N × 0x00                invalidate (400 on QL-8xx, 350 on QL-11xx)
//! ESC @                   initialize
//! ESC i a 0x01            switch to raster mode
//! per page / copy:
//!   ESC i S               status information request
//!   ESC i z …             print information (media + raster line count)
//!   ESC i M …             various mode (auto-cut)
//!   ESC i A …             cut every N labels
//!   ESC i K …             expanded mode (cut-at-end)
//!   ESC i d …             margin / feed amount
//!   M 0x00                compression off
//!   per row:
//!     g 0x00 n <n bytes>  raster graphics transfer (row mirrored)
//!   0x1A / 0x0C           print with feed (last) / print (more pages follow)
//! ```
//!
//! Bit polarity matches [`MonoBitmap`]: `1` = ink. Each row is mirrored
//! left-to-right before packing, matching the head wiring used by
//! `brother_ql` and Brother's own driver.
//!
//! `lbl` is not affiliated with Brother; see the repository disclaimer.

use lbl_core::job::CutMode;
use lbl_core::media::MediaLength;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;

/// Print-head geometry for a Brother QL chassis family.
#[derive(Debug, Clone, Copy)]
struct HeadProfile {
    /// Dots across the thermal head.
    head_dots: u32,
    /// Bytes per uncompressed raster row (`head_dots / 8`).
    bytes_per_row: usize,
    /// Leading `0x00` invalidate buffer size.
    invalidate_bytes: usize,
    /// Extra right offset applied on wide (QL-11xx) heads.
    additional_offset_r: u32,
}

const HEAD_QL8XX: HeadProfile = HeadProfile {
    head_dots: 720,
    bytes_per_row: 90,
    invalidate_bytes: 400,
    additional_offset_r: 0,
};

const HEAD_QL11XX: HeadProfile = HeadProfile {
    head_dots: 1296,
    bytes_per_row: 162,
    invalidate_bytes: 350,
    additional_offset_r: 44,
};

fn head_profile(ctx: &EncodeContext<'_>) -> HeadProfile {
    if ctx.capabilities.max_width_mm > 70.0 {
        HEAD_QL11XX
    } else {
        HEAD_QL8XX
    }
}

/// Media geometry for a DK label size at 300 dpi.
#[derive(Debug, Clone, Copy)]
struct MediaGeometry {
    /// Tape / label width in whole millimeters (print-info `mwidth`).
    tape_width_mm: u8,
    /// Die-cut length in whole millimeters, or `0` for continuous (`mlength`).
    tape_length_mm: u8,
    /// Printable dots across the head for this media.
    printable_dots: u32,
    /// Right-side offset when placing the printable band on the head.
    offset_r: u32,
    /// Feed margin in dots (`ESC i d`).
    feed_margin: u16,
    /// `0x0A` continuous, `0x0B` die-cut.
    media_type: u8,
    /// Whether this size needs the QL-11xx (wide) head.
    wide_head: bool,
}

const MEDIA_CONTINUOUS: u8 = 0x0A;
const MEDIA_DIE_CUT: u8 = 0x0B;

/// Known DK sizes. Narrow (≤ 62 mm) work on all QL raster printers; wide sizes
/// are restricted to QL-11xx / QL-1050-class heads.
const MEDIA_TABLE: &[MediaGeometry] = &[
    // Continuous rolls (narrow).
    MediaGeometry {
        tape_width_mm: 12,
        tape_length_mm: 0,
        printable_dots: 106,
        offset_r: 29,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 0,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 38,
        tape_length_mm: 0,
        printable_dots: 413,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 50,
        tape_length_mm: 0,
        printable_dots: 554,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 54,
        tape_length_mm: 0,
        printable_dots: 590,
        offset_r: 0,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 62,
        tape_length_mm: 0,
        printable_dots: 696,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: false,
    },
    // Continuous rolls (wide / QL-11xx).
    MediaGeometry {
        tape_width_mm: 102,
        tape_length_mm: 0,
        printable_dots: 1164,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 103,
        tape_length_mm: 0,
        printable_dots: 1200,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 104,
        tape_length_mm: 0,
        printable_dots: 1200,
        offset_r: 12,
        feed_margin: 35,
        media_type: MEDIA_CONTINUOUS,
        wide_head: true,
    },
    // Die-cut labels (narrow).
    MediaGeometry {
        tape_width_mm: 17,
        tape_length_mm: 54,
        printable_dots: 165,
        offset_r: 0,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 17,
        tape_length_mm: 87,
        printable_dots: 165,
        offset_r: 0,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 23,
        tape_length_mm: 23,
        printable_dots: 202,
        offset_r: 42,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 42,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 62,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 29,
        tape_length_mm: 90,
        printable_dots: 306,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 38,
        tape_length_mm: 90,
        printable_dots: 413,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 39,
        tape_length_mm: 48,
        printable_dots: 425,
        offset_r: 6,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 52,
        tape_length_mm: 29,
        printable_dots: 578,
        offset_r: 0,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 62,
        tape_length_mm: 29,
        printable_dots: 696,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 62,
        tape_length_mm: 100,
        printable_dots: 696,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: false,
    },
    // Die-cut labels (wide / QL-11xx). Brother reports 103×164 as width 104.
    MediaGeometry {
        tape_width_mm: 102,
        tape_length_mm: 51,
        printable_dots: 1164,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 102,
        tape_length_mm: 152,
        printable_dots: 1164,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 102,
        tape_length_mm: 153,
        printable_dots: 1164,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 103,
        tape_length_mm: 164,
        printable_dots: 1200,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 104,
        tape_length_mm: 164,
        printable_dots: 1200,
        offset_r: 12,
        feed_margin: 0,
        media_type: MEDIA_DIE_CUT,
        wide_head: true,
    },
];

/// Driver for Brother QL-800 / QL-810W / QL-820NWB(c) and QL-1100-family raster printers.
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

    fn resolve_media(ctx: &EncodeContext<'_>, head: HeadProfile) -> MediaGeometry {
        let wide = head.head_dots > 720;
        let width_mm = ctx.job.media.width_mm.round().clamp(1.0, 255.0) as u8;
        let candidates = MEDIA_TABLE.iter().copied().filter(|g| wide || !g.wide_head);
        match ctx.job.media.length {
            MediaLength::Continuous => candidates
                .filter(|g| g.media_type == MEDIA_CONTINUOUS)
                .min_by_key(|g| (g.tape_width_mm as i16 - width_mm as i16).unsigned_abs())
                .unwrap_or(MediaGeometry {
                    tape_width_mm: width_mm,
                    tape_length_mm: 0,
                    printable_dots: head.head_dots.saturating_sub(24),
                    offset_r: 12,
                    feed_margin: 35,
                    media_type: MEDIA_CONTINUOUS,
                    wide_head: wide,
                }),
            MediaLength::Fixed(len_mm) => {
                let length_mm = len_mm.round().clamp(1.0, 255.0) as u8;
                candidates
                    .filter(|g| g.media_type == MEDIA_DIE_CUT)
                    .min_by_key(|g| {
                        let dw = (g.tape_width_mm as i16 - width_mm as i16).unsigned_abs();
                        let dl = (g.tape_length_mm as i16 - length_mm as i16).unsigned_abs();
                        (dw as u32) * 1000 + dl as u32
                    })
                    .unwrap_or(MediaGeometry {
                        tape_width_mm: width_mm,
                        tape_length_mm: length_mm,
                        printable_dots: head.head_dots.saturating_sub(24),
                        offset_r: 12,
                        feed_margin: 0,
                        media_type: MEDIA_DIE_CUT,
                        wide_head: wide,
                    })
            }
        }
    }

    /// Place `bitmap` onto a full-width head row, clipped to the media's
    /// printable band and right-offset like Brother's driver.
    fn pad_to_head(
        bitmap: &MonoBitmap,
        geom: MediaGeometry,
        head: HeadProfile,
    ) -> Result<MonoBitmap, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let offset_r = geom.offset_r + head.additional_offset_r;
        let use_w = bitmap.width.min(geom.printable_dots).min(head.head_dots);
        let src_x0 = bitmap.width.saturating_sub(use_w) / 2;
        let max_start = head.head_dots.saturating_sub(use_w);
        let start_x = max_start.saturating_sub(offset_r).min(max_start);

        let mut out = MonoBitmap::new(head.head_dots, bitmap.height);
        for y in 0..bitmap.height {
            for x in 0..use_w {
                if bitmap.get(src_x0 + x, y) {
                    out.set(start_x + x, y, true);
                }
            }
        }
        Ok(out)
    }

    /// Pack one mirrored head row (MSB-first, 1 = ink).
    fn pack_mirrored_row(bitmap: &MonoBitmap, y: u32, head: HeadProfile) -> Vec<u8> {
        let mut row = vec![0u8; head.bytes_per_row];
        for x in 0..head.head_dots {
            if bitmap.get(x, y) {
                let mx = head.head_dots - 1 - x;
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
        head: HeadProfile,
        opts: PageEncodeOpts,
    ) {
        let PageEncodeOpts {
            auto_cut,
            cut_at_end,
            quality,
            first_page,
            last_page,
        } = opts;
        out.extend_from_slice(&[ESC, b'i', b'S']);

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

        // ESC i M: bit 6 = auto cut. ESC i A: cut every N labels (required with auto cut).
        // ESC i K: bit 3 = cut at end.
        out.extend_from_slice(&[ESC, b'i', b'M', if auto_cut { 1 << 6 } else { 0 }]);
        if auto_cut {
            out.extend_from_slice(&[ESC, b'i', b'A', 0x01]);
        }
        out.extend_from_slice(&[ESC, b'i', b'K', if cut_at_end { 1 << 3 } else { 0 }]);
        out.extend_from_slice(&[ESC, b'i', b'd']);
        out.extend_from_slice(&geom.feed_margin.to_le_bytes());
        out.extend_from_slice(&[b'M', 0x00]);

        for y in 0..bitmap.height {
            let row = Self::pack_mirrored_row(bitmap, y, head);
            out.extend_from_slice(&[b'g', 0x00, head.bytes_per_row as u8]);
            out.extend_from_slice(&row);
        }

        out.push(if last_page { 0x1A } else { 0x0C });
    }
}

#[derive(Debug, Clone, Copy)]
struct PageEncodeOpts {
    auto_cut: bool,
    cut_at_end: bool,
    quality: bool,
    first_page: bool,
    last_page: bool,
}

impl Driver for BrotherQlDriver {
    fn protocol(&self) -> Protocol {
        Protocol::BrotherQl
    }

    fn name(&self) -> &'static str {
        "brother-ql"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let head = head_profile(ctx);
        let geom = Self::resolve_media(ctx, head);
        let bitmap = Self::pad_to_head(bitmap, geom, head)?;
        let copies = ctx.copies();
        let (auto_cut, cut_at_end) = match ctx.cut_mode() {
            CutMode::None => (false, false),
            CutMode::Every => (true, false),
            CutMode::End => (false, true),
        };

        let mut out = Vec::with_capacity(
            head.invalidate_bytes
                + 64
                + copies as usize
                    * (bitmap.data.len() + bitmap.height as usize * (3 + head.bytes_per_row) + 48),
        );
        out.extend(std::iter::repeat_n(0u8, head.invalidate_bytes));
        out.extend_from_slice(&[ESC, b'@']);
        out.extend_from_slice(&[ESC, b'i', b'a', 0x01]);

        for index in 0..copies {
            Self::push_page(
                &mut out,
                &bitmap,
                geom,
                head,
                PageEncodeOpts {
                    auto_cut,
                    cut_at_end,
                    quality: self.quality_priority,
                    first_page: index == 0,
                    last_page: index + 1 == copies,
                },
            );
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

    fn ctx_job(
        media: Media,
        copies: u32,
        cut_mode: CutMode,
        max_width_mm: f64,
    ) -> (JobSpec, PrinterCapabilities) {
        let mut job = JobSpec::new(media);
        job.copies = copies;
        job.cut_mode = cut_mode;
        let caps = PrinterCapabilities {
            supports_cut: true,
            dpi: Dpi(300.0),
            max_width_mm,
            ..Default::default()
        };
        (job, caps)
    }

    #[test]
    fn emits_invalidate_init_and_raster_mode() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, CutMode::Every, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        assert!(bytes[..HEAD_QL8XX.invalidate_bytes].iter().all(|&b| b == 0));
        assert_eq!(
            &bytes[HEAD_QL8XX.invalidate_bytes..HEAD_QL8XX.invalidate_bytes + 6],
            &[ESC, b'@', ESC, b'i', b'a', 0x01]
        );
        assert_eq!(*bytes.last().unwrap(), 0x1A);
    }

    #[test]
    fn continuous_62mm_sets_media_type_and_width() {
        let bmp = MonoBitmap::new(8, 2);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, CutMode::Every, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        let needle = [ESC, b'i', b'z'];
        let pos = bytes.windows(3).position(|w| w == needle).unwrap();
        assert_eq!(bytes[pos + 4], MEDIA_CONTINUOUS);
        assert_eq!(bytes[pos + 5], 62);
        assert_eq!(bytes[pos + 6], 0);
        assert_eq!(&bytes[pos + 7..pos + 11], &[2, 0, 0, 0]);
    }

    #[test]
    fn die_cut_29x90_sets_length() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(
            Media::fixed(29.0, 90.0, Dpi(300.0)),
            1,
            CutMode::Every,
            62.0,
        );
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        let needle = [ESC, b'i', b'z'];
        let pos = bytes.windows(3).position(|w| w == needle).unwrap();
        assert_eq!(bytes[pos + 4], MEDIA_DIE_CUT);
        assert_eq!(bytes[pos + 5], 29);
        assert_eq!(bytes[pos + 6], 90);
    }

    #[test]
    fn cut_every_enables_auto_cut() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, CutMode::Every, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 0]));
    }

    #[test]
    fn cut_at_end_enables_expanded_cut_flag() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 2, CutMode::End, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 0]));
        assert!(!bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));
    }

    #[test]
    fn no_cut_when_not_requested() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, CutMode::None, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 0]));
        assert!(!bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
    }

    #[test]
    fn copies_use_form_feed_between_and_eof_at_end() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 3, CutMode::Every, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(bytes.iter().filter(|&&b| b == 0x0C).count(), 2);
        assert_eq!(bytes.iter().filter(|&&b| b == 0x1A).count(), 1);
    }

    #[test]
    fn raster_row_is_mirrored_and_full_head_width() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, CutMode::None, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        let g = bytes
            .windows(3)
            .position(|w| w == [b'g', 0x00, 90])
            .unwrap();
        let row = &bytes[g + 3..g + 3 + 90];
        assert_eq!(row[2] & (1 << 4), 1 << 4);
        assert!(row.iter().sum::<u8>() == 1 << 4);
    }

    #[test]
    fn wide_62mm_bitmap_is_center_cropped() {
        let mut bmp = MonoBitmap::new(732, 1);
        bmp.set(18, 0, true);
        let (job, caps) = ctx_job(Media::continuous(62.0, Dpi(300.0)), 1, CutMode::None, 62.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(!bytes.is_empty());
        let g = bytes
            .windows(3)
            .position(|w| w == [b'g', 0x00, 90])
            .unwrap();
        let row = &bytes[g + 3..g + 3 + 90];
        assert_ne!(row[88] & (1 << 4), 0);
    }

    #[test]
    fn ql1100_uses_162_byte_rows_and_wide_media() {
        let bmp = MonoBitmap::new(8, 1);
        let (job, caps) = ctx_job(
            Media::fixed(102.0, 152.0, Dpi(300.0)),
            1,
            CutMode::Every,
            103.0,
        );
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = BrotherQlDriver::new().encode(&bmp, &ctx).unwrap();

        assert!(bytes[..HEAD_QL11XX.invalidate_bytes]
            .iter()
            .all(|&b| b == 0));
        assert!(bytes.windows(3).any(|w| w == [b'g', 0x00, 162]));

        let needle = [ESC, b'i', b'z'];
        let pos = bytes.windows(3).position(|w| w == needle).unwrap();
        assert_eq!(bytes[pos + 4], MEDIA_DIE_CUT);
        assert_eq!(bytes[pos + 5], 102);
        assert_eq!(bytes[pos + 6], 152);
    }
}
