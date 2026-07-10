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
