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
//! N × 0x00                invalidate (200 default; override via caps)
//! ESC @                   initialize
//! ESC i a 0x01            switch to raster mode
//! per page / copy:
//!   ESC i z …             print information (TZe width + raster line count)
//!   ESC i M …             various mode (auto-cut)
//!   ESC i A …             cut every N (when auto-cut)
//!   ESC i K …             advanced mode (half-cut / no-chain / high-res)
//!   ESC i d …             margin / feed amount
//!   M 0x00 | M 0x02       compression off or TIFF PackBits
//!   per row:
//!     G n_lo n_hi <n B>   raster graphics transfer (opcode 0x47 + u16 LE)
//!     or Z                blank row (PackBits mode only)
//!   0x1A / 0x0C           print with feed (last) / print (more pages follow)
//! ```
//!
//! On the wire PT jobs use opcode `0x47` (ASCII `'G'`) with a little-endian
//! u16 payload length — matching nbuchwitz/ptouch and verified on PT-P710BT.
//! Brother's P900 PDF lists hex `47` next to ASCII `G`; the P700 manual lists
//! `g`/`67`. Cube-class firmware accepts `0x47` and stalls on `0x67` with u16
//! framing. Do not confuse this with QL's `g 00 <u8 length>` row shape.
//! Under `M 0x02`, row payloads must be TIFF PackBits (or `Z`); never raw head
//! bytes. High-resolution mode (capability-gated) duplicates each raster line
//! and doubles the feed margin; laminated high-res jobs use print-info media
//! type `0x09`.
//!
//! Bit polarity matches [`MonoBitmap`]: `1` = ink. Each row is mirrored
//! left-to-right before packing, matching Brother's own driver wiring.
//!
//! Durable protocol notes (RE sources, failure modes, multi-page framing,
//! head-to-cutter leader): `docs/src/reference/brother-pt-raster.md`.
//!
//! `lbl` is not affiliated with Brother; see the repository disclaimer.

use lbl_core::job::CutMode;
use lbl_driver_api::{
    is_blank_row, packbits_encode, Driver, DriverError, EncodeContext, MonoBitmap, Protocol,
};

