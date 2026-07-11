//! Phomemo M02-class driver (ESC/POS raster with vendor framing).
//!
//! Phomemo pocket printers (M02 / M02S / M02 Pro and close relatives) speak an
//! ESC/POS-like stream with extra `1F 11 …` markers around the job. The raster
//! payload itself is standard `GS v 0`. Reverse-engineered from community
//! captures ([vivier/phomemo-tools](https://github.com/vivier/phomemo-tools)).
//!
//! M02X uses different proprietary framing — see [`crate::PhomemoM02xDriver`].
//!
//! `lbl` is not affiliated with Phomemo.

use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;

/// Phomemo M02-family vendor markers observed around ESC/POS jobs.
const PHOMEMO_PREAMBLE: &[u8] = &[0x1F, 0x11, 0x02, 0x04];
const PHOMEMO_EPILOGUE: &[u8] = &[
    0x1F, 0x11, 0x08, 0x1F, 0x11, 0x0E, 0x1F, 0x11, 0x07, 0x1F, 0x11, 0x09,
];

/// Driver for Phomemo M02-class thermal printers.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhomemoDriver;

impl PhomemoDriver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }
}

impl Driver for PhomemoDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Phomemo
    }

    fn name(&self) -> &'static str {
        "phomemo-m02"
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
        out.extend_from_slice(&[ESC, b'@']);
        // Center justification (common in Phomemo app captures).
        out.extend_from_slice(&[ESC, b'a', 0x01]);
        out.extend_from_slice(PHOMEMO_PREAMBLE);

        for _ in 0..ctx.copies() {
            // Chunk tall images into ≤255-line GS v 0 blocks (Phomemo limit).
            let mut y0 = 0u32;
            while y0 < bitmap.height {
                let chunk_h = (bitmap.height - y0).min(255);
                out.extend_from_slice(&[GS, b'v', b'0', 0x00]);
                out.push((stride & 0xFF) as u8);
                out.push((stride >> 8) as u8);
                out.push((chunk_h & 0xFF) as u8);
                out.push(0x00);
                for y in y0..y0 + chunk_h {
                    let start = (y as usize) * stride;
                    out.extend_from_slice(&bitmap.data[start..start + stride]);
                }
                y0 += chunk_h;
            }

            out.extend_from_slice(&[ESC, b'd', 0x02]);
            out.extend_from_slice(&[ESC, b'd', 0x02]);
            out.extend_from_slice(PHOMEMO_EPILOGUE);
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
    fn emits_phomemo_framing_around_gs_v0() {
        let bmp = MonoBitmap::new(16, 2);
        let caps = PrinterCapabilities::default();
        let job = JobSpec::new(Media::continuous(48.0, Dpi(203.0)));
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = PhomemoDriver::new().encode(&bmp, &ctx).unwrap();

        assert_eq!(&bytes[0..2], &[ESC, b'@']);
        assert_eq!(&bytes[2..5], &[ESC, b'a', 0x01]);
        assert_eq!(&bytes[5..9], PHOMEMO_PREAMBLE);
        assert!(bytes.windows(4).any(|w| w == [GS, b'v', b'0', 0x00]));
        assert!(bytes
            .windows(PHOMEMO_EPILOGUE.len())
            .any(|w| w == PHOMEMO_EPILOGUE));
    }
}
