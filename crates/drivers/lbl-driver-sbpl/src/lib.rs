//! SATO SBPL (SATO Barcode Printer Language) driver.
//!
//! Emits `ESC A`…`ESC Z` with `A1` media size, optional `~A` cut interval, and
//! a custom graphic via `ESC G` as 8×8 binary blocks. Set bits print as black,
//! matching [`MonoBitmap`].

use lbl_core::job::CutMode;
use lbl_core::media::MediaLength;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

const ESC: u8 = 0x1b;

/// The SBPL driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct SbplDriver;

impl SbplDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn push_cmd(out: &mut Vec<u8>, body: &[u8]) {
        out.push(ESC);
        out.extend_from_slice(body);
    }

    /// Pack `bitmap` into SBPL 8×8 block order (left→right, top→bottom).
    ///
    /// Each block is 8 bytes (one byte per row). Width and height are padded
    /// up to the next multiple of 8 dots.
    fn to_blocks(bitmap: &MonoBitmap) -> (u32, u32, Vec<u8>) {
        let h_blocks = bitmap.width.div_ceil(8).max(1);
        let v_blocks = bitmap.height.div_ceil(8).max(1);
        let mut data = Vec::with_capacity((h_blocks * v_blocks * 8) as usize);
        for vb in 0..v_blocks {
            for hb in 0..h_blocks {
                for row in 0..8u32 {
                    let y = vb * 8 + row;
                    let mut byte = 0u8;
                    for bit in 0..8u32 {
                        let x = hb * 8 + bit;
                        if bitmap.get(x, y) {
                            byte |= 0x80u8 >> bit;
                        }
                    }
                    data.push(byte);
                }
            }
        }
        (h_blocks, v_blocks, data)
    }

    fn clamp_u4(v: u32) -> u32 {
        v.min(9999)
    }
}

impl Driver for SbplDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Sbpl
    }

    fn name(&self) -> &'static str {
        "sbpl-g"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["sbpl", "sato"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let media = &ctx.job.media;
        let width_dots = Self::clamp_u4(bitmap.width.max(1));
        let height_dots = Self::clamp_u4(match media.length {
            MediaLength::Fixed(_) => media
                .length_dots()
                .map(|d| d.0)
                .unwrap_or(bitmap.height)
                .max(1),
            MediaLength::Continuous => bitmap.height.max(1),
        });

        let (h_blocks, v_blocks, graphic) = Self::to_blocks(bitmap);
        if h_blocks > 999 || v_blocks > 999 {
            return Err(DriverError::Unsupported(format!(
                "SBPL graphic too large: {h_blocks}×{v_blocks} blocks (max 999)"
            )));
        }

        let copies = ctx.copies().clamp(1, 999_999);
        let mut out = Vec::with_capacity(64 + graphic.len());

        Self::push_cmd(&mut out, b"A");
        let mut a1 = String::new();
        let _ = write!(a1, "A1{height_dots:04}{width_dots:04}");
        Self::push_cmd(&mut out, a1.as_bytes());
        Self::push_cmd(&mut out, b"H0001");
        Self::push_cmd(&mut out, b"V0001");

        match ctx.cut_mode() {
            CutMode::None => {}
            CutMode::Every => Self::push_cmd(&mut out, b"~A0001"),
            CutMode::End => {
                let n = copies.min(9999);
                let mut cut = String::new();
                let _ = write!(cut, "~A{n:04}");
                Self::push_cmd(&mut out, cut.as_bytes());
            }
        }

        let mut g = String::new();
        let _ = write!(g, "GB{h_blocks:03}{v_blocks:03}");
        Self::push_cmd(&mut out, g.as_bytes());
        out.extend_from_slice(&graphic);

        let mut q = String::new();
        let _ = write!(q, "Q{copies:06}");
        Self::push_cmd(&mut out, q.as_bytes());
        Self::push_cmd(&mut out, b"Z");
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

    fn find_cmd<'a>(out: &'a [u8], cmd: &[u8]) -> Option<&'a [u8]> {
        out.windows(1 + cmd.len())
            .position(|w| w[0] == ESC && &w[1..] == cmd)
            .map(|i| &out[i..i + 1 + cmd.len()])
    }

    #[test]
    fn emits_a_a1_g_q_z() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = SbplDriver::new().encode(&bmp, &ctx).unwrap();

        assert_eq!(out[0], ESC);
        assert_eq!(out[1], b'A');
        // A1 + height 0432 (54mm@203) + width 0008 (bitmap).
        assert!(find_cmd(&out, b"A104320008").is_some());
        assert!(find_cmd(&out, b"H0001").is_some());
        assert!(find_cmd(&out, b"V0001").is_some());
        assert!(find_cmd(&out, b"GB001001").is_some());
        assert!(find_cmd(&out, b"Q000001").is_some());
        assert!(find_cmd(&out, b"Z").is_some());
        // First graphic data byte follows ESC + "GB001001".
        let g = out.windows(9).position(|w| w == b"\x1bGB001001").unwrap();
        assert_eq!(out[g + 9], 0x80);
    }

    #[test]
    fn cut_every_emits_tilde_a() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = SbplDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(find_cmd(&out, b"~A0001").is_some());
    }

    #[test]
    fn pads_to_eight_by_eight_blocks() {
        let bmp = MonoBitmap::new(9, 9);
        let (h, v, data) = SbplDriver::to_blocks(&bmp);
        assert_eq!(h, 2);
        assert_eq!(v, 2);
        assert_eq!(data.len(), 2 * 2 * 8);
    }
}
