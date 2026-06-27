//! A packed 1-bit-per-pixel monochrome bitmap, the hand-off format between the
//! ditherer and printer drivers.
//!
//! Bits are packed MSB-first within each byte and rows are byte-aligned (each
//! row occupies [`MonoBitmap::stride`] bytes). A set bit (`true`) means ink
//! (black) is deposited at that pixel.

use serde::{Deserialize, Serialize};

/// A monochrome (1bpp) bitmap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonoBitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Packed rows, `height * stride` bytes, MSB-first.
    pub data: Vec<u8>,
}

impl MonoBitmap {
    /// Create a blank (all-white) bitmap of the given size.
    pub fn new(width: u32, height: u32) -> Self {
        let stride = Self::stride_for(width);
        Self {
            width,
            height,
            data: vec![0u8; stride * height as usize],
        }
    }

    /// Bytes per row (`ceil(width / 8)`).
    pub fn stride(&self) -> usize {
        Self::stride_for(self.width)
    }

    /// Bytes per row for a given width.
    pub fn stride_for(width: u32) -> usize {
        ((width + 7) / 8) as usize
    }

    /// Get the pixel at `(x, y)`. Out-of-bounds reads return `false`.
    pub fn get(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let idx = y as usize * self.stride() + (x / 8) as usize;
        let mask = 0x80u8 >> (x % 8);
        self.data[idx] & mask != 0
    }

    /// Set the pixel at `(x, y)`. Out-of-bounds writes are ignored.
    pub fn set(&mut self, x: u32, y: u32, ink: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let stride = self.stride();
        let idx = y as usize * stride + (x / 8) as usize;
        let mask = 0x80u8 >> (x % 8);
        if ink {
            self.data[idx] |= mask;
        } else {
            self.data[idx] &= !mask;
        }
    }

    /// The packed bytes of row `y`.
    pub fn row(&self, y: u32) -> &[u8] {
        let stride = self.stride();
        let start = y as usize * stride;
        &self.data[start..start + stride]
    }

    /// Serialize to a binary PBM (P4) image. PBM packs bits MSB-first with
    /// byte-aligned rows and treats `1` as black, matching this type exactly, so
    /// it is the natural interchange format between the ditherer and drivers.
    pub fn to_pbm(&self) -> Vec<u8> {
        let header = format!("P4\n{} {}\n", self.width, self.height);
        let mut out = Vec::with_capacity(header.len() + self.data.len());
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    /// Parse a binary PBM (P4) image into a [`MonoBitmap`].
    pub fn from_pbm(bytes: &[u8]) -> Result<Self, String> {
        // Read the "P4" magic, then width and height tokens, then one whitespace
        // byte, then the packed raster.
        let mut pos = 0;
        let magic = read_token(bytes, &mut pos).ok_or("missing PBM magic")?;
        if magic != "P4" {
            return Err(format!("unsupported PBM magic: {magic}"));
        }
        let width: u32 = read_token(bytes, &mut pos)
            .ok_or("missing width")?
            .parse()
            .map_err(|_| "invalid width")?;
        let height: u32 = read_token(bytes, &mut pos)
            .ok_or("missing height")?
            .parse()
            .map_err(|_| "invalid height")?;
        // Single whitespace separator after the header tokens.
        if pos < bytes.len() {
            pos += 1;
        }
        let stride = Self::stride_for(width);
        let expected = stride * height as usize;
        if bytes.len() < pos + expected {
            return Err("PBM raster too short".to_string());
        }
        Ok(Self {
            width,
            height,
            data: bytes[pos..pos + expected].to_vec(),
        })
    }
}

/// Read the next whitespace-delimited token from `bytes`, advancing `pos` to
/// just past the token (used for the PBM header).
fn read_token(bytes: &[u8], pos: &mut usize) -> Option<String> {
    while *pos < bytes.len() && bytes[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
    let start = *pos;
    while *pos < bytes.len() && !bytes[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
    if start == *pos {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes[start..*pos]).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_roundtrip_and_stride() {
        let mut bmp = MonoBitmap::new(10, 3);
        assert_eq!(bmp.stride(), 2); // ceil(10/8)
        assert_eq!(bmp.data.len(), 6);
        bmp.set(0, 0, true);
        bmp.set(9, 2, true);
        assert!(bmp.get(0, 0));
        assert!(bmp.get(9, 2));
        assert!(!bmp.get(1, 0));
        // MSB-first: x=0 is the top bit of the first byte.
        assert_eq!(bmp.row(0)[0], 0x80);
    }

    #[test]
    fn pbm_roundtrip() {
        let mut bmp = MonoBitmap::new(13, 5);
        bmp.set(0, 0, true);
        bmp.set(12, 4, true);
        bmp.set(7, 2, true);
        let pbm = bmp.to_pbm();
        assert!(pbm.starts_with(b"P4\n13 5\n"));
        let back = MonoBitmap::from_pbm(&pbm).unwrap();
        assert_eq!(back, bmp);
    }
}
