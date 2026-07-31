//! SVG → [`CutPath`] import for cut-ready clipart.
//!
//! [`usvg`] resolves the document to absolute path geometry; this module
//! flattens curves and emits artboard-millimeter [`CutPath`]s.

use kurbo::{CubicBez, PathEl, Point as KurboPoint, QuadBez};
use lbl_core::{CutPath, CutPointMm};
use usvg::{Node, Options, Tree};

use crate::GpglError;

/// Errors specific to SVG import.
#[derive(Debug, thiserror::Error)]
pub enum SvgCutError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Gpgl(#[from] GpglError),
}

/// Flattening tolerance in artboard mm (kurbo adaptive subdivision).
const FLATTEN_TOLERANCE_MM: f64 = 0.25;

/// Parse SVG into cut paths.
///
/// Imports geometry `usvg` resolves to paths: `path`, basic shapes, nested
/// groups, transforms (`matrix` / `translate` / `scale`), `use`, and the full
/// path command set (curves and arcs). Text and raster images are not cut.
///
/// Output coordinates are artboard millimeters. Parser DPI is 25.4 so SVG
/// lengths in `mm` map 1:1; unitless user units are treated as mm.
pub fn cut_paths_from_svg(svg: &str) -> Result<Vec<CutPath>, SvgCutError> {
    let opt = Options {
        dpi: 25.4,
        ..Options::default()
    };

    let tree = Tree::from_str(svg, &opt).map_err(|e| SvgCutError::Message(e.to_string()))?;

    let mut paths = Vec::new();
    collect_group(tree.root(), &mut paths);

    if paths.is_empty() {
        return Err(SvgCutError::Message(
            "SVG has no cuttable geometry (need path/rect/circle/polygon; text and images are not outlined)"
                .into(),
        ));
    }
    Ok(paths)
}

fn collect_group(group: &usvg::Group, out: &mut Vec<CutPath>) {
    for node in group.children() {
        match node {
            Node::Group(g) => collect_group(g, out),
            Node::Path(p) => {
                if p.is_visible() {
                    out.extend(path_to_cut_paths(p));
                }
            }
            Node::Image(_) | Node::Text(_) => {}
        }
    }
}

