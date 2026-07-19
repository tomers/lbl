//! Minimal SVG → [`CutPath`] import for cut-ready clipart.

use lbl_core::{CutPath, CutPointMm};

use crate::GpglError;

/// Errors specific to SVG import.
#[derive(Debug, thiserror::Error)]
pub enum SvgCutError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Gpgl(#[from] GpglError),
}

/// Parse a subset of SVG into cut paths.
///
/// Supported: `rect`, `circle`, `ellipse`, `polygon`, `polyline`, and `path`
/// with `M`/`L`/`H`/`V`/`Z` (absolute and relative). Groups apply nested
/// `transform="translate(tx ty)"` and `transform="scale(sx[ sy])"` only.
///
/// Fails when the document has no geometry, or only unsupported elements
/// (`text`, `image`, …).
pub fn cut_paths_from_svg(svg: &str) -> Result<Vec<CutPath>, SvgCutError> {
    let mut paths = Vec::new();
    extract_elements(svg, &Transform::identity(), &mut paths)?;
    if paths.is_empty() {
        return Err(SvgCutError::Message(
            "SVG has no cuttable geometry (need path/rect/circle/polygon; text and images are not outlined)"
                .into(),
        ));
    }
    Ok(paths)
}

#[derive(Clone, Copy)]
struct Transform {
    tx: f64,
    ty: f64,
    sx: f64,
    sy: f64,
}

impl Transform {
    fn identity() -> Self {
        Self {
            tx: 0.0,
            ty: 0.0,
            sx: 1.0,
            sy: 1.0,
        }
    }

    fn apply(self, x: f64, y: f64) -> CutPointMm {
        CutPointMm {
            x_mm: x * self.sx + self.tx,
            y_mm: y * self.sy + self.ty,
        }
    }

    fn then(self, other: Transform) -> Self {
        Self {
            tx: self.tx + other.tx * self.sx,
            ty: self.ty + other.ty * self.sy,
            sx: self.sx * other.sx,
            sy: self.sy * other.sy,
        }
    }
}

fn extract_elements(xml: &str, xf: &Transform, out: &mut Vec<CutPath>) -> Result<(), SvgCutError> {
    let lower = xml.to_ascii_lowercase();
    // Very small tag scan — not a full XML parser.
    let mut i = 0;
    while let Some(rel) = lower[i..].find('<') {
        let start = i + rel;
        if lower[start..].starts_with("<!--") {
            if let Some(end) = lower[start..].find("-->") {
                i = start + end + 3;
                continue;
            }
            break;
        }
        let rest = &xml[start..];
        let rest_l = &lower[start..];
        let name_end = rest_l
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(1);
        let tag = &rest_l[1..name_end];
        let close = rest_l
            .find('>')
            .ok_or_else(|| SvgCutError::Message("malformed SVG tag".into()))?;
        let attrs = &rest[name_end..close];
        let self_close = rest_l[..close].ends_with('/');

        match tag {
            "rect" => {
                if let Some(p) = parse_rect(attrs, *xf) {
                    out.push(p);
                }
            }
            "circle" => {
                if let Some(p) = parse_circle(attrs, *xf) {
                    out.push(p);
                }
            }
            "ellipse" => {
                if let Some(p) = parse_ellipse(attrs, *xf) {
                    out.push(p);
                }
            }
            "polygon" | "polyline" => {
                let closed = tag == "polygon";
                if let Some(p) = parse_poly(attrs, *xf, closed) {
                    out.push(p);
                }
            }
            "path" => {
                if let Some(d) = attr(attrs, "d") {
                    out.extend(parse_path_d(&d, *xf)?);
                }
            }
            "g" if !self_close => {
                let child_xf = xf.then(parse_transform(attrs));
                let after = start + close + 1;
                if let Some(end_rel) = lower[after..].find("</g>") {
                    let inner = &xml[after..after + end_rel];
                    extract_elements(inner, &child_xf, out)?;
                    i = after + end_rel + 4;
                    continue;
                }
            }
            "text" | "image" | "use" => {
                // Explicitly unsupported for cut import.
            }
            _ => {}
        }
        i = start + close + 1;
    }
    Ok(())
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let pattern = format!("{name}=");
    let lower = attrs.to_ascii_lowercase();
    let idx = lower.find(&pattern)?;
    let rest = attrs[idx + pattern.len()..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)? + 1;
    Some(rest[1..end].to_string())
}

fn attr_f(attrs: &str, name: &str) -> Option<f64> {
    attr(attrs, name)?.parse().ok()
}

fn parse_transform(attrs: &str) -> Transform {
    let Some(t) = attr(attrs, "transform") else {
        return Transform::identity();
    };
    let mut xf = Transform::identity();
    if let Some(inner) = t
        .strip_prefix("translate(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<_> = inner
            .split(&[',', ' '][..])
            .filter(|s| !s.is_empty())
            .collect();
        if let Some(tx) = parts.first().and_then(|s| s.parse().ok()) {
            xf.tx = tx;
        }
        if let Some(ty) = parts.get(1).and_then(|s| s.parse().ok()) {
            xf.ty = ty;
        }
    } else if let Some(inner) = t.strip_prefix("scale(").and_then(|s| s.strip_suffix(')')) {
        let parts: Vec<_> = inner
            .split(&[',', ' '][..])
            .filter(|s| !s.is_empty())
            .collect();
        if let Some(sx) = parts.first().and_then(|s| s.parse().ok()) {
            xf.sx = sx;
            xf.sy = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(sx);
        }
    }
    xf
}