const ESC: u8 = 0x1B;
/// Print-info media type for laminated TZe on P700 / Cube-class (`ESC i z` n2).
///
/// The PT-E550W / P750W / P710BT raster reference uses `0x01` = laminated and
/// `0x00` = no media. (P900-class manuals reuse `0x00` for lam/non-lam — see
/// [`MEDIA_LAM_NONLAM_P900`].)
const MEDIA_LAMINATED: u8 = 0x01;
/// Print-info media type for laminated / non-laminated TZe on P900-class.
const MEDIA_LAM_NONLAM_P900: u8 = 0x00;
/// Print-info media type required for high-res laminated tape (P900-class).
const MEDIA_HIGH_RES_LAM: u8 = 0x09;
/// Default lead/trail feed (~1–2 mm depending on dpi), from Brother samples.
const DEFAULT_FEED_MARGIN: u16 = 14;
/// Raster graphics transfer opcode (ASCII `'G'` / 0x47) with u16 LE length.
///
/// Verified on PT-P710BT: `0x67` + u16 LE stalls bulk OUT; `0x47` + u16 LE
/// prints. Same byte as nbuchwitz/ptouch.
const RASTER_OPCODE: u8 = 0x47;

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
    invalidate_bytes: 200,
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

    fn emit_raster_row(out: &mut Vec<u8>, row: &[u8], packbits: bool) {
        if packbits {
            if is_blank_row(row) {
                out.push(b'Z');
                return;
            }
            // Under M 02 the payload must be PackBits, even when not shorter than raw.
            let payload = packbits_encode(row);
            let n = payload.len() as u16;
            out.push(RASTER_OPCODE);
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(&payload);
            return;
        }
        let n = row.len() as u16;
        out.push(RASTER_OPCODE);
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(row);
    }

    /// `ESC i z` page-index byte (`n9`).
    ///
    /// P900-class manuals use `0` / `1` / `2` (first / middle / last-or-single).
    /// The PT-E550W / P750W / P710BT (and H500 / P700 / E500) Raster Command
    /// Reference only documents `0` = starting page and `1` = other pages —
    /// including the last page and a single-page job. Sending `2` on those
    /// chassis is undefined; on a PT-P750W a two-label batch with `n9=2` on
    /// the final page latched `status_type=error` with error-info-1 bit 3
    /// ("weak batteries") until power-cycle.
    fn page_index(first: bool, last: bool, wide_head: bool) -> u8 {
        if wide_head {
            match (first, last) {
                (true, true) => 2,  // single-page job
                (true, false) => 0, // first of multi
                (false, true) => 2, // last of multi
                (false, false) => 1,
            }
        } else if first {
            0
        } else {
            1
        }
    }

    /// Convert millimeters to feed-margin dots at the device DPI.
    fn mm_to_feed_dots(mm: f64, dpi: f64) -> u16 {
        if !(mm.is_finite() && mm > 0.0 && dpi.is_finite() && dpi > 0.0) {
            return 0;
        }

        (mm * dpi / 25.4).round().clamp(0.0, u16::MAX as f64) as u16
    }

    /// `ESC i d` margin from [`EncodeContext::feed_plan`], falling back to geometry default.
    fn feed_margin_dots(ctx: &EncodeContext<'_>, geom: MediaGeometry) -> u16 {
        let dpi = ctx.capabilities.dpi.0;
        let from_plan = Self::mm_to_feed_dots(ctx.feed_plan.lead_mm, dpi);
        if from_plan > 0 {
            return from_plan;
        }
        if let Some(min) = ctx.capabilities.feed_lead_min_mm {
            let from_min = Self::mm_to_feed_dots(min, dpi);
            if from_min > 0 {
                return from_min;
            }
        }
        geom.feed_margin
    }

    /// Trailing blank feed rows from [`FeedPlan::end_mm`] (base DPI; hi-res doubles in emit).
    ///
    /// When a cut will fire, [`FeedPlan::end_mm`] includes the head-to-cutter
    /// clearance \(D_x\) (floored in `resolve_feed_plan`). Print-with-feed
    /// (`0x1A`) already advances that clearance to the cutter, so only emit
    /// raster blank for the surplus above \(D_x\) — otherwise the kept sticker
    /// would show ~\(2 D_x\) after last ink.
    fn end_blank_rows(ctx: &EncodeContext<'_>) -> u32 {
        let end_mm = ctx.feed_plan.end_mm;
        // Prefer the resolved plan gap; fall back to caps if a caller built a
        // plan without cutter_gap_mm populated but still passed feed_trail.
        let dx = if ctx.feed_plan.cutter_gap_mm > 0.0 {
            ctx.feed_plan.cutter_gap_mm
        } else {
            ctx.capabilities
                .feed_trail_mm
                .filter(|d| d.is_finite() && *d > 0.0)
                .unwrap_or(0.0)
        };
        let will_cut = ctx.capabilities.supports_cut && ctx.cut_mode().requests_cut();
        let encode_mm = if will_cut && dx > 0.0 {
            (end_mm - dx).max(0.0)
        } else {
            end_mm
        };
        Self::mm_to_feed_dots(encode_mm, ctx.capabilities.dpi.0) as u32
    }

    /// Full pre-cut prologue: zero raster + auto-cut + no-chain + `0x1A`.
    ///
    /// Half pre-cut does **not** use a separate page — on P750W a half+`0x0C`
    /// prologue produced two peel scores with only one half bit in the job
    /// (device capture 2026-07-30). Half is handled on the content page via
    /// auto-cut + half-cut (see encode). Only when [`FeedPlan::precut`] and
    /// full `precut_cut_kind`.
    fn push_full_precut_page(
        out: &mut Vec<u8>,
        head: HeadProfile,
        geom: MediaGeometry,
        feed_margin: u16,
        packbits: bool,
    ) {
        let media_type = if head.bytes_per_row > 16 {
            MEDIA_LAM_NONLAM_P900
        } else {
            MEDIA_LAMINATED
        };
        let wide_head = head.bytes_per_row > 16;
        // PI_RECOVER | PI_KIND | PI_WIDTH
        let flags: u8 = 0x80 | 0x02 | 0x04;
        out.extend_from_slice(&[ESC, b'i', b'z', flags]);
        out.push(media_type);
        out.push(geom.tape_width_mm);
        out.push(0x00);
        out.extend_from_slice(&0u32.to_le_bytes());
        out.push(Self::page_index(true, true, wide_head));
        out.push(0x00);
        out.extend_from_slice(&[ESC, b'i', b'M', 1 << 6]); // auto-cut
        out.extend_from_slice(&[ESC, b'i', b'A', 0x01]);
        out.extend_from_slice(&[ESC, b'i', b'K', 1 << 3]); // no-chain
        out.extend_from_slice(&[ESC, b'i', b'd']);
        out.extend_from_slice(&feed_margin.to_le_bytes());
        out.extend_from_slice(&[b'M', if packbits { 0x02 } else { 0x00 }]);
        out.push(0x1A);
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
            half_cut,
            no_chain,
            quality,
            high_res,
            packbits,
            first_page,
            last_page,
            feed_margin,
            end_blank_rows,
        } = opts;

        let media_type = if high_res {
            MEDIA_HIGH_RES_LAM
        } else if head.bytes_per_row > 16 {
            MEDIA_LAM_NONLAM_P900
        } else {
            MEDIA_LAMINATED
        };
        let feed = if high_res {
            feed_margin.saturating_mul(2)
        } else {
            feed_margin
        };
        let content_lines = if high_res {
            bitmap.height.saturating_mul(2)
        } else {
            bitmap.height
        };
        let end_lines = if high_res {
            end_blank_rows.saturating_mul(2)
        } else {
            end_blank_rows
        };
        let raster_lines = content_lines.saturating_add(end_lines);

        // PI_RECOVER | PI_KIND | PI_WIDTH [| PI_QUALITY]
        let mut flags: u8 = 0x80 | 0x02 | 0x04;
        if quality {
            flags |= 0x40;
        }
        out.extend_from_slice(&[ESC, b'i', b'z', flags]);
        out.push(media_type);
        out.push(geom.tape_width_mm);
        out.push(0x00); // continuous TZe — length unused
        out.extend_from_slice(&raster_lines.to_le_bytes());
        out.push(Self::page_index(
            first_page,
            last_page,
            head.bytes_per_row > 16,
        ));
        out.push(0x00);

        // ESC i M bit 6 = auto cut. ESC i A = cut each N when auto-cut.
        out.extend_from_slice(&[ESC, b'i', b'M', if auto_cut { 1 << 6 } else { 0 }]);
        if auto_cut {
            out.extend_from_slice(&[ESC, b'i', b'A', 0x01]);
        }
        // ESC i K: bit2 half-cut, bit3 no-chain (last page only), bit6 high-res.
        // Setting no-chain on every page of a multi-label batch makes Cube-class
        // devices feed/cut the head-to-cutter gap as an empty leader before each
        // label when each page is sent as its own job.
        let mut advanced: u8 = 0;
        if half_cut {
            advanced |= 1 << 2;
        }
        if no_chain {
            advanced |= 1 << 3;
        }
        if high_res {
            advanced |= 1 << 6;
        }
        out.extend_from_slice(&[ESC, b'i', b'K', advanced]);
        out.extend_from_slice(&[ESC, b'i', b'd']);
        out.extend_from_slice(&feed.to_le_bytes());
        out.extend_from_slice(&[b'M', if packbits { 0x02 } else { 0x00 }]);

        for y in 0..bitmap.height {
            let row = Self::pack_mirrored_row(bitmap, y, head);
            Self::emit_raster_row(out, &row, packbits);
            if high_res {
                Self::emit_raster_row(out, &row, packbits);
            }
        }
        let blank = vec![0u8; head.bytes_per_row];
        for _ in 0..end_blank_rows {
            Self::emit_raster_row(out, &blank, packbits);
            if high_res {
                Self::emit_raster_row(out, &blank, packbits);
            }
        }

        out.push(if last_page { 0x1A } else { 0x0C });
    }
}

