//! Render HTML to a raster image sized to printer media.
//!
//! Rendering is **two-pass** to keep photographic quality through the eventual
//! 1-bit dithering: the page is first rendered at a high resolution
//! (`supersample` x the target), then downscaled with a high-quality Lanczos3
//! filter to the exact device dimensions. See [`render_two_pass`].
//!
//! The actual page rasterization is provided by a [`RenderBackend`]. The
//! default backend drives headless Chromium in-process via
//! [chromiumoxide](https://docs.rs/chromiumoxide) (feature `chromium`, enabled
//! by default). A [`SidecarBackend`] can instead drive an external Node /
//! Playwright process behind the same trait.

mod backend;
mod downscale;

#[cfg(feature = "chromium")]
mod chromium;

pub use backend::{RenderBackend, SidecarBackend};
pub use downscale::downscale;

#[cfg(feature = "chromium")]
pub use chromium::ChromiumBackend;

use image::RgbaImage;
use lbl_core::Rotation;

/// Errors produced by the rendering stage.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The backend (browser/sidecar) failed.
    #[error("render backend error: {0}")]
    Backend(String),

    /// Decoding the backend's image output failed.
    #[error("image decode error: {0}")]
    Image(#[from] image::ImageError),

    /// An I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, RenderError>;

/// A request describing the target raster size at device resolution.
///
/// Each axis may be fixed (`Some`) or content-determined (`None`). Fixed-length
/// media pins both axes; continuous media leaves the feed axis `None` so the
/// content sizes it. At most one axis is normally `None`.
#[derive(Debug, Clone)]
pub struct RenderRequest {
    /// Target width in device dots. `None` lets the content determine the width
    /// (e.g. continuous media rendered in landscape).
    pub width_dots: Option<u32>,
    /// Target height in device dots. `None` lets the content determine the
    /// height (e.g. continuous media rendered in portrait).
    pub height_dots: Option<u32>,
    /// Supersample factor for the high-resolution first pass (clamped to >= 1).
    pub supersample: u32,
}

impl RenderRequest {
    /// The high-resolution width used for the first pass, if fixed.
    pub fn hires_width(&self) -> Option<u32> {
        self.width_dots.map(|w| w * self.supersample.max(1))
    }

    /// The high-resolution height used for the first pass, if fixed.
    pub fn hires_height(&self) -> Option<u32> {
        self.height_dots.map(|h| h * self.supersample.max(1))
    }
}

/// Render `html` to a device-resolution raster using `backend`, applying the
/// two-pass high-res-then-downscale strategy.
pub fn render_two_pass<B: RenderBackend>(
    backend: &B,
    html: &str,
    req: &RenderRequest,
) -> Result<RgbaImage> {
    let factor = req.supersample.max(1);
    let hires = backend.rasterize(html, req.hires_width(), req.hires_height())?;

    // Determine the exact target dimensions. For an auto axis, derive the
    // target extent from the captured high-res size divided by the supersample.
    let target_w = match req.width_dots {
        Some(w) => w.max(1),
        None => (hires.width() / factor).max(1),
    };
    let target_h = match req.height_dots {
        Some(h) => h.max(1),
        None => (hires.height() / factor).max(1),
    };

    Ok(downscale(&hires, target_w, target_h))
}

/// Rotate a rendered raster by the given [`Rotation`] (clockwise quarter-turns).
///
/// Content is laid out in the chosen reading frame first (by sizing the render
/// request accordingly); this turns that frame onto the print head.
pub fn apply_rotation(img: RgbaImage, rotation: Rotation) -> RgbaImage {
    match rotation {
        Rotation::None => img,
        Rotation::Cw90 => image::imageops::rotate90(&img),
        Rotation::Cw180 => image::imageops::rotate180(&img),
        Rotation::Cw270 => image::imageops::rotate270(&img),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBackend;
    impl RenderBackend for FakeBackend {
        fn rasterize(
            &self,
            _html: &str,
            width: Option<u32>,
            height: Option<u32>,
        ) -> Result<RgbaImage> {
            // Default an auto axis to a square so the divide-by-factor path is
            // exercised in tests.
            let w = width.or(height).unwrap_or(1);
            let h = height.or(width).unwrap_or(1);
            Ok(RgbaImage::from_pixel(
                w,
                h,
                image::Rgba([255, 255, 255, 255]),
            ))
        }
    }

    #[test]
    fn two_pass_produces_target_dims_fixed() {
        let req = RenderRequest {
            width_dots: Some(200),
            height_dots: Some(100),
            supersample: 3,
        };
        let img = render_two_pass(&FakeBackend, "<html></html>", &req).unwrap();
        assert_eq!(img.dimensions(), (200, 100));
    }

    #[test]
    fn two_pass_auto_height_divides_by_factor() {
        let req = RenderRequest {
            width_dots: Some(100),
            height_dots: None,
            supersample: 4,
        };
        // Fake backend returns a square at hires width (400x400) -> /4 = 100.
        let img = render_two_pass(&FakeBackend, "<html></html>", &req).unwrap();
        assert_eq!(img.dimensions(), (100, 100));
    }

    #[test]
    fn two_pass_auto_width_divides_by_factor() {
        let req = RenderRequest {
            width_dots: None,
            height_dots: Some(100),
            supersample: 4,
        };
        // Fake backend returns a square at hires height (400x400) -> /4 = 100.
        let img = render_two_pass(&FakeBackend, "<html></html>", &req).unwrap();
        assert_eq!(img.dimensions(), (100, 100));
    }

    #[test]
    fn rotation_swaps_axes_for_quarter_turns() {
        let img = RgbaImage::from_pixel(30, 10, image::Rgba([0, 0, 0, 255]));
        assert_eq!(
            apply_rotation(img.clone(), Rotation::None).dimensions(),
            (30, 10)
        );
        assert_eq!(
            apply_rotation(img.clone(), Rotation::Cw90).dimensions(),
            (10, 30)
        );
        assert_eq!(
            apply_rotation(img.clone(), Rotation::Cw180).dimensions(),
            (30, 10)
        );
        assert_eq!(apply_rotation(img, Rotation::Cw270).dimensions(), (10, 30));
    }
}
