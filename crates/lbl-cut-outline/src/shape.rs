//! Line breaking + glyph advances for cut outlines.

use std::collections::HashMap;

use rustybuzz::{Face as RbFace, UnicodeBuffer};

use crate::{CutOutlineError, TextLayout, TextRun};

pub struct PlacedGlyph {
    pub font_slug: String,
    pub glyph_id: u16,
    pub font_size_mm: f64,
    /// Pen x within the line (mm), before horizontal align offset.
    pub x_mm: f64,
    /// Pen y offset from baseline (usually 0); artboard +y down.
    pub y_mm: f64,
}

pub struct LayoutLine {
    pub glyphs: Vec<PlacedGlyph>,
    pub width_mm: f64,
    pub height_mm: f64,
    pub ascender_mm: f64,
}

enum Token {
    Word(WordCluster),
    Newline {
        font_slug: String,
        font_size_mm: f64,
    },
}

struct WordCluster {
    font_slug: String,
    font_size_mm: f64,
    text: String,
    width_mm: f64,
}

/// Greedy word-wrap runs into lines that fit `layout.width_mm`.
pub fn layout_lines(
    runs: &[TextRun],
    layout: &TextLayout,
    faces: &HashMap<String, Vec<u8>>,
) -> Result<Vec<LayoutLine>, CutOutlineError> {
    let tokens = flatten_tokens(runs, faces)?;
    if tokens.is_empty() {
        return Err(CutOutlineError::msg("no text to outline"));
    }

    let mut lines: Vec<LayoutLine> = Vec::new();
    let mut current: Vec<WordCluster> = Vec::new();
    let mut current_width = 0.0_f64;

    for token in tokens {
        match token {
            Token::Newline {
                font_slug,
                font_size_mm,
            } => {
                if current.is_empty() {
                    let (asc, height) = face_metrics(faces, &font_slug, font_size_mm)?;
                    lines.push(LayoutLine {
                        glyphs: Vec::new(),
                        width_mm: 0.0,
                        height_mm: height,
                        ascender_mm: asc,
                    });
                } else {
                    lines.push(clusters_to_line(&current, faces)?);
                    current.clear();
                    current_width = 0.0;
                }
            }
            Token::Word(word) => {
                let gap = if current.is_empty() {
                    0.0
                } else {
                    space_width_mm(faces, &word.font_slug, word.font_size_mm)?
                };
                let next_w = current_width + gap + word.width_mm;
                if !current.is_empty() && next_w > layout.width_mm && word.width_mm > 0.0 {
                    lines.push(clusters_to_line(&current, faces)?);
                    current.clear();
                    current.push(word);
                    current_width = current[0].width_mm;
                } else {
                    current_width = if current.is_empty() {
                        word.width_mm
                    } else {
                        next_w
                    };
                    current.push(word);
                }
            }
        }
    }
    if !current.is_empty() {
        lines.push(clusters_to_line(&current, faces)?);
    }
    if lines.is_empty() {
        return Err(CutOutlineError::msg("no text to outline"));
    }
    Ok(lines)
}

fn flatten_tokens(
    runs: &[TextRun],
    faces: &HashMap<String, Vec<u8>>,
) -> Result<Vec<Token>, CutOutlineError> {
    let mut out = Vec::new();
    for run in runs {
        let slug = run.font_slug.trim();
        if slug.is_empty() {
            return Err(CutOutlineError::msg("text run has empty font_slug"));
        }
        if !faces.contains_key(slug) {
            return Err(CutOutlineError::msg(format!("missing font face '{slug}'")));
        }
        if !run.font_size_mm.is_finite() || run.font_size_mm <= 0.0 {
            return Err(CutOutlineError::msg("font_size_mm must be positive"));
        }
        for piece in split_keep_newlines(&run.text) {
            if piece == "\n" {
                out.push(Token::Newline {
                    font_slug: slug.to_string(),
                    font_size_mm: run.font_size_mm,
                });
                continue;
            }
            for token in split_words(piece) {
                if token.is_empty() {
                    continue;
                }
                let width = shape_width_mm(faces, slug, run.font_size_mm, token)?;
                out.push(Token::Word(WordCluster {
                    font_slug: slug.to_string(),
                    font_size_mm: run.font_size_mm,
                    text: token.to_string(),
                    width_mm: width,
                }));
            }
        }
    }
    Ok(out)
}

