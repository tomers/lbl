//! TSPL (TSC Printer Language) driver.
//!
//! Emits `SIZE`/`GAP`/`CLS`/`BITMAP`/`PRINT` commands. Note that TSPL's
//! `BITMAP` uses the **opposite** ink convention to [`MonoBitmap`]: a `1` bit
//! means *not printed* (white). This driver therefore inverts the bytes.

use lbl_core::job::CutMode;
use lbl_core::media::{MediaLength, MediaSense};
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

const MM_PER_INCH: f64 = 25.4;

/// The TSPL driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct TsplDriver;

impl TsplDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }
}

impl Driver for TsplDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Tspl
    }

    fn name(&self) -> &'static str {
        "tspl-bitmap"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["tspl"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        let media = &ctx.job.media;
        let dpi = media.dpi.0;
        let width_mm = media.width_mm;
        let height_mm = match media.length {
            MediaLength::Fixed(mm) => mm,
            MediaLength::Continuous => (bitmap.height as f64 / dpi) * MM_PER_INCH,
        };

        let width_bytes = bitmap.stride();
        // TSPL: 1 = white, 0 = black -> invert our (1 = ink) bytes.
        let inverted: Vec<u8> = bitmap.data.iter().map(|b| !b).collect();
        let copies = ctx.copies();
        // SET CUTTER n cuts every n labels; End mode cuts once after the batch.
        let cutter_n = match ctx.cut_mode() {
            CutMode::None => None,
            CutMode::Every => Some(1u32),
            CutMode::End => Some(copies),
        };

        let mut header = String::new();
        let _ = write!(header, "SIZE {width_mm:.0} mm, {height_mm:.0} mm\r\n");
        match media.sense_or_inferred() {
            MediaSense::Gap { gap_mm, offset_mm } => {
                let _ = write!(header, "GAP {gap_mm:.0} mm, {offset_mm:.0} mm\r\n");
            }
            MediaSense::BlackMark { mark_mm, offset_mm } => {
                let _ = write!(header, "BLINE {mark_mm:.0} mm, {offset_mm:.0} mm\r\n");
            }
            MediaSense::Continuous => header.push_str("GAP 0 mm, 0 mm\r\n"),
        }
        header.push_str("DIRECTION 1\r\n");
        if let Some(n) = cutter_n {
            let _ = write!(header, "SET CUTTER {n}\r\n");
        }
        header.push_str("CLS\r\n");
        let _ = write!(header, "BITMAP 0,0,{width_bytes},{},0,", bitmap.height);

        let mut out = Vec::new();
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&inverted);
        out.extend_from_slice(b"\r\n");
        let _ = write!(
            // PRINT sets,copies
            StringSink(&mut out),
            "PRINT 1,{}\r\n",
            ctx.copies()
        );
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
    use lbl_core::job::JobSpec;
    use lbl_core::media::Media;
    use lbl_core::printer::PrinterCapabilities;
    use lbl_core::units::Dpi;

    #[test]
    fn emits_size_bitmap_print() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = TsplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("SIZE 25 mm, 54 mm"));
        assert!(text.contains("GAP 3 mm, 0 mm"));
        assert!(text.contains("BITMAP 0,0,1,1,0,"));
        assert!(text.contains("PRINT 1,1"));
    }

    #[test]
    fn continuous_emits_zero_gap() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::continuous(104.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = TsplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("GAP 0 mm, 0 mm"));
    }

    #[test]
    fn black_mark_emits_bline() {
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
        let out = TsplDriver::new().encode(&bmp, &ctx).unwrap();
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("BLINE 4 mm, 0 mm"));
        assert!(!text.contains("GAP "));
    }

    #[test]
    fn inverts_ink_bits() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true); // our byte: 0x80
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = TsplDriver::new().encode(&bmp, &ctx).unwrap();
        // The data byte after the BITMAP header should be inverted: !0x80 = 0x7F.
        let marker = b"0,0,1,1,0,";
        let pos = out.windows(marker.len()).position(|w| w == marker).unwrap();
        assert_eq!(out[pos + marker.len()], 0x7F);
    }
}
