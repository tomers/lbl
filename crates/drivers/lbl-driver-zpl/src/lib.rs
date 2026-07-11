//! ZPL (Zebra Programming Language) driver.
//!
//! Emits a label using the `^GFA` (Graphic Field, ASCII-hex) command. ZPL
//! `^GF` treats set bits as black dots, matching the [`MonoBitmap`] layout. The
//! label is wrapped in `^XA ... ^XZ`; a requested cut switches the printer into
//! cut mode with `^MMC`.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};
use std::fmt::Write;

/// The ZPL driver.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZplDriver;

impl ZplDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn label(bitmap: &MonoBitmap, cut: bool) -> String {
        let stride = bitmap.stride();
        let total = bitmap.data.len();

        let mut hex = String::with_capacity(total * 2);
        for byte in &bitmap.data {
            write!(hex, "{byte:02X}").expect("write to string");
        }

        let mut zpl = String::new();
        zpl.push_str("^XA\n");
        if cut {
            zpl.push_str("^MMC\n");
        }
        zpl.push_str("^FO0,0\n");
        // ^GFA,bytesTotal,bytesTotal,bytesPerRow,data
        let _ = writeln!(zpl, "^GFA,{total},{total},{stride},{hex}");
        zpl.push_str("^FS\n");
        zpl.push_str("^XZ\n");
        zpl
    }
}

impl Driver for ZplDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Zpl
    }

    fn name(&self) -> &'static str {
        "zpl-gfa"
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        let copies = ctx.copies();
        let mut out = String::new();
        for index in 0..copies {
            let cut = ctx.should_cut_after_copy(index, copies);
            out.push_str(&Self::label(bitmap, cut));
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
    fn wraps_in_xa_xz_with_gfa() {
        let mut bmp = MonoBitmap::new(8, 1);
        bmp.set(0, 0, true); // 0x80
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(300.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let out = String::from_utf8(ZplDriver::new().encode(&bmp, &ctx).unwrap()).unwrap();
        assert!(out.starts_with("^XA"));
        assert!(out.contains("^GFA,1,1,1,80"));
        assert!(out.trim_end().ends_with("^XZ"));
        assert!(!out.contains("^MMC"));
    }

    #[test]
    fn cut_adds_mmc() {
        let bmp = {
            let mut b = MonoBitmap::new(8, 1);
            b.set(1, 0, true);
            b
        };
        let caps = PrinterCapabilities {
            supports_cut: true,
            ..Default::default()
        };
        let mut job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(300.0)));
        job.cut_mode = CutMode::Every;
        let ctx = EncodeContext::new(&job, &caps);
        let out = String::from_utf8(ZplDriver::new().encode(&bmp, &ctx).unwrap()).unwrap();
        assert!(out.contains("^MMC"));
    }
}