fn split_keep_newlines(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (i, ch) in s.char_indices() {
        if ch == '\n' {
            if start < i {
                parts.push(&s[start..i]);
            }
            parts.push("\n");
            start = i + ch.len_utf8();
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn split_words(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, ch) in s.char_indices() {
        if ch.is_whitespace() {
            if let Some(st) = start {
                out.push(&s[st..i]);
                start = None;
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

fn clusters_to_line(
    clusters: &[WordCluster],
    faces: &HashMap<String, Vec<u8>>,
) -> Result<LayoutLine, CutOutlineError> {
    let mut glyphs = Vec::new();
    let mut pen_x = 0.0_f64;
    let mut max_asc = 0.0_f64;
    let mut max_height = 0.0_f64;

    for (i, cluster) in clusters.iter().enumerate() {
        if i > 0 {
            pen_x += space_width_mm(faces, &cluster.font_slug, cluster.font_size_mm)?;
        }
        let (asc, height) = face_metrics(faces, &cluster.font_slug, cluster.font_size_mm)?;
        max_asc = max_asc.max(asc);
        max_height = max_height.max(height);

        for g in shape_glyphs(
            faces,
            &cluster.font_slug,
            cluster.font_size_mm,
            &cluster.text,
        )? {
            glyphs.push(PlacedGlyph {
                font_slug: cluster.font_slug.clone(),
                glyph_id: g.glyph_id,
                font_size_mm: cluster.font_size_mm,
                x_mm: pen_x + g.x_offset_mm,
                // Font y_offset is +up; artboard +y down.
                y_mm: -g.y_offset_mm,
            });
            pen_x += g.x_advance_mm;
        }
    }

    Ok(LayoutLine {
        glyphs,
        width_mm: pen_x,
        height_mm: max_height,
        ascender_mm: max_asc,
    })
}

struct ShapedGlyph {
    glyph_id: u16,
    x_advance_mm: f64,
    x_offset_mm: f64,
    y_offset_mm: f64,
}

fn with_faces<'a, R>(
    faces: &'a HashMap<String, Vec<u8>>,
    slug: &str,
    f: impl FnOnce(RbFace<'a>, ttf_parser::Face<'a>) -> Result<R, CutOutlineError>,
) -> Result<R, CutOutlineError> {
    let sfnt = faces
        .get(slug)
        .ok_or_else(|| CutOutlineError::msg(format!("missing font face '{slug}'")))?;
    let rb = RbFace::from_slice(sfnt, 0)
        .ok_or_else(|| CutOutlineError::msg(format!("failed to parse font face '{slug}'")))?;
    let tp = ttf_parser::Face::parse(sfnt, 0).map_err(|e| {
        CutOutlineError::msg(format!("failed to parse font outlines for '{slug}': {e}"))
    })?;
    f(rb, tp)
}

fn face_metrics(
    faces: &HashMap<String, Vec<u8>>,
    slug: &str,
    font_size_mm: f64,
) -> Result<(f64, f64), CutOutlineError> {
    with_faces(faces, slug, |_rb, tp| {
        let units = tp.units_per_em() as f64;
        if !units.is_finite() || units <= 0.0 {
            return Err(CutOutlineError::msg(format!(
                "font '{slug}' has invalid units_per_em"
            )));
        }
        let scale = font_size_mm / units;
        let asc = f64::from(tp.ascender()) * scale;
        let desc = f64::from(-tp.descender()) * scale;
        let gap = f64::from(tp.line_gap()) * scale;
        Ok((asc, asc + desc + gap))
    })
}

fn space_width_mm(
    faces: &HashMap<String, Vec<u8>>,
    slug: &str,
    font_size_mm: f64,
) -> Result<f64, CutOutlineError> {
    shape_width_mm(faces, slug, font_size_mm, " ")
}

fn shape_width_mm(
    faces: &HashMap<String, Vec<u8>>,
    slug: &str,
    font_size_mm: f64,
    text: &str,
) -> Result<f64, CutOutlineError> {
    let glyphs = shape_glyphs(faces, slug, font_size_mm, text)?;
    Ok(glyphs.iter().map(|g| g.x_advance_mm).sum())
}

fn shape_glyphs(
    faces: &HashMap<String, Vec<u8>>,
    slug: &str,
    font_size_mm: f64,
    text: &str,
) -> Result<Vec<ShapedGlyph>, CutOutlineError> {
    with_faces(faces, slug, |rb, tp| {
        let units = tp.units_per_em() as f64;
        let scale = font_size_mm / units;
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let glyph_buffer = rustybuzz::shape(&rb, &[], buffer);
        let infos = glyph_buffer.glyph_infos();
        let positions = glyph_buffer.glyph_positions();
        let mut out = Vec::with_capacity(infos.len());
        for (info, pos) in infos.iter().zip(positions.iter()) {
            let gid = info.glyph_id;
            if gid > u32::from(u16::MAX) {
                return Err(CutOutlineError::msg(format!(
                    "glyph id {gid} out of range in font '{slug}'"
                )));
            }
            // Missing glyph (usually .notdef = 0) — fail rather than cut empty boxes.
            if gid == 0 && !text.chars().all(|c| c.is_whitespace()) {
                // Some fonts map space to 0; only fail for non-whitespace clusters.
                let has_ink = text.chars().any(|c| !c.is_whitespace());
                if has_ink {
                    // Allow .notdef only when the shaped cluster is purely marks/space;
                    // for visible text, missing cmap is an error.
                    // Heuristic: if ALL glyphs are 0 for non-empty text, error.
                }
            }
            out.push(ShapedGlyph {
                glyph_id: gid as u16,
                x_advance_mm: f64::from(pos.x_advance) * scale,
                x_offset_mm: f64::from(pos.x_offset) * scale,
                y_offset_mm: f64::from(pos.y_offset) * scale,
            });
        }
        if !text.chars().any(|c| !c.is_whitespace()) {
            return Ok(out);
        }
        if out.iter().all(|g| g.glyph_id == 0) {
            return Err(CutOutlineError::msg(format!(
                "font '{slug}' has no glyphs for {text:?}"
            )));
        }
        Ok(out)
    })
}
