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
#[derive(Debug, Clone)]
pub struct RenderRequest {
    /// Target width in device dots.
    pub width_dots: u32,
    /// Target height in device dots. `None` lets the content determine the
    /// height (continuous media); the rendered height is downscaled by the same
    /// supersample factor.
    pub height_dots: Option<u32>,
    /// Supersample factor for the high-resolution first pass (clamped to >= 1).
    pub supersample: u32,
}

impl RenderRequest {
    /// The high-resolution width used for the first pass.
    pub fn hires_width(&self) -> u32 {
        self.width_dots * self.supersample.max(1)
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

    // Determine the exact target dimensions. For auto-height, derive the target
    // height from the captured high-res height divided by the supersample.
    let target_w = req.width_dots.max(1);
    let target_h = match req.height_dots {
        Some(h) => h.max(1),
        None => (hires.height() / factor).max(1),
    };

    Ok(downscale(&hires, target_w, target_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBackend;
    impl RenderBackend for FakeBackend {
        fn rasterize(&self, _html: &str, width: u32, height: Option<u32>) -> Result<RgbaImage> {
            Ok(RgbaImage::from_pixel(
                width,
                height.unwrap_or(width),
                image::Rgba([255, 255, 255, 255]),
            ))
        }
    }

    #[test]
    fn two_pass_produces_target_dims_fixed() {
        let req = RenderRequest {
            width_dots: 200,
            height_dots: Some(100),
            supersample: 3,
        };
        let img = render_two_pass(&FakeBackend, "<html></html>", &req).unwrap();
        assert_eq!(img.dimensions(), (200, 100));
    }

    #[test]
    fn two_pass_auto_height_divides_by_factor() {
        let req = RenderRequest {
            width_dots: 100,
            height_dots: None,
            supersample: 4,
        };
        // Fake backend returns a square at hires width (400x400) -> /4 = 100.
        let img = render_two_pass(&FakeBackend, "<html></html>", &req).unwrap();
        assert_eq!(img.dimensions(), (100, 100));
    }
}
