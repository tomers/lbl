//! Brother P-touch / TZe tape raster driver.
//!
//! Implements the raster command language from Brother's *Raster Command
//! Reference* for:
//! - PT-H500 / PT-P700 / PT-E500 / PT-P710BT family (128-dot head @ 180 dpi)
//! - PT-P900 / P900W / P950NW / P910BT family (560-dot head @ 360 dpi)
//!
//! Head geometry is selected from [`DeviceCapabilities::max_width_mm`]: printers
//! wider than 24 mm use the P900-class 560-dot / 70-byte row layout.
//!
//! ## Print job structure
//!
//! ```text
//! N × 0x00                invalidate (350 on P700-class, 200 on P900-class)
//! ESC @                   initialize
//! ESC i a 0x01            switch to raster mode
//! per page / copy:
//!   ESC i z …             print information (TZe width + raster line count)
//!   ESC i M …             various mode (auto-cut)
//!   ESC i K …             advanced mode (no-chain / cut-at-end)
//!   ESC i d …             margin / feed amount
//!   M 0x00                compression off
//!   per row:
//!     G n_lo n_hi <n B>   raster graphics transfer (row mirrored)
//!   0x1A / 0x0C           print with feed (last) / print (more pages follow)
//! ```
//!
//! Brother's PDF lists the row opcode as ASCII `G` / hex `47` in the command
//! panel (some editions misprint `g` / `67`). On the wire PT jobs use `G` with
//! a little-endian payload length — the same framing as brother_ql / ptouch
//! drivers. Emitting QL-style `g 00 n` desynchronizes the parser and typically
//! ends in a red-LED communication / media error on Cube-class devices.
//!
//! Bit polarity matches [`MonoBitmap`]: `1` = ink. Each row is mirrored
//! left-to-right before packing, matching Brother's own driver wiring.
//!
//! `lbl` is not affiliated with Brother; see the repository disclaimer.

use lbl_core::job::CutMode;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
/// Laminated TZe tape media type (`ESC i z` n2).
const MEDIA_LAMINATED: u8 = 0x01;
/// Default lead/trail feed (~1–2 mm depending on dpi), from Brother samples.
const DEFAULT_FEED_MARGIN: u16 = 14;

/// Print-head geometry for a Brother PT chassis family.
#[derive(Debug, Clone, Copy)]
struct HeadProfile {
    head_dots: u32,
    bytes_per_row: usize,
    invalidate_bytes: usize,
}

const HEAD_P700: HeadProfile = HeadProfile {
    head_dots: 128,
    bytes_per_row: 16,
    invalidate_bytes: 350,
};

const HEAD_P900: HeadProfile = HeadProfile {
    head_dots: 560,
    bytes_per_row: 70,
    invalidate_bytes: 200,
};

fn head_profile(ctx: &EncodeContext<'_>) -> HeadProfile {
    if ctx.capabilities.max_width_mm > 24.0 {
        HEAD_P900
    } else {
        HEAD_P700
    }
}

/// Printable band for a TZe tape width on a given head.
#[derive(Debug, Clone, Copy)]
struct MediaGeometry {
    /// Cassette width in whole millimeters (`ESC i z` n3).
    tape_width_mm: u8,
    /// Printable dots across the head for this tape.
    printable_dots: u32,
    /// Left-side margin pins before the printable band.
    offset_l: u32,
    /// Feed margin in dots (`ESC i d`).
    feed_margin: u16,
    /// Whether this size needs the P900-class (wide) head.
    wide_head: bool,
}

/// Known TZe widths for the 128-dot / 180 dpi PT family (max 24 mm).
/// 3.5 mm cassettes are reported as 4 mm in the print-information command.
const MEDIA_TABLE_P700: &[MediaGeometry] = &[
    MediaGeometry {
        tape_width_mm: 4,
        printable_dots: 24,
        offset_l: 52,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 6,
        printable_dots: 32,
        offset_l: 48,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 9,
        printable_dots: 50,
        offset_l: 39,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 12,
        printable_dots: 70,
        offset_l: 29,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 18,
        printable_dots: 112,
        offset_l: 8,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: false,
    },
    MediaGeometry {
        tape_width_mm: 24,
        printable_dots: 128,
        offset_l: 0,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: false,
    },
];

