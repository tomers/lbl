//! Phomemo M110-class driver (M110 / M120 / M220 business labelers).
//!
//! Distinct from the M02 pocket framing: jobs open with print-speed /
//! density / media-type vendor commands, then a full-height `GS v 0` raster,
//! then a short `1F F0` footer. Reverse-engineered from
//! [vivier/phomemo-tools](https://github.com/vivier/phomemo-tools).
//!
//! `lbl` is not affiliated with Phomemo.

use lbl_core::media::MediaLength;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const US: u8 = 0x1F;

/// Default print speed (`ESC N 0x0d`): 0x01 slow … 0x05 fast.
const DEFAULT_SPEED: u8 = 0x05;
/// Default density (`ESC N 0x04`): 0x01 … 0x0f.
const DEFAULT_DENSITY: u8 = 0x0f;

/// Media type for gap-sensed die-cut labels (`1F 11`).
const MEDIA_GAPS: u8 = 0x0a;
/// Media type for continuous stock.
const MEDIA_CONTINUOUS: u8 = 0x0b;

/// Footer observed after each M110 raster job.
const M110_FOOTER: &[u8] = &[US, 0xF0, 0x05, 0x00, US, 0xF0, 0x03, 0x00];

/// Driver for Phomemo M110 / M120 / M220 thermal label printers.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhomemoM110Driver;

impl PhomemoM110Driver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    /// Map job density (1–5 UI scale) onto the M110 `ESC N 0x04` byte.
    fn density_byte(job_density: Option<u8>) -> u8 {
        match job_density {
            Some(1) => 0x03,
            Some(2) => 0x06,
            Some(3) => 0x09,
            Some(4) => 0x0c,
            Some(5) => 0x0f,
            _ => DEFAULT_DENSITY,
        }
    }

    fn media_type(length: MediaLength) -> u8 {
        match length {
            MediaLength::Continuous => MEDIA_CONTINUOUS,
            MediaLength::Fixed(_) => MEDIA_GAPS,
        }
    }
}

impl Driver for PhomemoM110Driver {
    fn protocol(&self) -> Protocol {
        Protocol::PhomemoM110
    }

    fn name(&self) -> &'static str {
        "phomemo-m110"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["phomemo-m110", "phomemom110", "m110"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        let stride = bitmap.stride();
        if stride > 0xFFFF {
            return Err(DriverError::Unsupported("raster too wide".into()));
        }
        if bitmap.height > 0xFFFF {
            return Err(DriverError::Unsupported("raster too tall".into()));
        }

        let density = Self::density_byte(ctx.job.density);
        let media = Self::media_type(ctx.job.media.length);
        let mut out = Vec::new();

        for _ in 0..ctx.copies() {
            out.extend_from_slice(&[ESC, b'N', 0x0d, DEFAULT_SPEED]);
            out.extend_from_slice(&[ESC, b'N', 0x04, density]);
            out.extend_from_slice(&[US, 0x11, media]);
            out.extend_from_slice(&[GS, b'v', b'0', 0x00]);
            out.push((stride & 0xFF) as u8);
            out.push((stride >> 8) as u8);
            out.push((bitmap.height & 0xFF) as u8);
            out.push((bitmap.height >> 8) as u8);
            out.extend_from_slice(&bitmap.data);
            out.extend_from_slice(M110_FOOTER);
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
    fn emits_m110_header_raster_and_footer() {
        let bmp = MonoBitmap::new(16, 2);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::fixed(40.0, 30.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoM110Driver::new().encode(&bmp, &ctx).unwrap();

        assert_eq!(&bytes[0..4], &[ESC, b'N', 0x0d, DEFAULT_SPEED]);
        assert_eq!(&bytes[4..8], &[ESC, b'N', 0x04, DEFAULT_DENSITY]);
        assert_eq!(&bytes[8..11], &[US, 0x11, MEDIA_GAPS]);
        assert_eq!(&bytes[11..15], &[GS, b'v', b'0', 0x00]);
        assert_eq!(&bytes[15..17], &[0x02, 0x00]);
        assert_eq!(&bytes[17..19], &[0x02, 0x00]);
        assert_eq!(&bytes[bytes.len() - 8..], M110_FOOTER);
    }

    #[test]
    fn continuous_media_sets_continuous_type() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::continuous(50.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoM110Driver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(3).any(|w| w == [US, 0x11, MEDIA_CONTINUOUS]));
    }

    #[test]
    fn density_maps_from_job() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = PrinterCapabilities::default();
        let mut job = JobSpec::new(Media::fixed(40.0, 30.0, Dpi(203.0)));
        job.density = Some(2);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoM110Driver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [ESC, b'N', 0x04, 0x06]));
    }
}
