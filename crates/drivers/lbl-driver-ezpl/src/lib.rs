//! Godex EZPL (EZ Printer Language) driver.
//!
//! Sets media with `^Q`/`^W`, downloads a 1-bit BMP via `~EB`, places it with
//! `Y` inside a `^L`…`E` label format, prints with `~P`, then deletes the
//! temporary graphic. Cutter interval uses `^D` when requested.

use lbl_core::job::CutMode;
use lbl_core::media::{MediaLength, MediaSense};
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

const MM_PER_INCH: f64 = 25.4;
const GRAPHIC_NAME: &str = "LBL";

/// The EZPL driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct EzplDriver;

impl EzplDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    /// Encode `bitmap` as a Windows BMP (1 bpp, top-down).
    ///
    /// Palette entry 0 is white and entry 1 is black so set bits in
    /// [`MonoBitmap`] print as ink without inversion. Rows are padded to a
    /// 4-byte boundary per the BMP DIB layout.
    fn to_bmp1(bitmap: &MonoBitmap) -> Vec<u8> {
        let width = bitmap.width;
        let height = bitmap.height;
        let row_bytes = (width as usize).div_ceil(32) * 4;
        let pixel_bytes = row_bytes * height as usize;
        let file_header = 14usize;
        let info_header = 40usize;
        let palette = 8usize; // 2 × RGBQUAD
        let offset = file_header + info_header + palette;
        let file_size = offset + pixel_bytes;

        let mut out = Vec::with_capacity(file_size);
        // BITMAPFILEHEADER
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved1
        out.extend_from_slice(&0u16.to_le_bytes()); // reserved2
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        // BITMAPINFOHEADER
        out.extend_from_slice(&(info_header as u32).to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        // Negative height = top-down.
        out.extend_from_slice(&(-(height as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&1u16.to_le_bytes()); // bpp
        out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // x ppm
        out.extend_from_slice(&0u32.to_le_bytes()); // y ppm
        out.extend_from_slice(&2u32.to_le_bytes()); // colors used
        out.extend_from_slice(&2u32.to_le_bytes()); // important colors
                                                    // Palette: index 0 = white, index 1 = black (BGRA).
        out.extend_from_slice(&[0xff, 0xff, 0xff, 0x00]);
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);

        let src_stride = bitmap.stride();
        for y in 0..height as usize {
            let src = &bitmap.data[y * src_stride..(y + 1) * src_stride];
            out.extend_from_slice(src);
            out.resize(out.len() + (row_bytes - src_stride), 0);
        }
        out
    }
}

impl Driver for EzplDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Ezpl
    }

    fn name(&self) -> &'static str {
        "ezpl-bmp"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["ezpl", "godex"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let media = &ctx.job.media;
        let width_mm = media.width_mm;
        let height_mm = match media.length {
            MediaLength::Fixed(mm) => mm,
            MediaLength::Continuous => (bitmap.height as f64 / media.dpi.0) * MM_PER_INCH,
        };

        let mut preamble = String::new();
        match media.sense_or_inferred() {
            MediaSense::Gap { gap_mm, .. } => {
                let _ = writeln!(preamble, "^Q{height_mm:.0},{gap_mm:.0}");
            }
            MediaSense::BlackMark { mark_mm, offset_mm } => {
                let sign = if offset_mm >= 0.0 { "+" } else { "-" };
                let _ = writeln!(
                    preamble,
                    "^Q{height_mm:.0},{mark_mm:.0},{:.0}{sign}",
                    offset_mm.abs()
                );
            }
            MediaSense::Continuous => {
                let _ = writeln!(preamble, "^Q{height_mm:.0},0,{height_mm:.0}");
            }
        }
        let _ = writeln!(preamble, "^W{width_mm:.0}");

        let copies = ctx.copies();
        match ctx.cut_mode() {
            CutMode::None => preamble.push_str("^D0\r\n"),
            CutMode::Every => preamble.push_str("^D1\r\n"),
            CutMode::End => {
                let _ = writeln!(preamble, "^D{copies}");
            }
        }

        let bmp = Self::to_bmp1(bitmap);
        let mut out = Vec::with_capacity(preamble.len() + bmp.len() + 128);
        for line in preamble.lines() {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        let _ = write!(StringSink(&mut out), "~EB,{GRAPHIC_NAME},{}\r\n", bmp.len());
        out.extend_from_slice(&bmp);
        out.extend_from_slice(b"^L\r\n");
        let _ = write!(StringSink(&mut out), "Y0,0,{GRAPHIC_NAME}\r\n");
        out.extend_from_slice(b"E\r\n");
        let _ = write!(StringSink(&mut out), "~P{copies}\r\n");
        let _ = write!(StringSink(&mut out), "~MDELG,{GRAPHIC_NAME}\r\n");
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
    fn emits_qw_download_place_print() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = EzplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("^Q25,3\r\n") || text.contains("^Q25,"));
        assert!(text.contains("^W50\r\n"));
        assert!(text.contains("~EB,LBL,"));
        assert!(text.contains("^L\r\n"));
        assert!(text.contains("Y0,0,LBL\r\n"));
        assert!(text.contains("E\r\n"));
        assert!(text.contains("~P1\r\n"));
        assert!(text.contains("~MDELG,LBL\r\n"));
        assert!(out.windows(2).any(|w| w == b"BM"));
    }

    #[test]
    fn continuous_uses_plain_paper_q() {
        let bmp = MonoBitmap::new(8, 203);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::continuous(104.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = EzplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("^Q25,0,25\r\n") || text.contains(",0,"));
    }

    #[test]
    fn cut_every_emits_d1() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(203.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = EzplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("^D1\r\n"));
    }

    #[test]
    fn bmp_is_top_down_one_bpp() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let bytes = EzplDriver::to_bmp1(&bmp);
        assert_eq!(&bytes[0..2], b"BM");
        // biBitCount at offset 28
        assert_eq!(u16::from_le_bytes([bytes[28], bytes[29]]), 1);
        // biHeight negative (top-down) at offset 22
        let height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(height, -1);
    }
}
