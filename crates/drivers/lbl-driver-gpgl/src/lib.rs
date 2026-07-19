//! GPGL cut encoder for Graphtec / Silhouette craft cutters.
//!
//! Parallel to raster drivers: consumes [`CutPath`]s and a [`CutJobSpec`], not a
//! bitmap. Artboard mm (origin top-left, +y down) convert to GPGL units of
//! 1/20 mm with origin at lower-left (+y up).

pub mod svg;

use lbl_core::{CutJobSpec, CutPath, CutPointMm};

/// Errors from GPGL encoding.
#[derive(Debug, thiserror::Error)]
pub enum GpglError {
    /// No cut paths were supplied.
    #[error("cut job has no paths")]
    EmptyPaths,
    /// A path had fewer than two points.
    #[error("cut path needs at least two points")]
    DegeneratePath,
    /// Sheet dimensions were non-positive.
    #[error("cut sheet size must be positive")]
    InvalidSheet,
}

/// GPGL uses twentieths of a millimeter.
const UNITS_PER_MM: f64 = 20.0;

/// Encode cut paths to a GPGL byte stream (commands terminated by `\x03`).
pub fn encode_cut(paths: &[CutPath], job: &CutJobSpec) -> Result<Vec<u8>, GpglError> {
    if paths.is_empty() {
        return Err(GpglError::EmptyPaths);
    }
    if job.width_mm <= 0.0 || job.height_mm <= 0.0 {
        return Err(GpglError::InvalidSheet);
    }

    let mut out = Vec::new();
    let opt = &job.silhouette;
    let w_u = mm_to_units(job.width_mm);
    let h_u = mm_to_units(job.height_mm);
    let copies = job.copies.max(1);

    for _ in 0..copies {
        out.extend_from_slice(b"\x1b\x04");
        push_cmd(&mut out, "FN", &[if opt.landscape { "1" } else { "0" }]);
        push_cmd(&mut out, "TB50", &["0"]);
        push_cmd(&mut out, "TG", &[&opt.mat.to_string()]);
        push_cmd(&mut out, "FX", &[&opt.force.to_string()]);
        push_cmd(&mut out, "!", &[&opt.speed.to_string()]);
        push_cmd(&mut out, "FC", &[&opt.tool_offset.to_string()]);
        push_cmd(&mut out, "\\", &["0", "0"]);
        push_cmd(&mut out, "Z", &[&format!("{w_u:.0}"), &format!("{h_u:.0}")]);

        for path in paths {
            encode_path(&mut out, path, job.height_mm)?;
        }

        push_cmd(&mut out, "L", &["0"]);
        push_cmd(&mut out, "\\", &["0", "0"]);
        push_cmd(&mut out, "M", &["0", "0"]);
        push_cmd(&mut out, "FN", &["0"]);
        push_cmd(&mut out, "TB50", &["0"]);
    }

    Ok(out)
}

fn encode_path(out: &mut Vec<u8>, path: &CutPath, height_mm: f64) -> Result<(), GpglError> {
    if path.points.len() < 2 {
        return Err(GpglError::DegeneratePath);
    }
    let mut pts: Vec<CutPointMm> = path.points.clone();
    if path.closed {
        if let (Some(first), Some(last)) = (pts.first(), pts.last()) {
            if (first.x_mm - last.x_mm).abs() > 1e-6 || (first.y_mm - last.y_mm).abs() > 1e-6 {
                pts.push(*first);
            }
        }
    }

    let (x0, y0) = to_device(pts[0], height_mm);
    push_cmd(out, "M", &[&format!("{x0:.2}"), &format!("{y0:.2}")]);
    for p in pts.iter().skip(1) {
        let (x, y) = to_device(*p, height_mm);
        push_cmd(out, "D", &[&format!("{x:.2}"), &format!("{y:.2}")]);
    }
    Ok(())
}

fn to_device(p: CutPointMm, height_mm: f64) -> (f64, f64) {
    let x = mm_to_units(p.x_mm);
    let y = mm_to_units(height_mm - p.y_mm);
    (x, y)
}

fn mm_to_units(mm: f64) -> f64 {
    mm * UNITS_PER_MM
}

fn push_cmd(out: &mut Vec<u8>, op: &str, args: &[&str]) {
    out.extend_from_slice(op.as_bytes());
    if !args.is_empty() {
        out.extend_from_slice(args.join(",").as_bytes());
    }
    out.push(0x03);
}

/// Status enquiry bytes (`ESC E`).
pub const STATUS_QUERY: &[u8] = b"\x1b\x05";

/// Initialize device (`ESC D` / EOT).
pub const INIT_CMD: &[u8] = b"\x1b\x04";

/// Firmware query.
pub fn firmware_query() -> Vec<u8> {
    let mut v = b"FG".to_vec();
    v.push(0x03);
    v
}

/// Parse a status response body (without requiring the trailing `\x03`).
pub fn parse_status(resp: &[u8]) -> Option<GpglStatus> {
    let s = std::str::from_utf8(resp)
        .ok()?
        .trim_end_matches('\x03')
        .trim();
    match s.chars().next()? {
        '0' => Some(GpglStatus::Ready),
        '1' => Some(GpglStatus::Moving),
        '2' => Some(GpglStatus::Unloaded),
        _ => None,
    }
}

/// Device motion / load status from `\x1b\x05`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpglStatus {
    Ready,
    Moving,
    Unloaded,
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::SilhouetteOptions;

    #[test]
    fn rect_emits_move_and_draw() {
        let path = CutPath::rect_mm(10.0, 20.0, 30.0, 40.0);
        let job = CutJobSpec {
            width_mm: 304.8,
            height_mm: 304.8,
            copies: 1,
            device_key: Some("cameo4".into()),
            silhouette: SilhouetteOptions {
                speed: 5,
                force: 10,
                mat: 1,
                tool_offset: 18,
                landscape: false,
            },
        };
        let bytes = encode_cut(&[path], &job).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("FX10\u{3}"));
        assert!(s.contains("!5\u{3}"));
        assert!(s.contains("FC18\u{3}"));
        assert!(s.contains("TG1\u{3}"));
        assert!(s.contains('M'));
        assert!(s.contains('D'));
        assert!(bytes.contains(&0x03));
    }

    #[test]
    fn empty_paths_fail() {
        let job = CutJobSpec::default();
        assert!(matches!(encode_cut(&[], &job), Err(GpglError::EmptyPaths)));
    }

    #[test]
    fn unit_conversion() {
        assert!((mm_to_units(1.0) - 20.0).abs() < 1e-9);
        assert!((mm_to_units(304.8) - 6096.0).abs() < 1e-6);
    }

    #[test]
    fn parse_status_ready() {
        assert_eq!(parse_status(b"0\x03"), Some(GpglStatus::Ready));
        assert_eq!(parse_status(b"1"), Some(GpglStatus::Moving));
        assert_eq!(parse_status(b"2\x03"), Some(GpglStatus::Unloaded));
    }
}
