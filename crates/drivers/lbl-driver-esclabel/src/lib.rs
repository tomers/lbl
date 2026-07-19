//! Epson ESC/Label (ColorWorks) driver.
//!
//! ESC/Label is Epson's ZPL II–compatible label language with media-layout
//! extensions (`^S(CLS,…)`). Monochrome jobs emit a `^GFA` graphic field
//! (set bits are printed, matching [`MonoBitmap`]). Full-color jobs register
//! a PNG via `~DY` and recall it with `^IM`.

use lbl_core::media::{MediaLength, MediaSense};
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

/// Object name used for volatile PNG registration (`~DY` / `^IM` / `^ID`).
const COLOR_OBJECT: &str = "LBL";

/// The ESC/Label driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct EscLabelDriver;

impl EscLabelDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn media_preamble(ctx: &EncodeContext, bitmap_height: u32, cut: bool) -> String {
        let media = &ctx.job.media;
        let width_dots = media.width_dots().0.max(1);
        let length_dots = match media.length {
            MediaLength::Fixed(_) => media
                .length_dots()
                .map(|d| d.0)
                .unwrap_or(bitmap_height)
                .max(1),
            MediaLength::Continuous => bitmap_height.max(1),
        };

        let mut out = String::new();
        out.push_str("^XA\n");
        // Epson media layout (dots). See ESC/Label ^S(CLS,b,c).
        let _ = writeln!(out, "^S(CLS,P,{width_dots}");
        let _ = writeln!(out, "^S(CLS,L,{length_dots}");
        match media.sense_or_inferred() {
            MediaSense::Gap { gap_mm, .. } => {
                let gap_dots = lbl_core::units::Millimeters(gap_mm)
                    .to_dots(media.dpi)
                    .0
                    .max(1);
                let _ = writeln!(out, "^S(CLS,C,{gap_dots}");
                out.push_str("^MNY\n");
            }
            MediaSense::BlackMark { .. } => out.push_str("^MNM\n"),
            MediaSense::Continuous => out.push_str("^MNN\n"),
        }
        if cut {
            out.push_str("^MMC\n");
        } else {
            out.push_str("^MMT\n");
        }
        out
    }

    fn label_mono(
        bitmap: &MonoBitmap,
        ctx: &EncodeContext,
        cut: bool,
    ) -> Result<String, DriverError> {
        let stride = bitmap.stride();
        let total = bitmap.data.len();
        if total == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let mut hex = String::with_capacity(total * 2);
        for byte in &bitmap.data {
            write!(hex, "{byte:02X}").expect("write to string");
        }

        let mut out = Self::media_preamble(ctx, bitmap.height, cut);
        out.push_str("^FO0,0\n");
        let _ = writeln!(out, "^GFA,{total},{total},{stride},{hex}");
        out.push_str("^FS\n");
        out.push_str("^XZ\n");
        Ok(out)
    }

    fn label_color(ctx: &EncodeContext, height_dots: u32, cut: bool) -> String {
        let mut out = Self::media_preamble(ctx, height_dots, cut);
        out.push_str("^FO0,0\n");
        let _ = writeln!(out, "^IMR:{COLOR_OBJECT}.PNG");
        out.push_str("^FS\n");
        out.push_str("^XZ\n");
        out
    }

    /// Register `png` in volatile memory, print it once per copy, then delete it.
    fn encode_color(
        &self,
        png: &[u8],
        height_dots: u32,
        ctx: &EncodeContext,
    ) -> Result<Vec<u8>, DriverError> {
        if png.is_empty() {
            return Err(DriverError::Unsupported("empty color PNG".into()));
        }

        let copies = ctx.copies();
        let mut out = Vec::with_capacity(png.len() + 256);

        // Clear leftover objects, then register the color graphic once.
        out.extend_from_slice(b"^XA\n^IDR:*.*\n^FS\n^XZ\n");
        let header = format!("~DYR:{COLOR_OBJECT},B,P,{},0,", png.len());
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(png);

        for index in 0..copies {
            let cut = ctx.should_cut_after_copy(index, copies);
            out.extend_from_slice(Self::label_color(ctx, height_dots, cut).as_bytes());
        }

        out.extend_from_slice(b"^XA\n^IDR:");
        out.extend_from_slice(COLOR_OBJECT.as_bytes());
        out.extend_from_slice(b".PNG\n^FS\n^XZ\n");
        Ok(out)
    }
}

impl Driver for EscLabelDriver {
    fn protocol(&self) -> Protocol {
        Protocol::EscLabel
    }

