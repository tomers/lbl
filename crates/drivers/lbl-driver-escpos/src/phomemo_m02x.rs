//! Phomemo M02X driver (proprietary framing around `GS v 0` raster).
//!
//! The M02X advertises the same BLE GATT service as the older M02 family but
//! uses a different wire format: a four-step bring-up, `1F 11 02 02` density
//! (not `02 04`), a single full-height `GS v 0` block (no 255-line chunking),
//! and no M02-style feed/epilogue footer. Reverse-engineered from the official
//! iOS app ([sgrankin/phomemo](https://github.com/sgrankin/phomemo)
//! `PROTOCOL.md`).
//!
//! Transport pacing (Write Without Response, ≤182 B chunks, ~30–60 ms gaps)
//! and waiting for the `1A 0F 0C` print-complete notify are BLE-layer concerns;
//! this driver only emits the job byte stream.
//!
//! `lbl` is not affiliated with Phomemo.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const US: u8 = 0x1F;

/// Vendor bring-up captured from the iOS Phomemo app before each job.
const M02X_BRINGUP: &[&[u8]] = &[
    &[US, 0x11, 0x08],
    &[US, 0x11, 0x37, 0x64],
    &[0xAA, 0xAB, 0xAC, 0x02],
    &[US, 0x11, 0x0B],
];

/// Default heat/density byte for `1F 11 02 NN` (official app uses `0x02`).
const DEFAULT_DENSITY: u8 = 0x02;

/// Driver for Phomemo M02X thermal printers.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhomemoM02xDriver;

impl PhomemoM02xDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    /// Map job density (1–5 UI scale) onto the M02X `1F 11 02 NN` byte.
    fn density_byte(job_density: Option<u8>) -> u8 {
        match job_density {
            Some(1) => 0x01,
            Some(2) => 0x02,
            Some(3) => 0x03,
            Some(4) => 0x04,
            Some(5) => 0x05,
            _ => DEFAULT_DENSITY,
        }
    }
}

impl Driver for PhomemoM02xDriver {
    fn protocol(&self) -> Protocol {
        Protocol::PhomemoM02x
    }

    fn name(&self) -> &'static str {
        "phomemo-m02x"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["phomemo-m02x", "phomemom02x", "m02x"]
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
        let mut out = Vec::new();

        for step in M02X_BRINGUP {
            out.extend_from_slice(step);
        }

        for _ in 0..ctx.copies() {
            out.extend_from_slice(&[ESC, b'@']);
            out.extend_from_slice(&[US, 0x11, 0x02, density]);
            // Full-height GS v 0 — M02X does not require ≤255-line chunks.
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
    use lbl_core::printer::DeviceCapabilities;
    use lbl_core::units::Dpi;

    #[test]
    fn emits_m02x_bringup_and_full_height_raster() {
        let bmp = MonoBitmap::new(16, 2);
        let caps = DeviceCapabilities::default();
        let job = JobSpec::new(Media::continuous(53.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoM02xDriver::new().encode(&bmp, &ctx).unwrap();

        assert_eq!(&bytes[0..3], &[US, 0x11, 0x08]);
        assert_eq!(&bytes[3..7], &[US, 0x11, 0x37, 0x64]);
        assert_eq!(&bytes[7..11], &[0xAA, 0xAB, 0xAC, 0x02]);
        assert_eq!(&bytes[11..14], &[US, 0x11, 0x0B]);
        assert_eq!(&bytes[14..16], &[ESC, b'@']);
        assert_eq!(&bytes[16..20], &[US, 0x11, 0x02, DEFAULT_DENSITY]);
        assert_eq!(&bytes[20..24], &[GS, b'v', b'0', 0x00]);
        assert_eq!(&bytes[24..26], &[0x02, 0x00]); // stride
        assert_eq!(&bytes[26..28], &[0x02, 0x00]); // full height (not chunked)
                                                   // No M02 feed or epilogue after the raster payload.
        assert!(!bytes.windows(3).any(|w| w == [ESC, b'd', 0x02]));
        assert!(!bytes.windows(4).any(|w| w == [US, 0x11, 0x0E]));
        assert_eq!(bytes.len(), 28 + bmp.data.len());
    }

    #[test]
    fn density_maps_from_job() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities::default();
        let mut job = JobSpec::new(Media::continuous(53.0, Dpi(203.0)));
        job.density = Some(4);
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoM02xDriver::new().encode(&bmp, &ctx).unwrap();
        assert!(bytes.windows(4).any(|w| w == [US, 0x11, 0x02, 0x04]));
    }
}
