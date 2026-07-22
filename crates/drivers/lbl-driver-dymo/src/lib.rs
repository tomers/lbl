//! DYMO drivers for `lbl`.
//!
//! This crate hosts two DYMO drivers, because DYMO uses two very different
//! protocols:
//!
//! - [`DymoDriver`] — the **LabelManager** tape protocol (this module). DYMO
//!   tape printers have a vertical print head: each transmitted "line" is one
//!   **column** of dots across the tape, and the tape feeds horizontally, so the
//!   encoder transposes the bitmap into columns. Command set follows the classic
//!   dymoprint / LabelManager USB stream: `ESC C 0`, `ESC B 0`, `ESC D n`, a
//!   `SYN`-prefixed line per column, then `ESC E` (cut, when present) and
//!   `ESC A` (status — host must read IN; reply arrives after cut/feed).
//!   Column order matches a `ROTATE_270` transpose when
//!   [`DeviceCapabilities::feed_reverse`] is set.
//! - [`LabelWriter550Driver`] — the **LabelWriter 550 series** raster protocol
//!   (see [`lw550`]), per DYMO's LW 550 Technical Reference.
//!
//! `lbl` is not affiliated with DYMO; see the repository disclaimer.

pub mod d1;
pub mod lw450;
pub mod lw550;

pub use lw450::LabelWriter450Driver;
pub use lw550::LabelWriter550Driver;

use lbl_driver_api::{ClientHandshake, Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const SYN: u8 = 0x16;

/// LabelManager protocol column height in dots for a tape width (64 for 12 mm).
fn protocol_head_dots(tape_width_mm: f64) -> u32 {
    let bytes = (8.0 * tape_width_mm / 12.0).floor() as u32;
    bytes.max(1) * 8
}

/// Dead-zone margin at each edge of the protocol column, in dots.
fn protocol_vertical_margin_dots(tape_width_mm: f64, printable_height_mm: f64) -> u32 {
    let margin_mm = ((tape_width_mm - printable_height_mm) / 2.0).max(0.0);
    if margin_mm <= f64::EPSILON {
        return 0;
    }
    let dots_per_mm = protocol_head_dots(tape_width_mm) as f64 / tape_width_mm;
    (margin_mm * dots_per_mm).round() as u32
}

/// Inkable rows inside a protocol column.
fn protocol_printable_dots(tape_width_mm: f64, printable_height_mm: f64) -> u32 {
    let protocol_h = protocol_head_dots(tape_width_mm);
    let margin = protocol_vertical_margin_dots(tape_width_mm, printable_height_mm);
    protocol_h.saturating_sub(2 * margin)
}

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

    fn resample_height_nearest(src: &MonoBitmap, target_h: u32) -> MonoBitmap {
        let mut out = MonoBitmap::new(src.width, target_h);
        if src.height == 0 || target_h == 0 {
            return out;
        }
        for y in 0..target_h {
            let src_y = (y as u64 * src.height as u64 / target_h as u64) as u32;
            let src_y = src_y.min(src.height - 1);
            for x in 0..src.width {
                if src.get(x, src_y) {
                    out.set(x, y, true);
                }
            }
        }
        out
    }

    /// Fit a render-resolution bitmap into the LabelManager protocol column.
    fn fit_to_protocol_column(
        bitmap: &MonoBitmap,
        tape_mm: f64,
        printable_height_mm: f64,
    ) -> MonoBitmap {
        let protocol_h = protocol_head_dots(tape_mm);
        if bitmap.height == protocol_h {
            return bitmap.clone();
        }
        let printable_mm = printable_height_mm.min(tape_mm);
        let v_margin = protocol_vertical_margin_dots(tape_mm, printable_mm);
        let printable_h = protocol_printable_dots(tape_mm, printable_mm).max(1);
        let scaled = if bitmap.height == printable_h {
            bitmap.clone()
        } else {
            Self::resample_height_nearest(bitmap, printable_h)
        };
        let mut out = MonoBitmap::new(bitmap.width, protocol_h);
        for y in 0..scaled.height.min(printable_h) {
            for x in 0..scaled.width {
                if scaled.get(x, y) {
                    out.set(x, y + v_margin, true);
                }
            }
        }
        out
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

        // Cut/feed first, then status. ESC A is the job terminator: its IN
        // reply is only emitted after the printer finishes preceding work, so
        // a host that drains it knows the chassis is ready for the next job.
        out.extend_from_slice(&[ESC, b'E']);
        out.extend_from_slice(&[ESC, b'A']);
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

    fn aliases(&self) -> &'static [&'static str] {
        &["dymo"]
    }

    fn handshake(&self) -> ClientHandshake {
        ClientHandshake::DymoD1
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
        let tape_mm = ctx.job.media.width_mm;
        let bitmap = if let Some(printable_mm) = ctx.capabilities.head_printable_height_mm {
            Self::fit_to_protocol_column(bitmap, tape_mm, printable_mm)
        } else {
            bitmap.clone()
        };

        let mut out = Vec::new();
        for _ in 0..ctx.copies() {
            Self::append_job(&mut out, &bitmap, lead_cols, trail_cols, feed_reverse)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::JobSpec;
    use lbl_core::media::Media;
    use lbl_core::printer::DeviceCapabilities;
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
        let caps = DeviceCapabilities::default();
        let job = ctx_job(1);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();

        // ESC C 0, ESC B 0, ESC D 1, then 3 columns each: SYN + 1 byte.
        assert_eq!(&bytes[0..3], &[ESC, b'C', 0x00]);
        assert_eq!(&bytes[3..6], &[ESC, b'B', 0x00]);
        assert_eq!(&bytes[6..9], &[ESC, b'D', 0x01]);
        assert_eq!(bytes[9], SYN);
        assert_eq!(bytes[10], 0x80); // column 0, y=0 set
                                     // ends with cut/feed, then status
        assert_eq!(&bytes[bytes.len() - 4..], &[ESC, b'E', ESC, b'A']);
    }

    #[test]
    fn copies_repeat_stream() {
        let bmp = MonoBitmap::new(2, 8);
        let caps = DeviceCapabilities::default();
        let job = ctx_job(3);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(bytes.iter().filter(|&&b| b == b'E').count(), 3);
    }

    #[test]
    fn feed_trail_adds_blank_columns() {
        let mut bmp = MonoBitmap::new(1, 8);
        bmp.set(0, 0, true);
        let caps = DeviceCapabilities {
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
        let caps = DeviceCapabilities {
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

    #[test]
    fn twelve_mm_encode_uses_protocol_column_height() {
        let mut bmp = MonoBitmap::new(1, 58);
        bmp.set(0, 29, true);
        let caps = DeviceCapabilities {
            dpi: Dpi(180.0),
            head_printable_height_mm: Some(8.2),
            ..Default::default()
        };
        let job = ctx_job(1);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = DymoDriver::new().encode(&bmp, &ctx).unwrap();
        assert_eq!(&bytes[6..9], &[ESC, b'D', 0x08]);
    }

    #[test]
    fn twelve_mm_protocol_geometry() {
        assert_eq!(protocol_head_dots(12.0), 64);
        assert_eq!(protocol_vertical_margin_dots(12.0, 8.2), 10);
        assert_eq!(protocol_printable_dots(12.0, 8.2), 44);
        assert_eq!(protocol_head_dots(6.0), 32);
        assert_eq!(protocol_vertical_margin_dots(6.0, 8.2), 0);
    }
}
