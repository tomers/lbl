//! Phomemo D30 / Q30 mini label-maker driver.
//!
//! These narrow-tape devices share a vendor bring-up sequence plus a
//! `1F 11 24 00` / `ESC @` / `GS v 0` raster job (no M02 or M110 footer).
//! Reverse-engineered from Android "Print Master" captures
//! ([tuxBurner/phomemo_d30](https://github.com/tuxBurner/phomemo_d30)).
//!
//! `lbl` is not affiliated with Phomemo.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const US: u8 = 0x1F;

/// BLE bring-up packets sniffed before each D30 job.
const D30_BRINGUP: &[&[u8]] = &[
    &[US, 0x11, 0x38],
    &[US, 0x11, 0x12, US, 0x11, 0x13],
    &[US, 0x11, 0x09],
    &[US, 0x11, 0x11],
    &[US, 0x11, 0x19],
    &[US, 0x11, 0x07],
    &[US, 0x11, 0x0a, US, 0x11, 0x02, 0x02],
];

/// Per-job marker immediately before `ESC @` / `GS v 0`.
const D30_JOB_MARK: &[u8] = &[US, 0x11, 0x24, 0x00];

/// Driver for Phomemo D30 / Q30 (and close relatives such as D35 / Q30S).
#[derive(Debug, Default, Clone, Copy)]
pub struct PhomemoD30Driver;

impl PhomemoD30Driver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }
}

impl Driver for PhomemoD30Driver {
    fn protocol(&self) -> Protocol {
        Protocol::PhomemoD30
    }

    fn name(&self) -> &'static str {
        "phomemo-d30"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["phomemo-d30", "phomemod30", "d30", "q30"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let stride = bitmap.stride();
        if stride > 0xFFFF {
            return Err(DriverError::Unsupported("raster too wide".into()));
        }
        if bitmap.height > 0xFFFF {
            return Err(DriverError::Unsupported("raster too tall".into()));
        }

        let mut out = Vec::new();
        for step in D30_BRINGUP {
            out.extend_from_slice(step);
        }

        for _ in 0..ctx.copies() {
            out.extend_from_slice(D30_JOB_MARK);
            out.extend_from_slice(&[ESC, b'@']);
            out.extend_from_slice(&[GS, b'v', b'0', 0x00]);
            out.push((stride & 0xFF) as u8);
            out.push((stride >> 8) as u8);
            out.push((bitmap.height & 0xFF) as u8);
            out.push((bitmap.height >> 8) as u8);
            out.extend_from_slice(&bitmap.data);
        }

        Ok(out)
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
    fn emits_d30_bringup_and_raster() {
        let bmp = MonoBitmap::new(16, 2);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(12.0, 40.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoD30Driver::new().encode(&bmp, &ctx).unwrap();

        assert_eq!(&bytes[0..3], &[US, 0x11, 0x38]);
        assert!(bytes.windows(D30_JOB_MARK.len()).any(|w| w == D30_JOB_MARK));
        assert!(bytes.windows(2).any(|w| w == [ESC, b'@']));
        assert!(bytes.windows(4).any(|w| w == [GS, b'v', b'0', 0x00]));
        // No M02 / M110 footers.
        assert!(!bytes.windows(3).any(|w| w == [ESC, b'd', 0x02]));
        assert!(!bytes.windows(4).any(|w| w == [US, 0xF0, 0x05, 0x00]));
    }
}
