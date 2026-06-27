//! Conversion of RGBA images to a luminance buffer.

use image::RgbaImage;

/// A grayscale buffer of luminance values in `[0.0, 255.0]` (0 = black).
#[derive(Debug, Clone)]
pub struct Gray {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major luminance values.
    pub lum: Vec<f32>,
}

impl Gray {
    /// Convert an RGBA image to luminance, compositing over a white background
    /// (so transparent areas read as "no ink").
    pub fn from_rgba(img: &RgbaImage) -> Self {
        let (width, height) = img.dimensions();
        let mut lum = Vec::with_capacity((width * height) as usize);
        for pixel in img.pixels() {
            let [r, g, b, a] = pixel.0;
            let a = a as f32 / 255.0;
            // Composite over white.
            let r = r as f32 * a + 255.0 * (1.0 - a);
            let g = g as f32 * a + 255.0 * (1.0 - a);
            let b = b as f32 * a + 255.0 * (1.0 - a);
            // Rec. 601 luma.
            lum.push(0.299 * r + 0.587 * g + 0.114 * b);
        }
        Self { width, height, lum }
    }

    #[inline]
    pub(crate) fn idx(&self, x: u32, y: u32) -> usize {
        (y * self.width + x) as usize
    }
}