#[derive(Debug, Clone, Copy)]
struct PageEncodeOpts {
    auto_cut: bool,
    /// ESC i K bit 2 — laminate-only cut when the chassis supports half-cut.
    half_cut: bool,
    /// ESC i K bit 3 — only on the true last page of the job (unless chain).
    no_chain: bool,
    quality: bool,
    high_res: bool,
    packbits: bool,
    first_page: bool,
    last_page: bool,
    /// `ESC i d` feed margin in base-DPI dots.
    feed_margin: u16,
    /// Blank raster rows after content (base DPI; doubled when hi-res).
    end_blank_rows: u32,
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
        let high_res = ctx.capabilities.supports_high_resolution;
        let packbits = ctx.capabilities.supports_packbits;
        let feed_margin = Self::feed_margin_dots(ctx, geom);
        let end_blank_rows = Self::end_blank_rows(ctx);
        let invalidate_bytes = ctx
            .capabilities
            .invalidate_bytes
            .map(|n| n as usize)
            .unwrap_or(head.invalidate_bytes);
        // CutMode::Every: auto-cut (+ cut-each) on every page; no-chain only on
        // the last page so multi-label batches do not eject a leader scrap per
        // label. CutMode::End: no auto-cut; no-chain on the last page only.
        // chain_print suppresses no-chain so the last label stays on the roll.
        //
        // Half pre-cut: do not emit a separate half+0x0C page. On P750W that
        // shape produced two peel scores (~Dx then ~lead) with only one half
        // bit in the job. Instead: one content page with auto-cut + half-cut
        // (Brother Editor “trim first part of left margin” / half leading tab);
        // final 0x1A still full-cuts.
        let half_precut = ctx.feed_plan.precut && ctx.feed_plan.precut_cut_kind.is_half();
        let full_precut = ctx.feed_plan.precut && !ctx.feed_plan.precut_cut_kind.is_half();
        let half_cut = ctx.cut_kind().is_half() || half_precut;
        let (auto_cut, want_no_chain) = match ctx.cut_mode() {
            CutMode::None => (false, false),
            CutMode::Every => {
                if half_precut {
                    (true, !ctx.chain_print())
                } else {
                    (!half_cut, !ctx.chain_print())
                }
            }
            CutMode::End => {
                if half_precut {
                    (true, !ctx.chain_print())
                } else {
                    (false, !ctx.chain_print())
                }
            }
        };

