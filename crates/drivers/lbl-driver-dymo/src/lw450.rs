//! DYMO LabelWriter 450-series classic raster driver.
//!
//! The LabelWriter 450 family (450, 450 Turbo, 450 Twin Turbo, 4XL) uses a
//! row-oriented SYN-based protocol that differs from the structured job framing
//! introduced on the LW550. Each raster row is sent as a `SYN` byte followed by
//! the packed row data (MSB-first, 1 = ink), left-aligned on the physical head.
//!
//! ## Print job structure
//!
//! ```text
//! ESC @                   reset printer
//! ESC D n                 set bytes per line (head_dots / 8)
//! ESC <density>           c/d/e/g strobe duty
//! ESC h | ESC i           text (300×300) or graphics (300×600) mode
//! ESC L n1 n2             label length, big-endian dots (or continuous)
//! [ESC q n]               Twin Turbo roll select
//! per copy:
//!   SYN <row_bytes...>    one raster row, padded to head width
//!   ESC G                 short form feed (between copies)
//! ESC E                   feed to tear position (after last copy)
//! ESC A                   get printer status (1-byte reply)
//! ```
//!
//! Reference: DYMO LabelWriter 400/450 series Tech Ref; thermal-label lw-raster.

use lbl_core::job::LwOutputMode;
use lbl_core::media::MediaLength;
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

use crate::ESC;
use crate::SYN;

const RESET: u8 = b'@';
const BYTES_PER_LINE: u8 = b'D';
const LABEL_LENGTH: u8 = b'L';
const ROLL_SELECT: u8 = b'q';
const TEXT_MODE: u8 = b'h';
const GRAPHICS_MODE: u8 = b'i';
const DENSITY_LIGHT: u8 = b'c';
const DENSITY_MEDIUM: u8 = b'd';
const DENSITY_NORMAL: u8 = b'e';
const DENSITY_DARK: u8 = b'g';
const SHORT_FORM_FEED: u8 = b'G';
const FORM_FEED_TEAR: u8 = b'E';
const STATUS: u8 = b'A';

/// Dots across the 57 mm print head (450 / 450 Turbo / 450 Twin Turbo).
const HEAD_DOTS_57MM: u32 = 672;

/// Dots across the 101 mm print head (4XL).
const HEAD_DOTS_4XL: u32 = 1248;

fn density_opcode(job_density: Option<u8>) -> u8 {
    match job_density {
        Some(1) | Some(2) => DENSITY_LIGHT, // ~75%
        Some(3) => DENSITY_MEDIUM,          // ~87.5%
        Some(4) | None => DENSITY_NORMAL,   // 100%
        Some(5) => DENSITY_DARK,            // ~112.5%
        Some(pct) if pct > 0 && pct < 80 => DENSITY_LIGHT,
        Some(pct) if (80..95).contains(&pct) => DENSITY_MEDIUM,
        Some(pct) if (95..110).contains(&pct) => DENSITY_NORMAL,
        Some(_) => DENSITY_DARK,
    }
}

/// Driver for the DYMO LabelWriter 450 series (450 / 450 Turbo / 450 Twin Turbo / 4XL).
#[derive(Debug, Default, Clone, Copy)]
pub struct LabelWriter450Driver;

impl LabelWriter450Driver {
    /// Create a new driver instance.
    pub fn new() -> Self {
        Self
    }

    fn head_dots(ctx: &EncodeContext<'_>) -> u32 {
        let from_caps = lbl_core::units::Millimeters(ctx.capabilities.max_width_mm)
            .to_dots(ctx.capabilities.dpi);
        if from_caps.0 > 900 {
            HEAD_DOTS_4XL
        } else {
            HEAD_DOTS_57MM
        }
    }

    fn pad_to_head(bitmap: &MonoBitmap, head_dots: u32) -> Result<MonoBitmap, DriverError> {
        if bitmap.width > head_dots {
            return Err(DriverError::Unsupported(format!(
                "bitmap width {} exceeds print head width {head_dots} dots",
                bitmap.width
            )));
        }
        if bitmap.width == head_dots {
            return Ok(bitmap.clone());
        }
        let mut out = MonoBitmap::new(head_dots, bitmap.height);
        for y in 0..bitmap.height {
            for x in 0..bitmap.width {
                if bitmap.get(x, y) {
                    out.set(x, y, true);
                }
            }
        }
        Ok(out)
    }

    fn emit_rows(out: &mut Vec<u8>, bitmap: &MonoBitmap) {
        for y in 0..bitmap.height {
            out.push(SYN);
            out.extend_from_slice(bitmap.row(y));
        }
    }

    fn label_length_be(ctx: &EncodeContext<'_>, bitmap: &MonoBitmap) -> [u8; 2] {
        match ctx.job.media.length {
            MediaLength::Continuous => {
                // Negative two-byte value selects continuous-feed mode.
                [0x80, 0x00]
            }
            MediaLength::Fixed(mm) => {
                let dpi = ctx.capabilities.dpi.0;
                let dots = ((mm / 25.4) * dpi).round().clamp(1.0, 32767.0) as u16;
                // Slightly larger than true length so TOF is found before timeout.
                let search = dots.saturating_add(dots / 20).max(bitmap.height as u16);
                search.to_be_bytes()
            }
        }
    }
}

impl Driver for LabelWriter450Driver {
    fn protocol(&self) -> Protocol {
        Protocol::DymoLwClassic
    }

