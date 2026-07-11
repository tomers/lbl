//! Epson ESC/Label (ColorWorks) driver.
//!
//! ESC/Label is Epson's ZPL II–compatible label language with media-layout
//! extensions (`^S(CLS,…)`). This driver emits a monochrome `^GFA` graphic
//! field inside `^XA … ^XZ`, matching the [`MonoBitmap`] bit convention
//! (set bits are printed).

use lbl_core::media::{MediaLength, MediaSense};
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

/// The ESC/Label driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct EscLabelDriver;

impl EscLabelDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn label(bitmap: &MonoBitmap, ctx: &EncodeContext, cut: bool) -> Result<String, DriverError> {
        let media = &ctx.job.media;
        let stride = bitmap.stride();
        let total = bitmap.data.len();
        if total == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let width_dots = media.width_dots().0.max(1);
        let length_dots = match media.length {
            MediaLength::Fixed(_) => media
                .length_dots()
                .map(|d| d.0)
                .unwrap_or(bitmap.height)
                .max(1),
            MediaLength::Continuous => bitmap.height.max(1),
        };

        let mut hex = String::with_capacity(total * 2);
        for byte in &bitmap.data {
            write!(hex, "{byte:02X}").expect("write to string");
        }

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
        out.push_str("^FO0,0\n");
        let _ = writeln!(out, "^GFA,{total},{total},{stride},{hex}");
        out.push_str("^FS\n");
        out.push_str("^XZ\n");
        Ok(out)
    }
}

impl Driver for EscLabelDriver {
    fn protocol(&self) -> Protocol {
        Protocol::EscLabel
    }

    fn name(&self) -> &'static str {
        "esclabel-gfa"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let copies = ctx.copies();
        let mut out = String::new();
        for index in 0..copies {
            let cut = ctx.should_cut_after_copy(index, copies);
            out.push_str(&Self::label(bitmap, ctx, cut)?);
        }
        Ok(out.into_bytes())
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
    fn emits_media_layout_and_gfa() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true); // 0x80
        let caps = PrinterCapabilities::default();
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
        let caps = PrinterCapabilities::default();
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
        let caps = PrinterCapabilities {
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
}