        let mut out = Vec::with_capacity(
            invalidate_bytes
                + 64
                + copies as usize
                    * (bitmap.data.len() + bitmap.height as usize * (3 + head.bytes_per_row) + 48),
        );
        if ctx.batch_first() {
            out.extend(std::iter::repeat_n(0u8, invalidate_bytes));
            out.extend_from_slice(&[ESC, b'@']);
            out.extend_from_slice(&[ESC, b'i', b'a', 0x01]);
            if full_precut {
                Self::push_full_precut_page(&mut out, head, geom, feed_margin, packbits);
            }
        }

        for index in 0..copies {
            let first_page = ctx.batch_first() && index == 0 && !full_precut;
            let last_page = ctx.batch_last() && index + 1 == copies;
            Self::push_page(
                &mut out,
                &bitmap,
                head,
                geom,
                PageEncodeOpts {
                    auto_cut,
                    half_cut,
                    no_chain: want_no_chain && last_page,
                    quality: self.quality_priority,
                    high_res,
                    packbits,
                    first_page,
                    last_page,
                    feed_margin,
                    end_blank_rows,
                },
            );
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::{CutKind, CutMode, JobSpec};
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
        assert!(bytes.windows(3).any(|w| w == [RASTER_OPCODE, 16, 0x00]));
        assert_eq!(*bytes.last().unwrap(), 0x1A);
        // P700-class single-page page index is 0 (starting page).
        let z = bytes
            .windows(3)
            .position(|w| w == [ESC, b'i', b'z'])
            .unwrap();
        assert_eq!(bytes[z + 4], MEDIA_LAMINATED);
        assert_eq!(bytes[z + 11], 0);
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
        assert!(bytes.windows(3).any(|w| w == [RASTER_OPCODE, 70, 0x00]));
        assert!(bytes.starts_with(&[0u8; 200]));
    }

