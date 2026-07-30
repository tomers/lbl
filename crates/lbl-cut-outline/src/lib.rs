//! Shape catalog font bytes into craft-cutter [`CutPath`] polylines.
//!
//! Accepts WOFF2 or SFNT (TTF/OTF) face bytes. Each glyph contour becomes one
//! closed [`CutPath`] (outers and holes alike — GPGL has no fill rule).
//! Coordinates are artboard millimeters: origin top-left, +x right, +y down.

mod curve;
mod decode;
mod outline;
mod shape;

use std::collections::HashMap;

use lbl_core::{CutPath, CutPointMm};
use serde::{Deserialize, Serialize};

pub use decode::decode_font_bytes;
pub use outline::glyph_to_cut_paths;

/// Errors from font decode, shaping, or outline extraction.
#[derive(Debug, thiserror::Error)]
pub enum CutOutlineError {
    #[error("{0}")]
    Message(String),
}

impl CutOutlineError {
    pub(crate) fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }
}

/// Horizontal alignment within the layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

/// Vertical alignment of the text block within the layout box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextValign {
    Top,
    #[default]
    Middle,
    Bottom,
}

/// One styled run of text (same face + size).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    /// Catalog slug matching a [`FontFace::slug`].
    pub font_slug: String,
    pub font_size_mm: f64,
}

/// Piece-local layout box for wrapping and alignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextLayout {
    pub width_mm: f64,
    pub height_mm: f64,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub valign: TextValign,
}

/// Font face bytes keyed by catalog slug.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FontFace {
    pub slug: String,
    /// Raw WOFF2 or SFNT bytes (JSON: standard base64).
    #[serde(with = "serde_bytes_b64")]
    pub bytes: Vec<u8>,
}

mod serde_bytes_b64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(de)?;
        base64::engine::general_purpose::STANDARD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// Spec consumed by WASM / JSON callers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextToCutPathsSpec {
    pub faces: Vec<FontFace>,
    pub runs: Vec<TextRun>,
    pub layout: TextLayout,
}

/// Convert shaped text runs into closed glyph-outline cut paths.
pub fn text_to_cut_paths(
    faces: &[FontFace],
    runs: &[TextRun],
    layout: &TextLayout,
) -> Result<Vec<CutPath>, CutOutlineError> {
    if !layout.width_mm.is_finite()
        || layout.width_mm <= 0.0
        || !layout.height_mm.is_finite()
        || layout.height_mm <= 0.0
    {
        return Err(CutOutlineError::msg("layout box must be positive"));
    }
    if runs.is_empty() || runs.iter().all(|r| r.text.trim().is_empty()) {
        return Err(CutOutlineError::msg("no text to outline"));
    }

    let mut parsed: HashMap<String, Vec<u8>> = HashMap::new();
    for face in faces {
        let slug = face.slug.trim();
        if slug.is_empty() {
            return Err(CutOutlineError::msg("font face slug is empty"));
        }
        if parsed.contains_key(slug) {
            continue;
        }
        let sfnt = decode::decode_font_bytes(&face.bytes)?;
        // Validate parse early.
        rustybuzz::Face::from_slice(&sfnt, 0)
            .ok_or_else(|| CutOutlineError::msg(format!("failed to parse font face '{slug}'")))?;
        ttf_parser::Face::parse(&sfnt, 0).map_err(|e| {
            CutOutlineError::msg(format!("failed to parse font outlines for '{slug}': {e}"))
        })?;
        parsed.insert(slug.to_string(), sfnt);
    }

    let lines = shape::layout_lines(runs, layout, &parsed)?;
    let block_height = lines.iter().map(|l| l.height_mm).sum::<f64>();
    let y0 = match layout.valign {
        TextValign::Top => 0.0,
        TextValign::Middle => ((layout.height_mm - block_height) / 2.0).max(0.0),
        TextValign::Bottom => (layout.height_mm - block_height).max(0.0),
    };

    let mut paths = Vec::new();
    let mut pen_y = y0;
    for line in &lines {
        let x0 = match layout.align {
            TextAlign::Start => 0.0,
            TextAlign::Center => ((layout.width_mm - line.width_mm) / 2.0).max(0.0),
            TextAlign::End => (layout.width_mm - line.width_mm).max(0.0),
        };
        let baseline = pen_y + line.ascender_mm;
        for g in &line.glyphs {
            let sfnt = parsed.get(&g.font_slug).ok_or_else(|| {
                CutOutlineError::msg(format!("missing font face '{}'", g.font_slug))
            })?;
            let face = ttf_parser::Face::parse(sfnt, 0).map_err(|e| {
                CutOutlineError::msg(format!("outline parse '{}': {e}", g.font_slug))
            })?;
            let units = face.units_per_em() as f64;
            if !units.is_finite() || units <= 0.0 {
                return Err(CutOutlineError::msg(format!(
                    "font '{}' has invalid units_per_em",
                    g.font_slug
                )));
            }
            let scale = g.font_size_mm / units;
            let origin = CutPointMm {
                x_mm: x0 + g.x_mm,
                y_mm: baseline + g.y_mm,
            };
            paths.extend(outline::glyph_to_cut_paths(
                &face, g.glyph_id, origin, scale,
            )?);
        }
        pen_y += line.height_mm;
    }

    if paths.is_empty() {
        return Err(CutOutlineError::msg(
            "no glyph outlines produced (font may lack outlines for this text)",
        ));
    }
    Ok(paths)
}

