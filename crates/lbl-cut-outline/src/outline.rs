//! Glyph contour → closed [`CutPath`] list.

use lbl_core::{CutPath, CutPointMm};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::curve::{cubic_point, quad_point, CURVE_SEGMENTS};
use crate::CutOutlineError;

/// Outline one glyph into piece-local mm paths.
///
/// Font space: +y up. Artboard: +y down. `origin` is the glyph pen on the
/// baseline in artboard mm; `scale` is mm per font unit.
pub fn glyph_to_cut_paths(
    face: &Face<'_>,
    glyph_id: u16,
    origin: CutPointMm,
    scale: f64,
) -> Result<Vec<CutPath>, CutOutlineError> {
    let mut builder = PathBuilder {
        origin,
        scale,
        contours: Vec::new(),
        current: Vec::new(),
        last: (0.0, 0.0),
        started: false,
    };
    let gid = GlyphId(glyph_id);
    match face.outline_glyph(gid, &mut builder) {
        Some(_) => Ok(builder.finish()),
        None => {
            // Space / mark with no outline — skip silently.
            Ok(Vec::new())
        }
    }
}

struct PathBuilder {
    origin: CutPointMm,
    scale: f64,
    contours: Vec<CutPath>,
    current: Vec<CutPointMm>,
    last: (f64, f64),
    started: bool,
}

impl PathBuilder {
    fn map_point(&self, x: f32, y: f32) -> CutPointMm {
        // Font +y up → artboard +y down.
        CutPointMm {
            x_mm: self.origin.x_mm + f64::from(x) * self.scale,
            y_mm: self.origin.y_mm - f64::from(y) * self.scale,
        }
    }

    fn push(&mut self, p: CutPointMm) {
        self.current.push(p);
    }

    fn finish(mut self) -> Vec<CutPath> {
        if self.started && self.current.len() >= 2 {
            self.contours.push(CutPath {
                points: std::mem::take(&mut self.current),
                closed: true,
            });
        }
        self.contours
            .into_iter()
            .filter(|p| p.points.len() >= 3)
            .collect()
    }
}

impl OutlineBuilder for PathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        if self.started && self.current.len() >= 2 {
            self.contours.push(CutPath {
                points: std::mem::take(&mut self.current),
                closed: true,
            });
        }
        self.current.clear();
        let p = self.map_point(x, y);
        self.last = (f64::from(x), f64::from(y));
        self.push(p);
        self.started = true;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.map_point(x, y);
        self.last = (f64::from(x), f64::from(y));
        self.push(p);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x0, y0) = self.last;
        let x1 = f64::from(x1);
        let y1 = f64::from(y1);
        let x = f64::from(x);
        let y = f64::from(y);
        for i in 1..=CURVE_SEGMENTS {
            let t = i as f64 / CURVE_SEGMENTS as f64;
            let (px, py) = quad_point((x0, y0), (x1, y1), (x, y), t);
            self.push(CutPointMm {
                x_mm: self.origin.x_mm + px * self.scale,
                y_mm: self.origin.y_mm - py * self.scale,
            });
        }
        self.last = (x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x0, y0) = self.last;
        let x1 = f64::from(x1);
        let y1 = f64::from(y1);
        let x2 = f64::from(x2);
        let y2 = f64::from(y2);
        let x = f64::from(x);
        let y = f64::from(y);
        for i in 1..=CURVE_SEGMENTS {
            let t = i as f64 / CURVE_SEGMENTS as f64;
            let (px, py) = cubic_point((x0, y0), (x1, y1), (x2, y2), (x, y), t);
            self.push(CutPointMm {
                x_mm: self.origin.x_mm + px * self.scale,
                y_mm: self.origin.y_mm - py * self.scale,
            });
        }
        self.last = (x, y);
    }

    fn close(&mut self) {
        // Contour closed via CutPath.closed; optional repeat of first point omitted.
    }
}