/// Known TZe widths for the 560-dot / 360 dpi PT-P900 family (max 36 mm).
const MEDIA_TABLE_P900: &[MediaGeometry] = &[
    MediaGeometry {
        tape_width_mm: 4,
        printable_dots: 48,
        offset_l: 248,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 6,
        printable_dots: 64,
        offset_l: 240,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 9,
        printable_dots: 106,
        offset_l: 219,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 12,
        printable_dots: 150,
        offset_l: 197,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 18,
        printable_dots: 234,
        offset_l: 155,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 24,
        printable_dots: 320,
        offset_l: 112,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
    MediaGeometry {
        tape_width_mm: 36,
        printable_dots: 454,
        offset_l: 45,
        feed_margin: DEFAULT_FEED_MARGIN,
        wide_head: true,
    },
];

/// Driver for Brother P-touch PT-P700 / P900-class TZe printers.
#[derive(Debug, Clone, Copy, Default)]
pub struct BrotherPtDriver {
    /// Prefer quality over speed (`PI_QUALITY` in print information).
    pub quality_priority: bool,
}

impl BrotherPtDriver {
    /// Create a driver with quality-priority printing enabled.
    pub fn new() -> Self {
        Self {
            quality_priority: true,
        }
    }

    fn resolve_media(ctx: &EncodeContext<'_>) -> (HeadProfile, MediaGeometry) {
        let head = head_profile(ctx);
        let wide = head.head_dots > HEAD_P700.head_dots;
        let mut width_mm = ctx.job.media.width_mm.round().clamp(1.0, 255.0) as u8;
        // 3.5 mm TZe cassettes use 4 mm in the print-information command.
        if (ctx.job.media.width_mm - 3.5).abs() < 0.3 {
            width_mm = 4;
        }
        let table = if wide {
            MEDIA_TABLE_P900
        } else {
            MEDIA_TABLE_P700
        };
        let geom = table
            .iter()
            .copied()
            .filter(|g| wide || !g.wide_head)
            .min_by_key(|g| (g.tape_width_mm as i16 - width_mm as i16).unsigned_abs())
            .unwrap_or(MediaGeometry {
                tape_width_mm: width_mm.min(if wide { 36 } else { 24 }),
                printable_dots: head.head_dots,
                offset_l: 0,
                feed_margin: DEFAULT_FEED_MARGIN,
                wide_head: wide,
            });
        (head, geom)
    }

    /// Place `bitmap` onto a full-width head row inside the tape's printable band.
    fn pad_to_head(
        bitmap: &MonoBitmap,
        head: HeadProfile,
        geom: MediaGeometry,
    ) -> Result<MonoBitmap, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let use_w = bitmap
            .width
            .min(geom.printable_dots)
            .min(head.head_dots.saturating_sub(geom.offset_l));
        let src_x0 = (bitmap.width - use_w) / 2;
        let max_start = head.head_dots.saturating_sub(use_w);
        let start_x = geom.offset_l.min(max_start);

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
        head: HeadProfile,
        geom: MediaGeometry,
        opts: PageEncodeOpts,
    ) {
        let PageEncodeOpts {
            auto_cut,
            cut_at_end,
            quality,
            first_page,
            last_page,
        } = opts;

        // PI_RECOVER | PI_KIND | PI_WIDTH [| PI_QUALITY]
        let mut flags: u8 = 0x80 | 0x02 | 0x04;
        if quality {
            flags |= 0x40;
        }
        out.extend_from_slice(&[ESC, b'i', b'z', flags]);
        out.push(MEDIA_LAMINATED);
        out.push(geom.tape_width_mm);
        out.push(0x00); // continuous TZe — length unused
        out.extend_from_slice(&bitmap.height.to_le_bytes());
        out.push(if first_page { 0 } else { 1 });
        out.push(0x00);

        // ESC i M bit 6 = auto cut. ESC i K bit 3 = no chain (cut/feed at end).
        out.extend_from_slice(&[ESC, b'i', b'M', if auto_cut { 1 << 6 } else { 0 }]);
        out.extend_from_slice(&[ESC, b'i', b'K', if cut_at_end { 1 << 3 } else { 0 }]);
        out.extend_from_slice(&[ESC, b'i', b'd']);
        out.extend_from_slice(&geom.feed_margin.to_le_bytes());
        out.extend_from_slice(&[b'M', 0x00]);

        for y in 0..bitmap.height {
            let row = Self::pack_mirrored_row(bitmap, y, head);
            // PT framing: `G` + little-endian u16 length + payload (not QL's
            // `g 00 <u8 length>`). Length must be head.bytes_per_row when
            // compression is off (`M 00`).
            let n = row.len() as u16;
            out.push(b'G');
            out.extend_from_slice(&n.to_le_bytes());
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

impl Driver for BrotherPtDriver {
    fn protocol(&self) -> Protocol {
        Protocol::BrotherPt
    }

    fn name(&self) -> &'static str {
        "brother-pt"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["brother-pt", "brother_pt", "brotherpt", "pt", "tze"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let (head, geom) = Self::resolve_media(ctx);
        let bitmap = Self::pad_to_head(bitmap, head, geom)?;
        let copies = ctx.copies();
        // P710BT has no `ESC i A` (cut-each-N). Auto-cut alone leaves chain
        // mode on, and Brother then skips feed/cut after the last page — so a
        // single label never gets cut. Match working PT drivers (ptouch /
        // brother_ql): "cut every" sets both auto-cut and no-chain.
        let (auto_cut, cut_at_end) = match ctx.cut_mode() {
            CutMode::None => (false, false),
            CutMode::Every => (true, true),
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
                head,
                geom,
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
    use lbl_core::printer::DeviceCapabilities;
    use lbl_core::units::Dpi;

    fn ctx_job(
        width_mm: f64,
        copies: u32,
        cut_mode: CutMode,
        dpi: f64,
        max_width_mm: f64,
    ) -> (JobSpec, DeviceCapabilities) {
        let media = Media::continuous(width_mm, Dpi(dpi));
        let mut job = JobSpec::new(media);
        job.copies = copies;
        job.cut_mode = cut_mode;
        let caps = DeviceCapabilities {
            dpi: Dpi(dpi),
            max_width_mm,
            supports_cut: true,
            ..DeviceCapabilities::default()
        };
        (job, caps)
    }

    #[test]
    fn encodes_header_and_full_width_rows() {
        let (job, caps) = ctx_job(24.0, 1, CutMode::Every, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(128, 2);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();

        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'a', 0x01]));
        assert!(bytes.windows(3).any(|w| w == [ESC, b'i', b'z']));
        assert!(bytes.windows(3).any(|w| w == [b'G', 16, 0x00]));
        assert_eq!(*bytes.last().unwrap(), 0x1A);
    }

    #[test]
    fn resolves_12mm_printable_band() {
        let (job, caps) = ctx_job(12.0, 1, CutMode::None, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        let (_head, geom) = BrotherPtDriver::resolve_media(&ctx);
        assert_eq!(geom.tape_width_mm, 12);
        assert_eq!(geom.printable_dots, 70);
        assert_eq!(geom.offset_l, 29);
    }

    #[test]
    fn p900_uses_70_byte_rows_and_36mm_band() {
        let (job, caps) = ctx_job(36.0, 1, CutMode::Every, 360.0, 36.0);
        let ctx = EncodeContext::new(&job, &caps);
        let (head, geom) = BrotherPtDriver::resolve_media(&ctx);
        assert_eq!(head.bytes_per_row, 70);
        assert_eq!(head.head_dots, 560);
        assert_eq!(geom.tape_width_mm, 36);
        assert_eq!(geom.printable_dots, 454);
        assert_eq!(geom.offset_l, 45);

        let bmp = MonoBitmap::new(454, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(3).any(|w| w == [b'G', 70, 0x00]));
        assert!(bytes.starts_with(&[0u8; 200]));
    }

    #[test]
    fn cut_every_sets_auto_cut_and_no_chain() {
        let (job, caps) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));
    }

    #[test]
    fn cut_at_end_sets_no_chain_bit() {
        let (job, caps) = ctx_job(18.0, 2, CutMode::End, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));
        assert_eq!(bytes.iter().filter(|&&b| b == 0x0C).count(), 1);
        assert_eq!(*bytes.last().unwrap(), 0x1A);
    }
}