    #[test]
    fn cut_every_sets_auto_cut_no_chain_and_cut_each() {
        let (job, caps) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));
    }

    #[test]
    fn half_cut_sets_advanced_bit() {
        let (mut job, mut caps) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        job.cut_kind = CutKind::Half;
        caps.supports_half_cut = true;
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        // Half (bit2) + no-chain (bit3). Auto-cut must be off — it overrides half-cut.
        assert!(bytes
            .windows(4)
            .any(|w| w == [ESC, b'i', b'K', (1 << 2) | (1 << 3)]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 0]));
        assert!(!bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
    }

    #[test]
    fn half_cut_clamped_without_cap() {
        let (mut job, caps) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        job.cut_kind = CutKind::Half;
        let ctx = EncodeContext::new(&job, &caps);
        assert_eq!(ctx.cut_kind(), CutKind::Full);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));
        assert!(!bytes
            .windows(4)
            .any(|w| w[0..3] == [ESC, b'i', b'K'] && w[3] & (1 << 2) != 0));
    }

    #[test]
    fn chain_print_suppresses_no_chain() {
        let (mut job, caps) = ctx_job(18.0, 2, CutMode::Every, 180.0, 24.0);
        job.chain_print = true;
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        let k_flags: Vec<u8> = bytes
            .windows(4)
            .filter(|w| w[..3] == [ESC, b'i', b'K'])
            .map(|w| w[3])
            .collect();
        assert!(k_flags.iter().all(|&f| f & (1 << 3) == 0));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
    }

    #[test]
    fn every_half_chain_sets_half_without_no_chain() {
        let (mut job, mut caps) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        job.cut_kind = CutKind::Half;
        job.chain_print = true;
        caps.supports_half_cut = true;
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 2]));
        assert!(!bytes
            .windows(4)
            .any(|w| w[0..3] == [ESC, b'i', b'K'] && w[3] & (1 << 3) != 0));
    }

    #[test]
    fn cut_at_end_sets_no_chain_bit() {
        let (job, caps) = ctx_job(18.0, 2, CutMode::End, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        // No-chain only on the last copy.
        let k_flags: Vec<u8> = bytes
            .windows(4)
            .filter(|w| w[..3] == [ESC, b'i', b'K'])
            .map(|w| w[3])
            .collect();
        assert_eq!(k_flags, vec![0, 1 << 3]);
        assert_eq!(bytes.iter().filter(|&&b| b == 0x0C).count(), 1);
        assert_eq!(*bytes.last().unwrap(), 0x1A);
        let z_positions: Vec<_> = bytes
            .windows(3)
            .enumerate()
            .filter(|(_, w)| *w == [ESC, b'i', b'z'])
            .map(|(i, _)| i)
            .collect();
        assert_eq!(bytes[z_positions[0] + 11], 0); // first / starting page
        assert_eq!(bytes[z_positions[1] + 11], 1); // other pages (incl. last)
    }

    #[test]
    fn batch_continuation_skips_prologue_and_defers_no_chain() {
        let (mut job0, caps) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        job0.batch_index = 0;
        job0.batch_total = 2;
        let (mut job1, _) = ctx_job(18.0, 1, CutMode::Every, 180.0, 24.0);
        job1.batch_index = 1;
        job1.batch_total = 2;
        let bmp = MonoBitmap::new(8, 1);
        let first = BrotherPtDriver::new()
            .encode(&bmp, &EncodeContext::new(&job0, &caps))
            .unwrap();
        let second = BrotherPtDriver::new()
            .encode(&bmp, &EncodeContext::new(&job1, &caps))
            .unwrap();

        assert!(first.starts_with(&[0u8; 200]));
        assert!(first.windows(4).any(|w| w == [ESC, b'@', ESC, b'i']));
        assert_eq!(*first.last().unwrap(), 0x0C);
        assert!(first.windows(4).any(|w| w == [ESC, b'i', b'K', 0]));
        assert!(!first.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));

        assert!(!second.starts_with(&[0u8; 8]));
        assert!(!second.windows(4).any(|w| w == [ESC, b'@', ESC, b'i']));
        assert_eq!(*second.last().unwrap(), 0x1A);
        assert!(second.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));

        let mut combined = first.clone();
        combined.extend_from_slice(&second);
        assert_eq!(combined.iter().filter(|&&b| b == 0x1A).count(), 1);
        assert_eq!(combined.iter().filter(|&&b| b == 0x0C).count(), 1);

        let z0 = first
            .windows(3)
            .position(|w| w == [ESC, b'i', b'z'])
            .unwrap();
        let z1 = second
            .windows(3)
            .position(|w| w == [ESC, b'i', b'z'])
            .unwrap();
        assert_eq!(first[z0 + 11], 0); // starting page
        assert_eq!(second[z1 + 11], 1); // other / last page on 128-pin
    }

    #[test]
    fn p900_page_index_uses_last_or_single_value_two() {
        let (job, caps) = ctx_job(36.0, 2, CutMode::End, 360.0, 36.0);
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        let z_positions: Vec<_> = bytes
            .windows(3)
            .enumerate()
            .filter(|(_, w)| *w == [ESC, b'i', b'z'])
            .map(|(i, _)| i)
            .collect();
        assert_eq!(bytes[z_positions[0] + 11], 0);
        assert_eq!(bytes[z_positions[1] + 11], 2);
    }

    #[test]
    fn high_res_duplicates_rows_and_sets_flags() {
        let (job, mut caps) = ctx_job(36.0, 1, CutMode::None, 360.0, 36.0);
        caps.supports_high_resolution = true;
        let ctx = EncodeContext::new(&job, &caps);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 6]));
        let z = bytes
            .windows(3)
            .position(|w| w == [ESC, b'i', b'z'])
            .unwrap();
        assert_eq!(bytes[z + 4], MEDIA_HIGH_RES_LAM);
        assert_eq!(&bytes[z + 7..z + 11], &2u32.to_le_bytes()); // duplicated lines
        assert_eq!(
            bytes
                .windows(3)
                .filter(|w| *w == [RASTER_OPCODE, 70, 0x00])
                .count(),
            2
        );
    }

    #[test]
    fn packbits_mode_payloads_decode_to_head_width() {
        let (job, mut caps) = ctx_job(24.0, 1, CutMode::Every, 180.0, 24.0);
        caps.supports_packbits = true;
        let ctx = EncodeContext::new(&job, &caps);
        // High-entropy ink so PackBits often does not shrink — must still be PackBits,
        // not raw head bytes (raw under M 02 desyncs the Cube into a blank job).
        let mut bmp = MonoBitmap::new(128, 8);
        for y in 0..8 {
            for x in 0..128 {
                if (x * 3 + y * 7) % 5 < 2 {
                    bmp.set(x, y, true);
                }
            }
        }
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(2).any(|w| w == [b'M', 0x02]));

        fn decode_packbits(payload: &[u8]) -> Option<Vec<u8>> {
            let mut out = Vec::new();
            let mut i = 0usize;
            while i < payload.len() {
                let n = payload[i] as i8;
                i += 1;
                if n >= 0 {
                    let cnt = n as usize + 1;
                    if i + cnt > payload.len() {
                        return None;
                    }
                    out.extend_from_slice(&payload[i..i + cnt]);
                    i += cnt;
                } else if n != -128 {
                    let cnt = (1i16 - i16::from(n)) as usize;
                    if i >= payload.len() {
                        return None;
                    }
                    out.extend(std::iter::repeat_n(payload[i], cnt));
                    i += 1;
                }
            }
            Some(out)
        }

        let mut i = bytes.windows(2).position(|w| w == [b'M', 0x02]).unwrap() + 2;
        let mut g_rows = 0usize;
        while i < bytes.len() && bytes[i] != 0x1A && bytes[i] != 0x0C {
            if bytes[i] == b'Z' {
                i += 1;
                continue;
            }
            assert_eq!(
                bytes[i], RASTER_OPCODE,
                "unexpected byte {:#x} at {i}",
                bytes[i]
            );
            let n = u16::from_le_bytes([bytes[i + 1], bytes[i + 2]]) as usize;
            let payload = &bytes[i + 3..i + 3 + n];
            let decoded = decode_packbits(payload).expect("PackBits payload under M 02");
            assert_eq!(decoded.len(), 16);
            g_rows += 1;
            i += 3 + n;
        }
        assert!(g_rows > 0);
    }

    #[test]
    fn precut_emits_zero_raster_page_then_content() {
        let (mut job, mut caps) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        job.feed_lead_mm = Some(2.0);
        job.precut = Some(true);
        // Explicit full so this fixture stays independent of half-cut defaults.
        job.precut_cut_kind = Some(CutKind::Full);
        caps.supports_precut = true;
        caps.feed_trail_mm = Some(24.0);
        let plan = lbl_core::resolve_feed_plan(&caps, &job).unwrap();
        assert!(plan.precut);
        assert_eq!(plan.precut_cut_kind, CutKind::Full);
        let ctx = EncodeContext::with_feed_plan(&job, &caps, plan);
        let bmp = MonoBitmap::new(8, 2);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();

        // Two print-info blocks: precut (0 lines) then content.
        let z_positions: Vec<_> = bytes
            .windows(3)
            .enumerate()
            .filter(|(_, w)| *w == [ESC, b'i', b'z'])
            .map(|(i, _)| i)
            .collect();
        assert_eq!(z_positions.len(), 2);
        let z0 = z_positions[0];
        // raster_lines LE u32 at z+7.. — zero for precut
        assert_eq!(&bytes[z0 + 7..z0 + 11], &[0, 0, 0, 0]);
        assert_eq!(bytes.iter().filter(|&&b| b == 0x1A).count(), 2);

        // Full precut: auto-cut + no-chain.
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'K', 1 << 3]));

        // ESC i d follows lead ≈ 2 mm at 180 dpi → 14 dots
        let expected_feed = BrotherPtDriver::mm_to_feed_dots(2.0, 180.0);
        assert!(bytes.windows(5).any(|w| {
            w[0..3] == [ESC, b'i', b'd'] && u16::from_le_bytes([w[3], w[4]]) == expected_feed
        }));
    }

    #[test]
    fn precut_half_uses_content_auto_and_half_no_prologue() {
        let (mut job, mut caps) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        job.feed_lead_mm = Some(2.0);
        job.precut = Some(true);
        caps.supports_precut = true;
        caps.supports_half_cut = true;
        caps.feed_trail_mm = Some(24.0);
        let plan = lbl_core::resolve_feed_plan(&caps, &job).unwrap();
        assert_eq!(plan.precut_cut_kind, CutKind::Half);
        let ctx = EncodeContext::with_feed_plan(&job, &caps, plan);
        let bmp = MonoBitmap::new(8, 1);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();

        // No separate prologue — one print-info block only.
        let z_count = bytes.windows(3).filter(|w| *w == [ESC, b'i', b'z']).count();
        assert_eq!(z_count, 1);
        // Auto-cut + half-cut + no-chain on the content page; job ends 0x1A.
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'M', 1 << 6]));
        assert!(bytes.windows(4).any(|w| w == [ESC, b'i', b'A', 0x01]));
        assert!(bytes
            .windows(4)
            .any(|w| w == [ESC, b'i', b'K', (1 << 2) | (1 << 3)]));
        assert_eq!(*bytes.last().unwrap(), 0x1A);
        let expected_feed = BrotherPtDriver::mm_to_feed_dots(2.0, 180.0);
        assert!(bytes.windows(5).any(|w| {
            w[0..3] == [ESC, b'i', b'd'] && u16::from_le_bytes([w[3], w[4]]) == expected_feed
        }));
    }

    #[test]
    fn precut_only_on_batch_first() {
        let (mut job0, mut caps) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        job0.feed_lead_mm = Some(2.0);
        job0.precut = Some(true);
        job0.batch_index = 0;
        job0.batch_total = 2;
        caps.supports_precut = true;
        caps.feed_trail_mm = Some(24.0);
        let plan = lbl_core::resolve_feed_plan(&caps, &job0).unwrap();

        let (mut job1, _) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        job1.feed_lead_mm = Some(2.0);
        job1.precut = Some(true);
        job1.batch_index = 1;
        job1.batch_total = 2;

        let bmp = MonoBitmap::new(8, 1);
        let first = BrotherPtDriver::new()
            .encode(&bmp, &EncodeContext::with_feed_plan(&job0, &caps, plan))
            .unwrap();
        let second = BrotherPtDriver::new()
            .encode(&bmp, &EncodeContext::with_feed_plan(&job1, &caps, plan))
            .unwrap();

        // Precut page only on first segment (two ESC i z: precut + content).
        assert_eq!(
            first.windows(3).filter(|w| *w == [ESC, b'i', b'z']).count(),
            2
        );
        assert_eq!(
            second
                .windows(3)
                .filter(|w| *w == [ESC, b'i', b'z'])
                .count(),
            1
        );
    }

    #[test]
    fn without_precut_flag_no_zero_line_page() {
        let (job, caps) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        let ctx = EncodeContext::new(&job, &caps);
        assert!(!ctx.feed_plan.precut);
        let bmp = MonoBitmap::new(8, 2);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(
            bytes.windows(3).filter(|w| *w == [ESC, b'i', b'z']).count(),
            1
        );
        assert_eq!(bytes.iter().filter(|&&b| b == 0x1A).count(), 1);
    }

    #[test]
    fn cut_end_clearance_not_double_emitted_as_blank_rows() {
        // FeedPlan.end_mm is floored to Dx for preview; 0x1A supplies that
        // clearance — only surplus above Dx becomes trailing blank rasters.
        let (mut job, mut caps) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        job.feed_lead_mm = Some(24.0);
        job.feed_end_mm = Some(0.0);
        caps.feed_trail_mm = Some(24.0);
        let plan = lbl_core::resolve_feed_plan(&caps, &job).unwrap();
        assert!((plan.end_mm - 24.0).abs() < 1e-9);
        let ctx = EncodeContext::with_feed_plan(&job, &caps, plan);
        let bmp = MonoBitmap::new(8, 2);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        let z = bytes
            .windows(3)
            .position(|w| *w == [ESC, b'i', b'z'])
            .unwrap();
        let raster_lines =
            u32::from_le_bytes([bytes[z + 7], bytes[z + 8], bytes[z + 9], bytes[z + 10]]);
        assert_eq!(raster_lines, 2, "Dx end clearance must not add blank rows");

        job.feed_end_mm = Some(30.0);
        let plan_hi = lbl_core::resolve_feed_plan(&caps, &job).unwrap();
        assert!((plan_hi.end_mm - 30.0).abs() < 1e-9);
        let ctx_hi = EncodeContext::with_feed_plan(&job, &caps, plan_hi);
        let bytes_hi = BrotherPtDriver::new().encode(&bmp, &ctx_hi).unwrap();
        let z_hi = bytes_hi
            .windows(3)
            .position(|w| *w == [ESC, b'i', b'z'])
            .unwrap();
        let raster_hi = u32::from_le_bytes([
            bytes_hi[z_hi + 7],
            bytes_hi[z_hi + 8],
            bytes_hi[z_hi + 9],
            bytes_hi[z_hi + 10],
        ]);
        // Surplus 6 mm @ 180 dpi ≈ 42 dots
        let surplus = ((6.0_f64 * 180.0) / 25.4).round() as u32;
        assert_eq!(raster_hi, 2 + surplus);
    }

    #[test]
    fn end_pad_below_dx_does_not_stack_on_firmware_clearance() {
        // Horizontal/axes padding sets G_end=20 while Dx=24: policy floors end
        // to Dx for preview, but 0x1A already advances Dx — emit no blank rows.
        let (mut job, mut caps) = ctx_job(12.0, 1, CutMode::Every, 180.0, 24.0);
        job.feed_lead_mm = Some(20.0);
        job.feed_end_mm = Some(20.0);
        job.precut = Some(true);
        caps.feed_trail_mm = Some(24.0);
        caps.supports_precut = true;
        let plan = lbl_core::resolve_feed_plan(&caps, &job).unwrap();
        assert!((plan.end_mm - 24.0).abs() < 1e-9);
        let ctx = EncodeContext::with_feed_plan(&job, &caps, plan);
        let bmp = MonoBitmap::new(8, 2);
        let bytes = BrotherPtDriver::new().encode(&bmp, &ctx).unwrap();
        let z = bytes
            .windows(3)
            .position(|w| *w == [ESC, b'i', b'z'])
            .unwrap();
        // Skip precut page (zero raster lines); find the content page.
        let content_z = bytes[z + 1..]
            .windows(3)
            .position(|w| *w == [ESC, b'i', b'z'])
            .map(|i| z + 1 + i)
            .unwrap_or(z);
        let raster_lines = u32::from_le_bytes([
            bytes[content_z + 7],
            bytes[content_z + 8],
            bytes[content_z + 9],
            bytes[content_z + 10],
        ]);
        assert_eq!(
            raster_lines, 2,
            "G_end ≤ Dx must not add blank rows on top of 0x1A clearance"
        );
    }
}