    fn name(&self) -> &'static str {
        "dymo-labelwriter-450"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["dymo-lw-classic", "dymolwclassic", "lw450"]
    }

    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.width == 0 || bitmap.height == 0 {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }

        let head_dots = Self::head_dots(ctx);
        let bitmap = Self::pad_to_head(bitmap, head_dots)?;
        let bytes_per_line = (head_dots / 8) as u8;
        let copies = ctx.copies();
        let dymo = ctx.job.driver.dymo.unwrap_or_default();
        let output_mode = dymo.output_mode.unwrap_or_default();
        let length = Self::label_length_be(ctx, &bitmap);

        let mut out = Vec::with_capacity(bitmap.data.len() * copies as usize + 32);

        out.extend_from_slice(&[ESC, RESET]);
        out.extend_from_slice(&[ESC, BYTES_PER_LINE, bytes_per_line]);
        out.extend_from_slice(&[ESC, density_opcode(ctx.job.density)]);
        out.extend_from_slice(&[
            ESC,
            match output_mode {
                LwOutputMode::Text => TEXT_MODE,
                LwOutputMode::Graphics => GRAPHICS_MODE,
            },
        ]);
        out.extend_from_slice(&[ESC, LABEL_LENGTH, length[0], length[1]]);
        if let Some(roll) = dymo.roll {
            out.extend_from_slice(&[ESC, ROLL_SELECT, roll.wire_byte()]);
        }

        for i in 0..copies {
            Self::emit_rows(&mut out, &bitmap);
            if i + 1 < copies {
                out.extend_from_slice(&[ESC, SHORT_FORM_FEED]);
            }
        }

        out.extend_from_slice(&[ESC, FORM_FEED_TEAR]);
        out.extend_from_slice(&[ESC, STATUS]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::{DriverOptions, DymoLwOptions, JobSpec, LwRollSelect};
    use lbl_core::media::Media;
    use lbl_core::printer::DeviceCapabilities;
    use lbl_core::units::Dpi;

    fn ctx_job(media: Media, copies: u32) -> JobSpec {
        let mut job = JobSpec::new(media);
        job.copies = copies;
        job
    }

    #[test]
    fn emits_reset_density_mode_and_length() {
        let bmp = MonoBitmap::new(8, 2);
        let caps = DeviceCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        assert_eq!(&bytes[0..2], &[ESC, b'@']);
        assert_eq!(&bytes[2..5], &[ESC, b'D', 84]);
        assert_eq!(&bytes[5..7], &[ESC, b'e']);
        assert_eq!(&bytes[7..9], &[ESC, b'h']);
        assert_eq!(bytes[9], ESC);
        assert_eq!(bytes[10], b'L');
        assert_eq!(&bytes[bytes.len() - 2..], &[ESC, b'A']);
    }

    #[test]
    fn syn_row_per_line() {
        let mut bmp = MonoBitmap::new(8, 2);
        bmp.set(0, 0, true);
        let caps = DeviceCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        let syn_count = bytes.iter().filter(|&&b| b == SYN).count();
        assert_eq!(syn_count, 2);
    }

    #[test]
    fn copies_use_short_feed_between_and_tear_at_end() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities::default();
        let job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 3);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();

        let short_ff = bytes.windows(2).filter(|w| w == &[ESC, b'G']).count();
        let tear = bytes.windows(2).filter(|w| w == &[ESC, b'E']).count();
        assert_eq!(short_ff, 2);
        assert_eq!(tear, 1);
    }

    #[test]
    fn continuous_sets_negative_length() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities::default();
        let job = ctx_job(Media::continuous(57.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();
        let l = bytes
            .windows(4)
            .find(|w| w[0] == ESC && w[1] == b'L')
            .unwrap();
        assert_eq!(&l[2..4], &[0x80, 0x00]);
    }

    #[test]
    fn twin_turbo_roll_select() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities::default();
        let mut job = ctx_job(Media::fixed(25.0, 54.0, Dpi(300.0)), 1);
        job.driver = DriverOptions {
            dymo: Some(DymoLwOptions {
                roll: Some(LwRollSelect::Left),
                ..Default::default()
            }),
        };
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();
        assert!(bytes.windows(3).any(|w| w == [ESC, b'q', b'1']));
    }

    #[test]
    fn wide_media_uses_4xl_head() {
        let bmp = MonoBitmap::new(8, 1);
        let caps = DeviceCapabilities {
            dpi: Dpi(300.0),
            max_width_mm: 104.0,
            ..Default::default()
        };
        let job = ctx_job(Media::fixed(104.0, 159.0, Dpi(300.0)), 1);
        let bytes = LabelWriter450Driver::new()
            .encode(&bmp, &EncodeContext::new(&job, &caps))
            .unwrap();
        assert_eq!(&bytes[2..5], &[ESC, b'D', 156]);
    }

    #[test]
    fn rejects_bitmap_wider_than_head() {
        let bmp = MonoBitmap::new(700, 1);
        let caps = DeviceCapabilities::default();
        let job = ctx_job(Media::fixed(57.0, 25.0, Dpi(300.0)), 1);
        let err = LabelWriter450Driver::new().encode(&bmp, &EncodeContext::new(&job, &caps));
        assert!(matches!(err, Err(DriverError::Unsupported(_))));
    }
}
