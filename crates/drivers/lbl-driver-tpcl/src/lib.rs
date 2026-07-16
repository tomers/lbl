//! Toshiba TEC TPCL (TEC Printer Command Language) driver.
//!
//! Sets media with `ESC D`, clears the buffer with `ESC C`, draws a 1-bit
//! graphic via `ESC SG` hex mode, then issues with `ESC XS`. Dimensions use
//! 0.1 mm units; set bits in [`MonoBitmap`] print as black.

use lbl_core::job::CutMode;
use lbl_core::media::{MediaLength, MediaSense};
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

const ESC: u8 = 0x1b;
const LF: u8 = 0x0a;
const NUL: u8 = 0x00;
const MM_PER_INCH: f64 = 25.4;
const DEFAULT_GAP_MM: f64 = 3.0;

/// The TPCL driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct TpclDriver;

impl TpclDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn push_cmd(out: &mut Vec<u8>, body: &[u8]) {
        out.push(ESC);
        out.extend_from_slice(body);
        out.push(LF);
        out.push(NUL);
    }

    fn tenths(mm: f64) -> u32 {
        mm.mul_add(10.0, 0.5).clamp(1.0, 15_000.0) as u32
    }

    /// Pack `bitmap` for TPCL hex mode (8 dots/byte, width padded to 8).
    fn to_hex(bitmap: &MonoBitmap) -> (u32, u32, Vec<u8>) {
        let width = bitmap.width.div_ceil(8).saturating_mul(8).max(8);
        let height = bitmap.height.max(1);
        let stride = (width / 8) as usize;
        let mut data = vec![0u8; stride * height as usize];
        for y in 0..height {
            for x in 0..bitmap.width.min(width) {
                if bitmap.get(x, y) {
                    let byte = (y as usize) * stride + (x / 8) as usize;
                    data[byte] |= 0x80u8 >> (x % 8);
                }
            }
        }
        (width, height, data)
    }
}

impl Driver for TpclDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Tpcl
    }

    fn name(&self) -> &'static str {
        "tpcl-sg"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["tpcl", "toshiba", "tec"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let media = &ctx.job.media;
        let width_mm = media.width_mm;
        let length_mm = match media.length {
            MediaLength::Fixed(mm) => mm,
            MediaLength::Continuous => (bitmap.height as f64 / media.dpi.0) * MM_PER_INCH,
        };

        let (pitch_mm, sensor) = match media.sense_or_inferred() {
            MediaSense::Gap { gap_mm, .. } => {
                let gap = if gap_mm > 0.0 { gap_mm } else { DEFAULT_GAP_MM };
                (length_mm + gap, b'2')
            }
            MediaSense::BlackMark { mark_mm, .. } => (length_mm + mark_mm.max(0.0), b'1'),
            MediaSense::Continuous => (length_mm, b'0'),
        };

        let (gw, gh, graphic) = Self::to_hex(bitmap);
        if gw > 9999 || gh > 99_999 {
            return Err(DriverError::Unsupported(format!(
                "TPCL graphic too large: {gw}×{gh} dots"
            )));
        }

        let copies = ctx.copies().clamp(1, 9999);
        let cut_interval = match ctx.cut_mode() {
            CutMode::None => 0u32,
            CutMode::Every => 1,
            CutMode::End => copies.min(100),
        };

        let mut out = Vec::with_capacity(96 + graphic.len());

        let mut d = String::new();
        let _ = write!(
            d,
            "D{:04},{:04},{:04}",
            Self::tenths(pitch_mm),
            Self::tenths(width_mm),
            Self::tenths(length_mm)
        );
        Self::push_cmd(&mut out, d.as_bytes());
        Self::push_cmd(&mut out, b"C");

        // ESC SG;x,y,width,height,mode,data LF NUL — hex overwrite mode.
        let mut sg = String::new();
        let _ = write!(sg, "SG;0000,0000,{gw:04},{gh:05},1,");
        out.push(ESC);
        out.extend_from_slice(sg.as_bytes());
        out.extend_from_slice(&graphic);
        out.push(LF);
        out.push(NUL);

        // XS;I,aaaa,bbb cdefgh — cut, sensor, batch, speed 6, DT, top-first, no status.
        let mut xs = String::new();
        let _ = write!(xs, "XS;I,{copies:04},{cut_interval:03}");
        xs.push(sensor as char);
        xs.push_str("C6010");
        Self::push_cmd(&mut out, xs.as_bytes());

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

    fn find_cmd<'a>(out: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
        out.windows(prefix.len() + 1)
            .position(|w| w[0] == ESC && w[1..].starts_with(prefix))
            .map(|i| {
                let start = i;
                let end = out[start..]
                    .iter()
                    .position(|&b| b == NUL)
                    .map(|j| start + j + 1)
                    .unwrap_or(out.len());
                &out[start..end]
            })
    }

    #[test]
    fn emits_d_c_sg_xs() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = TpclDriver::new().encode(&bmp, &ctx).unwrap();

        assert!(find_cmd(&out, b"D").is_some());
        // 25 mm label + default 3 mm gap → pitch 280 tenths; width 500; length 250.
        assert!(find_cmd(&out, b"D0280,0500,0250").is_some());
        assert!(find_cmd(&out, b"C").is_some());
        assert!(find_cmd(&out, b"SG;0000,0000,0008,00001,1,").is_some());
        const SG_HDR: &[u8] = b"\x1bSG;0000,0000,0008,00001,1,";
        let sg = out.windows(SG_HDR.len()).position(|w| w == SG_HDR).unwrap();
        assert_eq!(out[sg + SG_HDR.len()], 0x80);
        assert!(find_cmd(&out, b"XS;I,0001,0002C6010").is_some());
    }

    #[test]
    fn continuous_uses_sensor_zero() {
        let bmp = MonoBitmap::new(8, 203);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::continuous(104.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = TpclDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(find_cmd(&out, b"XS;I,0001,0000C6010").is_some());
    }

    #[test]
    fn black_mark_uses_sensor_one() {
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
        let out = TpclDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(find_cmd(&out, b"XS;I,0001,0001C6010").is_some());
        assert!(find_cmd(&out, b"D1560,1020,1520").is_some());
    }

    #[test]
    fn cut_every_sets_interval_one() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(203.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = TpclDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(find_cmd(&out, b"XS;I,0001,0012C6010").is_some());
    }

    #[test]
    fn pads_width_to_eight() {
        let bmp = MonoBitmap::new(9, 2);
        let (w, h, data) = TpclDriver::to_hex(&bmp);
        assert_eq!(w, 16);
        assert_eq!(h, 2);
        assert_eq!(data.len(), 4);
    }
}
