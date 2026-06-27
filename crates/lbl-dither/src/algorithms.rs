//! Dithering algorithms producing a 1-bit [`MonoBitmap`].

use lbl_core::bitmap::MonoBitmap;

use crate::grayscale::Gray;

/// 8x8 Bayer matrix for ordered dithering (values 0..63).
#[rustfmt::skip]
const BAYER8: [[u8; 8]; 8] = [
    [ 0, 32,  8, 40,  2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44,  4, 36, 14, 46,  6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [ 3, 35, 11, 43,  1, 33,  9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47,  7, 39, 13, 45,  5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// A pixel at/above this luminance is white; below it is black.
const MID: f32 = 128.0;

/// Hard threshold: ink where luminance is below `level`.
pub fn threshold(gray: &Gray, level: u8) -> MonoBitmap {
    let mut bmp = MonoBitmap::new(gray.width, gray.height);
    for y in 0..gray.height {
        for x in 0..gray.width {
            let v = gray.lum[gray.idx(x, y)];
            bmp.set(x, y, v < level as f32);
        }
    }
    bmp
}

/// Ordered (Bayer 8x8) dithering — fast and tile-free, good for flat tints.
pub fn ordered(gray: &Gray) -> MonoBitmap {
    let mut bmp = MonoBitmap::new(gray.width, gray.height);
    for y in 0..gray.height {
        for x in 0..gray.width {
            let v = gray.lum[gray.idx(x, y)];
            // Map Bayer value 0..63 to a 0..255 threshold.
            let t = (BAYER8[(y % 8) as usize][(x % 8) as usize] as f32 + 0.5) * 4.0;
            bmp.set(x, y, v < t);
        }
    }
    bmp
}

/// Floyd-Steinberg error diffusion. When `photo_aware` is set, near-pure source
/// pixels (typical of text/line art) are hard-thresholded and excluded from
/// error diffusion so edges stay crisp, while mid-tones (photos) dither
/// smoothly.
pub fn floyd_steinberg(gray: &Gray, photo_aware: bool) -> MonoBitmap {
    let w = gray.width as i64;
    let h = gray.height as i64;
    let mut buf = gray.lum.clone();
    let mut bmp = MonoBitmap::new(gray.width, gray.height);

    let at = |x: i64, y: i64| -> usize { (y * w + x) as usize };

    for y in 0..h {
        for x in 0..w {
            let old = buf[at(x, y)];
            let new = if old < MID { 0.0 } else { 255.0 };
            bmp.set(x as u32, y as u32, new == 0.0);

            let is_pure = old <= 8.0 || old >= 247.0;
            if photo_aware && is_pure {
                continue; // keep crisp; do not diffuse
            }

            let err = old - new;
            let mut diffuse = |dx: i64, dy: i64, factor: f32| {
                let nx = x + dx;
                let ny = y + dy;
                if nx >= 0 && nx < w && ny >= 0 && ny < h {
                    buf[at(nx, ny)] += err * factor;
                }
            };
            diffuse(1, 0, 7.0 / 16.0);
            diffuse(-1, 1, 3.0 / 16.0);
            diffuse(0, 1, 5.0 / 16.0);
            diffuse(1, 1, 1.0 / 16.0);
        }
    }
    bmp
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid(lum: u8) -> Gray {
        let img = RgbaImage::from_pixel(8, 8, Rgba([lum, lum, lum, 255]));
        Gray::from_rgba(&img)
    }

    #[test]
    fn threshold_black_and_white() {
        let black = threshold(&solid(0), 128);
        assert!(black.get(0, 0));
        let white = threshold(&solid(255), 128);
        assert!(!white.get(0, 0));
    }

    #[test]
    fn floyd_pure_black_all_ink() {
        let bmp = floyd_steinberg(&solid(0), true);
        for y in 0..8 {
            for x in 0..8 {
                assert!(bmp.get(x, y));
            }
        }
    }

    #[test]
    fn ordered_midtone_is_mixed() {
        let bmp = ordered(&solid(128));
        let ink: u32 = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .map(|(x, y)| bmp.get(x, y) as u32)
            .sum();
        // Roughly half on/half off for a 50% gray.
        assert!(ink > 20 && ink < 44, "ink count was {ink}");
    }
}