fn path_to_cut_paths(path: &usvg::Path) -> Vec<CutPath> {
    let Some(transformed) = path.data().clone().transform(path.abs_transform()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut pts: Vec<CutPointMm> = Vec::new();
    let mut closed = false;
    let mut cur = KurboPoint::ZERO;

    let flush = |out: &mut Vec<CutPath>, pts: &mut Vec<CutPointMm>, closed: &mut bool| {
        if pts.len() >= 2 {
            out.push(CutPath {
                points: std::mem::take(pts),
                closed: *closed,
            });
        } else {
            pts.clear();
        }
        *closed = false;
    };

    for seg in transformed.segments() {
        match seg {
            usvg::tiny_skia_path::PathSegment::MoveTo(p) => {
                flush(&mut out, &mut pts, &mut closed);
                cur = KurboPoint::new(p.x as f64, p.y as f64);
                pts.push(cut_pt(cur));
            }
            usvg::tiny_skia_path::PathSegment::LineTo(p) => {
                cur = KurboPoint::new(p.x as f64, p.y as f64);
                pts.push(cut_pt(cur));
            }
            usvg::tiny_skia_path::PathSegment::QuadTo(p1, p) => {
                let end = KurboPoint::new(p.x as f64, p.y as f64);
                let quad = QuadBez::new(cur, KurboPoint::new(p1.x as f64, p1.y as f64), end);
                append_flattened(&mut pts, quad.raise());
                cur = end;
            }
            usvg::tiny_skia_path::PathSegment::CubicTo(p1, p2, p) => {
                let end = KurboPoint::new(p.x as f64, p.y as f64);
                let cubic = CubicBez::new(
                    cur,
                    KurboPoint::new(p1.x as f64, p1.y as f64),
                    KurboPoint::new(p2.x as f64, p2.y as f64),
                    end,
                );
                append_flattened(&mut pts, cubic);
                cur = end;
            }
            usvg::tiny_skia_path::PathSegment::Close => {
                closed = true;
                flush(&mut out, &mut pts, &mut closed);
            }
        }
    }
    flush(&mut out, &mut pts, &mut closed);
    out
}

fn append_flattened(pts: &mut Vec<CutPointMm>, cubic: CubicBez) {
    // `flatten` re-emits MoveTo(p0); the caller already recorded the start point.
    let mut first = true;
    kurbo::flatten(
        [
            PathEl::MoveTo(cubic.p0),
            PathEl::CurveTo(cubic.p1, cubic.p2, cubic.p3),
        ],
        FLATTEN_TOLERANCE_MM,
        |el| {
            if let PathEl::LineTo(p) = el {
                pts.push(cut_pt(p));
            } else if let PathEl::MoveTo(p) = el {
                if first {
                    first = false;
                } else {
                    pts.push(cut_pt(p));
                }
            }
        },
    );
}

fn cut_pt(p: KurboPoint) -> CutPointMm {
    CutPointMm {
        x_mm: p.x,
        y_mm: p.y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_svg() {
        let paths = cut_paths_from_svg(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="1" y="2" width="10" height="5"/></svg>"#,
        )
        .unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].closed);
        assert!(paths[0].points.len() >= 4);
    }

    #[test]
    fn text_only_fails() {
        let err = cut_paths_from_svg(r#"<svg><text>hi</text></svg>"#).unwrap_err();
        assert!(err.to_string().contains("no cuttable"));
    }

    #[test]
    fn path_with_id_attribute() {
        let paths =
            cut_paths_from_svg(r#"<svg><path id="path4396" d="M0 0 L10 0 L10 10 Z"/></svg>"#)
                .unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].closed);
        assert!(paths[0].points.len() >= 3);
    }

    #[test]
    fn implicit_number_separators() {
        let paths = cut_paths_from_svg(r#"<svg><path d="M0 0c-10-10 10-10 20 0"/></svg>"#).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].points.len() > 2);
    }

    #[test]
    fn matrix_and_smooth_cubic() {
        let svg = r#"
            <svg>
              <g transform="translate(10 20)">
                <g transform="matrix(2 0 0 2 0 0)">
                  <path d="M0 0 c10 0 10 10 0 10 s-10 0 0-10 z"/>
                </g>
              </g>
            </svg>
        "#;
        let paths = cut_paths_from_svg(svg).unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].closed);
        assert!(paths[0].points.len() > 8);
        assert!((paths[0].points[0].x_mm - 10.0).abs() < 1e-3);
        assert!((paths[0].points[0].y_mm - 20.0).abs() < 1e-3);
    }

    #[test]
    fn nested_matrix_and_cubic_paths() {
        let svg = r#"
<svg viewBox="0 0 1084.4 715.84">
  <g transform="translate(265.17 -128.96)">
    <g transform="matrix(1.4271 0 0 1.4271 -251.09 -363.51)">
      <path id="path4396" d="m473.37 749.77c-39.069-0.22252-86.101 0.82603-120.61-0.0355z"/>
      <path id="path4398" d="m-9.863 846.71c235.12-276.83 506.24 127.12 759.87-188.64-225.65 226.56-577.25-82.24-759.87 188.64z"/>
    </g>
  </g>
</svg>
"#;
        let paths = cut_paths_from_svg(svg).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|p| p.closed));
        assert!(paths.iter().all(|p| p.points.len() >= 2));
    }

    #[test]
    fn quadratic_and_arc() {
        let paths = cut_paths_from_svg(
            r#"<svg><path d="M0 0 Q10 20 20 0 T40 0 A10 10 0 0 1 50 10 Z"/></svg>"#,
        )
        .unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].closed);
        assert!(paths[0].points.len() > 10);
    }
}
