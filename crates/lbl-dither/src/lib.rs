//! Convert raster images to a printer's bit depth with photo-aware dithering.
//!
//! The output is a 1-bit [`MonoBitmap`] (the format printer drivers consume).
//! Several algorithms are available via [`Algorithm`]; the default `Auto` uses
//! photo-aware Floyd-Steinberg error diffusion, which keeps text/line art crisp
//! while dithering photographic mid-tones smoothly.
//!
//! ```
//! use lbl_dither::{dither, Algorithm};
//! use image::{Rgba, RgbaImage};
//!
//! let img = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 255]));
//! let bmp = dither(&img, Algorithm::Auto);
//! assert!(bmp.get(0, 0)); // black -> ink
//! ```

mod algorithms;
mod grayscale;

pub use grayscale::Gray;

use image::RgbaImage;
use lbl_core::bitmap::MonoBitmap;

/// Errors produced by the dithering stage.
#[derive(Debug, thiserror::Error)]
pub enum DitherError {
    /// Failed to decode/encode an image.
    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    /// An unknown algorithm name was supplied.
    #[error("unknown dither algorithm: {0}")]
    UnknownAlgorithm(String),

    /// An I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Available dithering strategies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    /// Photo-aware Floyd-Steinberg (default): crisp text, smooth photos.
    Auto,
    /// Plain Floyd-Steinberg error diffusion (diffuses everywhere).
    FloydSteinberg,
    /// Ordered (Bayer 8x8) dithering.
    Ordered,
    /// Hard threshold at the given luminance level (no dithering).
    Threshold(u8),
}

impl Algorithm {
    /// Parse an algorithm from a CLI-friendly name.
    pub fn parse(name: &str) -> Result<Self, DitherError> {
        Ok(match name.to_ascii_lowercase().as_str() {
            "auto" => Algorithm::Auto,
            "bayer" | "ordered" => Algorithm::Ordered,
            "floyd" | "floyd-steinberg" | "fs" => Algorithm::FloydSteinberg,
            "none" | "threshold" => Algorithm::Threshold(128),
            other => return Err(DitherError::UnknownAlgorithm(other.to_string())),
        })
    }
}

/// Dither an RGBA image into a 1-bit [`MonoBitmap`] using `algorithm`.
pub fn dither(img: &RgbaImage, algorithm: Algorithm) -> MonoBitmap {
    let gray = Gray::from_rgba(img);
    match algorithm {
        Algorithm::Auto => algorithms::floyd_steinberg(&gray, true),
        Algorithm::FloydSteinberg => algorithms::floyd_steinberg(&gray, false),
        Algorithm::Ordered => algorithms::ordered(&gray),
        Algorithm::Threshold(level) => algorithms::threshold(&gray, level),
    }
}

/// Split an RGBA label into black (high-energy) and red (low-energy) planes for
/// Brother QL DK-22251 two-color tape.
///
/// Heuristic mirrors `brother_ql`'s HSV filters: saturated red-ish pixels go to
/// the red plane; dark non-red pixels go to black. Red is subtracted from black
/// so overlapping ink prefers red.
pub fn split_black_red(img: &RgbaImage, black_luma_max: u8) -> (MonoBitmap, MonoBitmap) {
    let w = img.width();
    let h = img.height();
    let mut black = MonoBitmap::new(w, h);
    let mut red = MonoBitmap::new(w, h);

    for y in 0..h {
        for x in 0..w {
            let p = img.get_pixel(x, y).0;
            let (r, g, b, a) = (p[0], p[1], p[2], p[3]);
            if a < 128 {
                continue;
            }
            let luma = ((u16::from(r) * 77 + u16::from(g) * 150 + u16::from(b) * 29) / 256) as u8;
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let chroma = max.saturating_sub(min);
            // Hue near red/magenta with enough saturation and brightness.
            let is_red =
                chroma > 40 && r >= g.saturating_add(30) && r >= b.saturating_add(30) && luma > 40;
            if is_red {
                red.set(x, y, true);
            } else if luma <= black_luma_max {
                black.set(x, y, true);
            }
        }
    }
    (black, red)
}

#[cfg(test)]
mod two_color_tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn splits_red_and_black_pixels() {
        let mut img = RgbaImage::from_pixel(2, 1, Rgba([255, 255, 255, 255]));
        img.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        img.put_pixel(1, 0, Rgba([220, 30, 30, 255]));
        let (black, red) = split_black_red(&img, 80);
        assert!(black.get(0, 0));
        assert!(!black.get(1, 0));
        assert!(!red.get(0, 0));
        assert!(red.get(1, 0));
    }
}