/// Convenience wrapper for JSON / WASM.
pub fn text_to_cut_paths_from_spec(
    spec: &TextToCutPathsSpec,
) -> Result<Vec<CutPath>, CutOutlineError> {
    text_to_cut_paths(&spec.faces, &spec.runs, &spec.layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roboto_face() -> FontFace {
        FontFace {
            slug: "roboto".into(),
            bytes: include_bytes!("../fixtures/roboto-400-latin.woff2").to_vec(),
        }
    }

    fn layout(w: f64, h: f64) -> TextLayout {
        TextLayout {
            width_mm: w,
            height_mm: h,
            align: TextAlign::Start,
            valign: TextValign::Top,
        }
    }

    #[test]
    fn outlines_letter_o_has_hole() {
        let paths = text_to_cut_paths(
            &[roboto_face()],
            &[TextRun {
                text: "O".into(),
                font_slug: "roboto".into(),
                font_size_mm: 20.0,
            }],
            &layout(40.0, 40.0),
        )
        .expect("outline O");
        assert!(
            paths.len() >= 2,
            "expected outer+hole for O, got {}",
            paths.len()
        );
        assert!(paths.iter().all(|p| p.closed && p.points.len() >= 3));
    }

    #[test]
    fn multi_glyph_advances() {
        let paths = text_to_cut_paths(
            &[roboto_face()],
            &[TextRun {
                text: "AB".into(),
                font_slug: "roboto".into(),
                font_size_mm: 12.0,
            }],
            &layout(80.0, 30.0),
        )
        .expect("outline AB");
        assert!(!paths.is_empty());
        let min_x = paths
            .iter()
            .flat_map(|p| p.points.iter().map(|pt| pt.x_mm))
            .fold(f64::INFINITY, f64::min);
        let max_x = paths
            .iter()
            .flat_map(|p| p.points.iter().map(|pt| pt.x_mm))
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(max_x - min_x > 5.0, "AB should span horizontally");
    }

    #[test]
    fn empty_text_errors() {
        let err = text_to_cut_paths(
            &[roboto_face()],
            &[TextRun {
                text: "   ".into(),
                font_slug: "roboto".into(),
                font_size_mm: 10.0,
            }],
            &layout(40.0, 20.0),
        );
        assert!(err.is_err());
    }

    #[test]
    fn missing_font_errors() {
        let err = text_to_cut_paths(
            &[roboto_face()],
            &[TextRun {
                text: "Hi".into(),
                font_slug: "missing".into(),
                font_size_mm: 10.0,
            }],
            &layout(40.0, 20.0),
        );
        assert!(err.is_err());
    }

    #[test]
    fn invalid_font_bytes_error() {
        let err = text_to_cut_paths(
            &[FontFace {
                slug: "bad".into(),
                bytes: b"not-a-font".to_vec(),
            }],
            &[TextRun {
                text: "Hi".into(),
                font_slug: "bad".into(),
                font_size_mm: 10.0,
            }],
            &layout(40.0, 20.0),
        );
        assert!(err.is_err());
    }

    #[test]
    fn wraps_long_line() {
        let paths = text_to_cut_paths(
            &[roboto_face()],
            &[TextRun {
                text: "hello world again".into(),
                font_slug: "roboto".into(),
                font_size_mm: 8.0,
            }],
            &layout(25.0, 60.0),
        )
        .expect("wrapped text");
        assert!(!paths.is_empty());
    }
}
