//! Honeywell / Datamax-O'Neil / Citizen DPL (Datamax Programming Language) driver.
//!
//! Disables immediate commands with `SOH D`, selects media sense, downloads a
//! 1-bit BMP via `STX I`, places it with an image record inside `STX L`…`E`,
//! then deletes the temporary graphic. Optional cut uses label-format `:`.
//! Citizen CL-S / CL-E native Datamax mode uses the same command set.
use lbl_core::job::CutMode;
use lbl_core::media::{MediaLength, MediaSense};
use lbl_core::units::Millimeters;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const GRAPHIC_NAME: &str = "LBL";
const MODULE: u8 = b'D';
const MM_PER_INCH: f64 = 25.4;

/// The DPL driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct DplDriver;

impl DplDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn push_soh(out: &mut Vec<u8>, body: &[u8]) {
        out.push(SOH);
        out.extend_from_slice(body);
    }

    fn push_stx_line(out: &mut Vec<u8>, body: &[u8]) {
        out.push(STX);
        out.extend_from_slice(body);
        out.push(b'\r');
    }

    fn push_line(out: &mut Vec<u8>, body: &[u8]) {
        out.extend_from_slice(body);
        out.push(b'\r');
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
        let palette = 8usize;
        let offset = file_header + info_header + palette;
        let file_size = offset + pixel_bytes;

        let mut out = Vec::with_capacity(file_size);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(file_size as u32).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        out.extend_from_slice(&(info_header as u32).to_le_bytes());
        out.extend_from_slice(&(width as i32).to_le_bytes());
        out.extend_from_slice(&(-(height as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
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

impl Driver for DplDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Dpl
    }

    fn name(&self) -> &'static str {
        "dpl-bmp"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["dpl", "honeywell", "datamax", "citizen"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let media = &ctx.job.media;
        let copies = ctx.copies().clamp(1, 99_999);
        let bmp = Self::to_bmp1(bitmap);
        let mut out = Vec::with_capacity(128 + bmp.len());

        // Binary BMP download can contain SOH/STX/ESC; disable immediate cmds.
        Self::push_soh(&mut out, b"D");

        // Metric units so continuous length is mm/10.
        Self::push_stx_line(&mut out, b"m");

        match media.sense_or_inferred() {
            MediaSense::Gap { .. } => Self::push_stx_line(&mut out, b"e"),
            MediaSense::BlackMark { .. } => Self::push_stx_line(&mut out, b"r"),
            MediaSense::Continuous => {
                let length_mm = match media.length {
                    MediaLength::Fixed(mm) => mm,
                    MediaLength::Continuous => (bitmap.height as f64 / media.dpi.0) * MM_PER_INCH,
                };
                // cnnnn in metric mode: length in 0.1 mm units.
                let tenths = Millimeters(length_mm).0.mul_add(10.0, 0.5) as u32;
                let tenths = tenths.clamp(1, 9999);
                let mut c = String::new();
                let _ = write!(c, "c{tenths:04}");
                Self::push_stx_line(&mut out, c.as_bytes());
            }
        }

        // Module D, binary BMP (lowercase b = not flipped), name LBL.
        let mut download = Vec::with_capacity(8 + GRAPHIC_NAME.len());
        download.push(b'I');
        download.push(MODULE);
        download.push(b'b');
        download.extend_from_slice(GRAPHIC_NAME.as_bytes());
        Self::push_stx_line(&mut out, &download);
        out.extend_from_slice(&bmp);

        Self::push_stx_line(&mut out, b"L");
        match ctx.cut_mode() {
            CutMode::None => {}
            CutMode::Every => Self::push_line(&mut out, b":0001"),
            CutMode::End => {
                let n = copies.min(9999);
                let mut cut = String::new();
                let _ = write!(cut, ":{n:04}");
                Self::push_line(&mut out, cut.as_bytes());
            }
        }
        // Rotation 1, image field, 1×1 multipliers, row/col 0, name LBL.
        let mut place = String::new();
        let _ = write!(place, "1Y11000000000000{GRAPHIC_NAME}");
        Self::push_line(&mut out, place.as_bytes());
        let mut q = String::new();
        let _ = write!(q, "Q{copies}");
        Self::push_line(&mut out, q.as_bytes());
        Self::push_line(&mut out, b"E");

        // Delete image: module D, type G (graphic), name LBL.
        let mut del = Vec::with_capacity(8 + GRAPHIC_NAME.len());
        del.push(b'x');
        del.push(MODULE);
        del.push(b'G');
        del.extend_from_slice(GRAPHIC_NAME.as_bytes());
        Self::push_stx_line(&mut out, &del);

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

    #[test]
    fn emits_download_place_print_delete() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = DplDriver::new().encode(&bmp, &ctx).unwrap();

        assert_eq!(out[0], SOH);
        assert_eq!(out[1], b'D');
        assert!(out.windows(3).any(|w| w == [STX, b'm', b'\r']));
        assert!(out.windows(3).any(|w| w == [STX, b'e', b'\r']));
        assert!(out.windows(7).any(|w| w == b"\x02IDbLBL"));
        assert!(out.windows(2).any(|w| w == b"BM"));
        assert!(out.windows(3).any(|w| w == [STX, b'L', b'\r']));
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("1Y11000000000000LBL\r"));
        assert!(text.contains("Q1\r"));
        assert!(text.contains("E\r"));
        assert!(out.windows(7).any(|w| w == b"\x02xDGLBL"));
    }

    #[test]
    fn continuous_emits_c_length() {
        let bmp = MonoBitmap::new(8, 203);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::continuous(104.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = DplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        // 203 dots @ 203 dpi ≈ 25.4 mm → 254 tenths.
        assert!(text.contains("c0254\r") || text.contains("\x02c0254\r"));
    }

    #[test]
    fn black_mark_emits_r() {
        use lbl_core::media::MediaSense;
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let mut media = Media::fixed(102.0, 152.0, Dpi(203.0));
        media.sense = Some(MediaSense::BlackMark {
            mark_mm: 4.0,
            offset_mm: 0.0,
        });
        let job = JobSpec::new(media);
        let ctx = EncodeContext::new(&job, &caps);
        let out = DplDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(out.windows(3).any(|w| w == [STX, b'r', b'\r']));
        assert!(!out.windows(3).any(|w| w == [STX, b'e', b'\r']));
    }

    #[test]
    fn cut_every_emits_colon() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(203.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = DplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains(":0001\r"));
    }
}
