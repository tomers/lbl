//! High-quality downscaling used by the second pass.

use image::imageops::FilterType;
use image::RgbaImage;

/// Downscale (or resize) `img` to the exact target dimensions using a
/// high-quality Lanczos3 filter. This is the second pass that preserves
/// photographic detail before dithering: render large, then shrink smoothly.
pub fn downscale(img: &RgbaImage, target_w: u32, target_h: u32) -> RgbaImage {
    if img.width() == target_w && img.height() == target_h {
        return img.clone();
    }
    image::imageops::resize(img, target_w.max(1), target_h.max(1), FilterType::Lanczos3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscales_to_target() {
        let big = RgbaImage::from_pixel(300, 150, image::Rgba([255, 255, 255, 255]));
        let small = downscale(&big, 100, 50);
        assert_eq!(small.dimensions(), (100, 50));
    }

    #[test]
    fn identity_when_same_size() {
        let img = RgbaImage::from_pixel(10, 10, image::Rgba([0, 0, 0, 255]));
        let out = downscale(&img, 10, 10);
        assert_eq!(out.dimensions(), (10, 10));
    }
}