fn parse_rect(attrs: &str, xf: Transform) -> Option<CutPath> {
    let x = attr_f(attrs, "x").unwrap_or(0.0);
    let y = attr_f(attrs, "y").unwrap_or(0.0);
    let w = attr_f(attrs, "width")?;
    let h = attr_f(attrs, "height")?;
    let mut p = CutPath::rect_mm(x, y, w, h);
    p.points = p
        .points
        .iter()
        .map(|pt| xf.apply(pt.x_mm, pt.y_mm))
        .collect();
    Some(p)
}

fn parse_circle(attrs: &str, xf: Transform) -> Option<CutPath> {
    let cx = attr_f(attrs, "cx")?;
    let cy = attr_f(attrs, "cy")?;
    let r = attr_f(attrs, "r")?;
    Some(ellipse_path(cx, cy, r, r, xf))
}

fn parse_ellipse(attrs: &str, xf: Transform) -> Option<CutPath> {
    let cx = attr_f(attrs, "cx")?;
    let cy = attr_f(attrs, "cy")?;
    let rx = attr_f(attrs, "rx")?;
    let ry = attr_f(attrs, "ry")?;
    Some(ellipse_path(cx, cy, rx, ry, xf))
}

fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64, xf: Transform) -> CutPath {
    const N: usize = 48;
    let mut points = Vec::with_capacity(N + 1);
    for i in 0..N {
        let a = std::f64::consts::TAU * (i as f64) / (N as f64);
        points.push(xf.apply(cx + rx * a.cos(), cy + ry * a.sin()));
    }
    CutPath {
        points,
        closed: true,
    }
}

fn parse_poly(attrs: &str, xf: Transform, closed: bool) -> Option<CutPath> {
    let pts = attr(attrs, "points")?;
    let nums: Vec<f64> = pts
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if nums.len() < 4 {
        return None;
    }
    let mut points = Vec::new();
    for chunk in nums.chunks(2) {
        if chunk.len() == 2 {
            points.push(xf.apply(chunk[0], chunk[1]));
        }
    }
    Some(CutPath { points, closed })
}

fn parse_path_d(d: &str, xf: Transform) -> Result<Vec<CutPath>, SvgCutError> {
    let mut paths = Vec::new();
    let mut cur = CutPointMm {
        x_mm: 0.0,
        y_mm: 0.0,
    };
    let mut start = cur;
    let mut pts: Vec<CutPointMm> = Vec::new();
    let tokens = tokenize_path(d);
    let mut i = 0;
    let mut cmd = 'M';
    while i < tokens.len() {
        let t = &tokens[i];
        if t.len() == 1 && t.chars().next().unwrap().is_alphabetic() {
            cmd = t.chars().next().unwrap();
            i += 1;
        }
        match cmd {
            'M' | 'm' => {
                flush_path(&mut paths, &mut pts, false);
                let rel = cmd == 'm';
                let x = num(&tokens, &mut i)?;
                let y = num(&tokens, &mut i)?;
                cur = if rel {
                    CutPointMm {
                        x_mm: cur.x_mm + x,
                        y_mm: cur.y_mm + y,
                    }
                } else {
                    CutPointMm { x_mm: x, y_mm: y }
                };
                start = cur;
                pts.push(xf.apply(cur.x_mm, cur.y_mm));
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' | 'l' => {
                let rel = cmd == 'l';
                let x = num(&tokens, &mut i)?;
                let y = num(&tokens, &mut i)?;
                cur = if rel {
                    CutPointMm {
                        x_mm: cur.x_mm + x,
                        y_mm: cur.y_mm + y,
                    }
                } else {
                    CutPointMm { x_mm: x, y_mm: y }
                };
                pts.push(xf.apply(cur.x_mm, cur.y_mm));
            }
            'H' | 'h' => {
                let rel = cmd == 'h';
                let x = num(&tokens, &mut i)?;
                cur.x_mm = if rel { cur.x_mm + x } else { x };
                pts.push(xf.apply(cur.x_mm, cur.y_mm));
            }
            'V' | 'v' => {
                let rel = cmd == 'v';
                let y = num(&tokens, &mut i)?;
                cur.y_mm = if rel { cur.y_mm + y } else { y };
                pts.push(xf.apply(cur.x_mm, cur.y_mm));
            }
            'Z' | 'z' => {
                flush_path(&mut paths, &mut pts, true);
                cur = start;
            }
            _ => {
                return Err(SvgCutError::Message(format!(
                    "unsupported SVG path command '{cmd}' (use M/L/H/V/Z)"
                )));
            }
        }
    }
    flush_path(&mut paths, &mut pts, false);
    Ok(paths)
}

fn flush_path(paths: &mut Vec<CutPath>, pts: &mut Vec<CutPointMm>, closed: bool) {
    if pts.len() >= 2 {
        paths.push(CutPath {
            points: std::mem::take(pts),
            closed,
        });
    } else {
        pts.clear();
    }
}

fn tokenize_path(d: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in d.chars() {
        if c.is_ascii_alphabetic() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else if c == ',' || c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn num(tokens: &[String], i: &mut usize) -> Result<f64, SvgCutError> {
    let s = tokens
        .get(*i)
        .ok_or_else(|| SvgCutError::Message("unexpected end of path data".into()))?;
    *i += 1;
    s.parse()
        .map_err(|_| SvgCutError::Message(format!("bad number '{s}'")))
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
        assert_eq!(paths[0].points.len(), 4);
    }

    #[test]
    fn text_only_fails() {
        let err = cut_paths_from_svg(r#"<svg><text>hi</text></svg>"#).unwrap_err();
        assert!(err.to_string().contains("no cuttable"));
    }
}
