//! Virtual printer drivers for on-screen sinks (image file and HTML gallery).
//!
//! Unlike the hardware drivers, these do not target a physical printer
//! protocol. [`FileDriver`] turns the dithered [`MonoBitmap`] into image-file
//! bytes; [`HtmlDriver`] is a no-op encode so the pipeline can own the gallery
//! write. Both are registered so protocol id resolution stays driver-owned.
//!
//! The concrete file format for [`FileDriver`] is the virtual printer's selected
//! **media type** ([`MediaType`]) — choosing PNG vs BMP vs PBM is analogous to
//! loading a different roll of media into a real printer.
//!
//! Set bits in the bitmap are ink (black); clear bits are the blank media
//! (white).

use std::io::Cursor;

use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use lbl_driver_api::{Driver, DriverError, EncodeContext, MonoBitmap, Protocol};

mod html;

pub use html::HtmlDriver;

/// How the virtual printer stores labels to disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VirtualExportMode {
    /// Rasterize, dither, and encode as a bitmap image (emulates a printed label).
    #[default]
    Raster,
    /// Skip rasterization; emit a vector PDF sized to the configured media.
    Vector,
}

impl VirtualExportMode {
    /// Parse a CLI/config value (case-insensitive).
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "bitmap" | "image" | "raster" => VirtualExportMode::Raster,
            "pdf" | "vector" => VirtualExportMode::Vector,
            other => return Err(format!("unknown export mode: {other}")),
        })
    }

    /// The lowercase canonical name.
    pub fn name(&self) -> &'static str {
        match self {
            VirtualExportMode::Raster => "raster",
            VirtualExportMode::Vector => "vector",
        }
    }
}

/// The output file format the virtual printer emits — its "media type".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Lossless PNG (the default).
    Png,
    /// Windows bitmap.
    Bmp,
    /// Tagged Image File Format.
    Tiff,
    /// Graphics Interchange Format (single frame).
    Gif,
    /// Portable Bitmap (binary P4) — the native 1-bit interchange format.
    Pbm,
    /// Vector PDF (vector export mode only; not produced via [`encode_image`]).
    Pdf,
}

impl MediaType {
    /// Every raster image format supported in raster export mode.
    pub const RASTER: [MediaType; 5] = [
        MediaType::Png,
        MediaType::Bmp,
        MediaType::Tiff,
        MediaType::Gif,
        MediaType::Pbm,
    ];

    /// Every supported media type, in display order.
    pub const ALL: [MediaType; 6] = [
        MediaType::Png,
        MediaType::Bmp,
        MediaType::Tiff,
        MediaType::Gif,
        MediaType::Pbm,
        MediaType::Pdf,
    ];

    /// Parse a CLI-friendly media-type name (case-insensitive).
    pub fn parse(name: &str) -> Result<Self, String> {
        Ok(match name.trim().to_ascii_lowercase().as_str() {
            "bmp" => MediaType::Bmp,
            "gif" => MediaType::Gif,
            "pbm" => MediaType::Pbm,
            "pdf" => MediaType::Pdf,
            "png" => MediaType::Png,
            "tif" | "tiff" => MediaType::Tiff,
            other => return Err(format!("unknown media type: {other}")),
        })
    }

    /// The lowercase canonical name.
    pub fn name(&self) -> &'static str {
        match self {
            MediaType::Png => "png",
            MediaType::Bmp => "bmp",
            MediaType::Tiff => "tiff",
            MediaType::Gif => "gif",
            MediaType::Pbm => "pbm",
            MediaType::Pdf => "pdf",
        }
    }

    /// The file extension (without the dot).
    pub fn extension(&self) -> &'static str {
        match self {
            MediaType::Tiff => "tiff",
            other => other.name(),
        }
    }

    /// The IANA media (MIME) type.
    pub fn mime(&self) -> &'static str {
        match self {
            MediaType::Png => "image/png",
            MediaType::Bmp => "image/bmp",
            MediaType::Tiff => "image/tiff",
            MediaType::Gif => "image/gif",
            MediaType::Pbm => "image/x-portable-bitmap",
            MediaType::Pdf => "application/pdf",
        }
    }

    /// The corresponding [`image::ImageFormat`], if this type is encoded via the
    /// `image` crate (PBM is encoded natively instead).
    fn image_format(&self) -> Option<ImageFormat> {
        match self {
            MediaType::Png => Some(ImageFormat::Png),
            MediaType::Bmp => Some(ImageFormat::Bmp),
            MediaType::Tiff => Some(ImageFormat::Tiff),
            MediaType::Gif => Some(ImageFormat::Gif),
            MediaType::Pbm | MediaType::Pdf => None,
        }
    }
}