    fn name(&self) -> &'static str {
        "esclabel"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["esc-label", "esc/label", "esclabel", "colorworks"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if let Some(png) = ctx.color_png.filter(|_| ctx.capabilities.supports_color) {
            let height = if bitmap.height > 0 {
                bitmap.height
            } else {
                ctx.job.media.length_dots().map(|d| d.0).unwrap_or(1).max(1)
            };
            return self.encode_color(png, height, ctx);
        }

        let copies = ctx.copies();
        let mut out = String::new();
        for index in 0..copies {
            let cut = ctx.should_cut_after_copy(index, copies);
            out.push_str(&Self::label_mono(bitmap, ctx, cut)?);
        }
        Ok(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::{CutMode, JobSpec};
    use lbl_core::media::Media;
    use lbl_core::printer::DeviceCapabilities;
    use lbl_core::units::Dpi;

    /// Minimal valid 1×1 red PNG (RGB).
    fn tiny_png() -> Vec<u8> {
        vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0xC9, 0xFE, 0x92,
            0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    #[test]
    fn emits_media_layout_and_gfa() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true); // 0x80
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(600.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = String::from_utf8(EscLabelDriver::new().encode(&bmp, &ctx).unwrap()).unwrap();
        assert!(out.starts_with("^XA"));
        assert!(out.contains("^S(CLS,P,1181")); // 50mm @ 600 dpi
        assert!(out.contains("^S(CLS,L,591")); // 25mm @ 600 dpi
        assert!(out.contains("^S(CLS,C,71")); // 3mm gap @ 600 dpi
        assert!(out.contains("^MNY"));
        assert!(out.contains("^MMT"));
        assert!(out.contains("^GFA,1,1,1,80"));
        assert!(out.trim_end().ends_with("^XZ"));
    }

    #[test]
    fn continuous_emits_mnn_without_gap_layout() {
        let bmp = MonoBitmap::new(8, 2);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::continuous(108.0, Dpi(600.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = String::from_utf8(EscLabelDriver::new().encode(&bmp, &ctx).unwrap()).unwrap();
        assert!(out.contains("^MNN"));
        assert!(!out.contains("^MNY"));
        assert!(!out.contains("^S(CLS,C,"));
        assert!(out.contains("^S(CLS,L,2")); // bitmap height
    }

    #[test]
    fn cut_emits_mmc() {
        let bmp = {
            let mut b = MonoBitmap::new(8, 1);
            b.set(1, 0, true);
            b
        };
        let caps = DeviceCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(600.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = String::from_utf8(EscLabelDriver::new().encode(&bmp, &ctx).unwrap()).unwrap();
        assert!(out.contains("^MMC"));
        assert!(!out.contains("^MMT"));
    }

    #[test]
    fn color_emits_dy_im_and_skips_gfa() {
        let bmp = MonoBitmap::new(8, 1);
        let png = tiny_png();
        let caps = DeviceCapabilities {
            supports_color: true,
            ..Default::default()
        };
        let job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(600.0)));
        let ctx = EncodeContext::new(&job, &caps).with_color_png(&png);
        let bytes = EscLabelDriver::new().encode(&bmp, &ctx).unwrap();

        let header = format!("~DYR:{COLOR_OBJECT},B,P,{},0,", png.len());
        let header_pos = bytes
            .windows(header.len())
            .position(|w| w == header.as_bytes())
            .expect("~DY header");
        let png_start = header_pos + header.len();
        assert_eq!(&bytes[png_start..png_start + png.len()], png.as_slice());

        let ascii = String::from_utf8_lossy(&bytes);
        assert!(ascii.contains("^IDR:*.*"));
        assert!(ascii.contains(&format!("^IMR:{COLOR_OBJECT}.PNG")));
        assert!(ascii.contains(&format!("^IDR:{COLOR_OBJECT}.PNG")));
        assert!(ascii.contains("^S(CLS,P,"));
        assert!(!ascii.contains("^GFA,"));
    }

    #[test]
    fn color_png_ignored_without_supports_color() {
        let bmp = {
            let mut b = MonoBitmap::new(8, 1);
            b.set(0, 0, true);
            b
        };
        let png = tiny_png();
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::fixed(50.0, 25.0, Dpi(600.0)));
        let ctx = EncodeContext::new(&job, &caps).with_color_png(&png);
        let out = String::from_utf8(EscLabelDriver::new().encode(&bmp, &ctx).unwrap()).unwrap();
        assert!(out.contains("^GFA,"));
        assert!(!out.contains("~DY"));
    }
}
