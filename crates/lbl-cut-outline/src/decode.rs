//! Decode WOFF2 or pass through SFNT font bytes.

use crate::CutOutlineError;

/// Magic for WOFF2 (`wOF2`).
const WOFF2_TAG: [u8; 4] = [0x77, 0x4F, 0x46, 0x32];
/// Magic for WOFF1 (`wOFF`) — not supported; ask callers to use WOFF2/TTF.
const WOFF1_TAG: [u8; 4] = [0x77, 0x4F, 0x46, 0x46];

/// Return SFNT (TTF/OTF) bytes, decompressing WOFF2 when needed.
pub fn decode_font_bytes(bytes: &[u8]) -> Result<Vec<u8>, CutOutlineError> {
    if bytes.len() < 4 {
        return Err(CutOutlineError::msg("font bytes too short"));
    }
    let tag = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if tag == WOFF2_TAG {
        return wuff::decompress_woff2(bytes)
            .map_err(|e| CutOutlineError::msg(format!("woff2 decode failed: {e}")));
    }
    if tag == WOFF1_TAG {
        return Err(CutOutlineError::msg(
            "WOFF1 fonts are not supported; use WOFF2 or TTF/OTF",
        ));
    }
    Ok(bytes.to_vec())
}