/// Convert a [`MonoBitmap`] to a grayscale image (ink -> black, blank -> white).
pub fn mono_to_luma(bitmap: &MonoBitmap) -> GrayImage {
    let mut img = GrayImage::new(bitmap.width.max(1), bitmap.height.max(1));
    for y in 0..bitmap.height {
        for x in 0..bitmap.width {
            let v = if bitmap.get(x, y) { 0 } else { 255 };
            img.put_pixel(x, y, Luma([v]));
        }
    }
    img
}

/// Encode a [`MonoBitmap`] into image-file bytes of the given [`MediaType`].
pub fn encode_image(bitmap: &MonoBitmap, media_type: MediaType) -> Result<Vec<u8>, DriverError> {
    if let MediaType::Pbm = media_type {
        return Ok(bitmap.to_pbm());
    }
    let format = media_type
        .image_format()
        .expect("non-pbm media types map to an image format");
    let luma = mono_to_luma(bitmap);
    let mut out = Cursor::new(Vec::new());
    // GIF's encoder doesn't accept the 8-bit grayscale (`L8`) color type, so
    // promote to RGBA for it. The other formats encode grayscale directly.
    let result = if let MediaType::Gif = media_type {
        DynamicImage::ImageLuma8(luma)
            .to_rgba8()
            .write_to(&mut out, format)
    } else {
        luma.write_to(&mut out, format)
    };
    result.map_err(|e| DriverError::Encode(format!("{}: {e}", media_type.name())))?;
    Ok(out.into_inner())
}

/// The virtual "print to file" driver.
#[derive(Debug, Clone, Copy)]
pub struct FileDriver {
    media_type: MediaType,
}

impl FileDriver {
    /// Create a driver that emits the given media type.
    pub fn new(media_type: MediaType) -> Self {
        Self { media_type }
    }

    /// The configured media type (output format).
    pub fn media_type(&self) -> MediaType {
        self.media_type
    }
}

impl Default for FileDriver {
    fn default() -> Self {
        Self::new(MediaType::Png)
    }
}

impl Driver for FileDriver {
    fn protocol(&self) -> Protocol {
        Protocol::Virtual
    }

    fn name(&self) -> &'static str {
        "virtual-file"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["virtual", "file"]
    }

    fn encode(&self, bitmap: &MonoBitmap, _ctx: &EncodeContext) -> Result<Vec<u8>, DriverError> {
        if bitmap.data.is_empty() {
            return Err(DriverError::Unsupported("empty bitmap".into()));
        }
        encode_image(bitmap, self.media_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::job::JobSpec;
    use lbl_core::media::Media;
    use lbl_core::printer::PrinterCapabilities;
    use lbl_core::units::Dpi;

    fn ctx_bitmap() -> MonoBitmap {
        let mut bmp = MonoBitmap::new(8, 2);
        bmp.set(0, 0, true);
        bmp.set(7, 1, true);
        bmp
    }

    #[test]
    fn png_has_signature() {
        let bmp = ctx_bitmap();
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(300.0)));
        let caps = PrinterCapabilities::default();
        let ctx = EncodeContext::new(&job, &caps);
        let bytes = FileDriver::new(MediaType::Png).encode(&bmp, &ctx).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn gif_encodes_grayscale_bitmap() {
        let bmp = ctx_bitmap();
        let bytes = encode_image(&bmp, MediaType::Gif).unwrap();
        assert!(bytes.starts_with(b"GIF"), "expected GIF magic");
    }

    #[test]
    fn pbm_is_native() {
        let bmp = ctx_bitmap();
        let bytes = encode_image(&bmp, MediaType::Pbm).unwrap();
        assert!(bytes.starts_with(b"P4\n8 2\n"));
    }

    #[test]
    fn parse_and_extension_roundtrip() {
        for mt in MediaType::ALL {
            assert_eq!(MediaType::parse(mt.name()).unwrap(), mt);
        }
        assert_eq!(MediaType::parse("TIF").unwrap(), MediaType::Tiff);
        assert!(MediaType::parse("jpeg").is_err());
    }

    #[test]
    fn export_mode_parse() {
        assert_eq!(
            VirtualExportMode::parse("vector").unwrap(),
            VirtualExportMode::Vector
        );
        assert_eq!(
            VirtualExportMode::parse("RASTER").unwrap(),
            VirtualExportMode::Raster
        );
        assert!(VirtualExportMode::parse("svg").is_err());
    }

    #[test]
    fn empty_bitmap_is_unsupported() {
        let bmp = MonoBitmap {
            width: 0,
            height: 0,
            data: vec![],
        };
        let job = JobSpec::new(Media::fixed(25.0, 54.0, Dpi(300.0)));
        let caps = PrinterCapabilities::default();
        let ctx = EncodeContext::new(&job, &caps);
        assert!(FileDriver::new(MediaType::Png).encode(&bmp, &ctx).is_err());
    }
}
