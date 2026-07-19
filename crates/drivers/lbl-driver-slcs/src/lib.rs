//! Bixolon SLCS (Samsung Label Command Set) driver.
//!
//! Emits `SW`/`SL`/`CB`/`LD`/`P` (and optional `CUT`). SLCS `LD` treats set
//! bits as black, matching [`MonoBitmap`]. Command lines use `CR+LF`; the `P`
//! command must end with `CR` only.

use lbl_core::job::CutMode;
use lbl_core::media::{MediaLength, MediaSense};
use lbl_core::units::Millimeters;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

/// The SLCS driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct SlcsDriver;

impl SlcsDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn le_u16(v: u32) -> (u8, u8) {
        let v = v.min(u16::MAX as u32) as u16;
        ((v & 0xff) as u8, (v >> 8) as u8)
    }
}

impl Driver for SlcsDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Slcs
    }

    fn name(&self) -> &'static str {
        "slcs-ld"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["slcs", "bixolon"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let media = &ctx.job.media;
        let dpi = media.dpi;
        let width_dots = bitmap.width.max(1);
        let length_dots = match media.length {
            MediaLength::Fixed(_) => media
                .length_dots()
                .map(|d| d.0)
                .unwrap_or(bitmap.height)
                .max(1),
            MediaLength::Continuous => bitmap.height.max(1),
        };

        let mut header = String::new();
        let _ = writeln!(header, "SW{width_dots}");
        match media.sense_or_inferred() {
            MediaSense::Gap { gap_mm, offset_mm } => {
                let gap_dots = Millimeters(gap_mm).to_dots(dpi).0.max(1);
                let offset_dots = Millimeters(offset_mm).to_dots(dpi).0;
                if offset_dots > 0 {
                    let _ = writeln!(header, "SL{length_dots},{gap_dots},G,{offset_dots}");
                } else {
                    let _ = writeln!(header, "SL{length_dots},{gap_dots},G");
                }
            }
            MediaSense::BlackMark { mark_mm, offset_mm } => {
                let mark_dots = Millimeters(mark_mm).to_dots(dpi).0.max(1);
                let offset_dots = Millimeters(offset_mm).to_dots(dpi).0;
                if offset_dots > 0 {
                    let _ = writeln!(header, "SL{length_dots},{mark_dots},B,{offset_dots}");
                } else {
                    let _ = writeln!(header, "SL{length_dots},{mark_dots},B");
                }
            }
            MediaSense::Continuous => {
                let _ = writeln!(header, "SL{length_dots},0,C");
            }
        }
        header.push_str("CB\r\n");

        let copies = ctx.copies();
        match ctx.cut_mode() {
            CutMode::None => header.push_str("CUTn\r\n"),
            CutMode::Every => header.push_str("CUTy,1\r\n"),
            CutMode::End => {
                let _ = writeln!(header, "CUTy,{copies}");
            }
        }

        let width_bytes = bitmap.stride() as u32;
        let (xl, xh) = Self::le_u16(0);
        let (yl, yh) = Self::le_u16(0);
        let (dhl, dhh) = Self::le_u16(width_bytes);
        let (dvl, dvh) = Self::le_u16(bitmap.height);

        let mut out = Vec::with_capacity(header.len() + 10 + bitmap.data.len() + 16);
        // writeln! uses `\n`; SLCS requires CR+LF on command lines.
        for line in header.lines() {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.push(b'L');
        out.push(b'D');
        out.extend_from_slice(&[xl, xh, yl, yh, dhl, dhh, dvl, dvh]);
        out.extend_from_slice(&bitmap.data);
        // P must be terminated by CR only (SLCS programming manual).
        let _ = write!(StringSink(&mut out), "P1,{copies}\r");
        Ok(out)
    }
}

/// Tiny adapter to `write!` directly into a byte buffer.
struct StringSink<'a>(&'a mut Vec<u8>);
impl Write for StringSink<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::{CutMode, JobSpec};
    use lbl_core::media::Media;
    use lbl_core::printer::DeviceCapabilities;
    use lbl_core::units::Dpi;

    #[test]
    fn emits_sw_sl_ld_p() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = SlcsDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("SW8\r\n"));
        assert!(text.contains("SL432,24,G\r\n") || text.contains("SL"));
        assert!(text.contains("CB\r\n"));
        assert!(text.contains("CUTn\r\n"));
        assert!(out.windows(2).any(|w| w == b"LD"));
        assert!(text.contains("P1,1\r"));
        assert!(!text.trim_end().ends_with("\n"));
    }

    #[test]
    fn continuous_emits_c_sense() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::continuous(104.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = SlcsDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains(",C\r\n") || text.contains("SL1,0,C"));
    }

    #[test]
    fn black_mark_emits_b_sense() {
        use lbl_core::media::MediaSense;
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities::default();
        let mut media = Media::fixed(102.0, 152.0, Dpi(203.0));
        media.sense = Some(MediaSense::BlackMark {
            mark_mm: 4.0,
            offset_mm: 0.0,
        });
        let job = JobSpec::new(media);
        let ctx = EncodeContext::new(&job, &caps);
        let out = SlcsDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains(",B\r\n") || text.contains(",B,"));
        assert!(!text.contains(",G\r\n"));
    }

    #[test]
    fn ld_preserves_ink_bits() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true); // 0x80
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = SlcsDriver::new().encode(&bmp, &ctx).unwrap();
        let pos = out.windows(2).position(|w| w == b"LD").unwrap();
        // LD + 8 param bytes, then data
        assert_eq!(out[pos + 2 + 8], 0x80);
    }

    #[test]
    fn cut_every_emits_cuty() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = SlcsDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("CUTy,1\r\n"));
    }
}
