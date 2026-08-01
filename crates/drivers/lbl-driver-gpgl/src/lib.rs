//! GPGL cut encoder for Graphtec / Silhouette craft cutters.
//!
//! Parallel to raster drivers: consumes [`CutPath`]s and a [`CutJobSpec`], not a
//! bitmap.
//!
//! # Coordinate systems
//!
//! Artboard mm use origin top-left, +x right, +y down (canvas / SVG).
//! Cameo GPGL uses origin top-left, +x top→bottom (into the machine), +y
//! left→right (carriage). Mapping is an axis swap — device `(x, y) =
//! (artboard_y, artboard_x)` in 1/20 mm units — matching inkscape-silhouette's
//! `move_mm_cmd(y, x)`. Encode uses portrait (`FN0` / `TB50,0`); Cameo
//! landscape `FN` skips blade-align tick compensation.

pub mod svg;

use lbl_core::{CutJobSpec, CutPath, CutPointMm, SilhouetteOptions};

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
    // After axis swap, GPGL X is sheet height (feed) and Y is sheet width (carriage).
    let feed_u = mm_to_units(job.height_mm);
    let carriage_u = mm_to_units(job.width_mm);
    let copies = job.copies.max(1);
    let passes = opt.passes.max(1);
    let holder = opt.tool_holder.max(1);

    for _ in 0..copies {
        out.extend_from_slice(b"\x1b\x04");
        push_cmd(&mut out, "FN", &["0"]);
        // Portrait. Opcode ends in digits — must be `TB50,0`, not `TB500`
        // (push_cmd concatenates the first arg onto the mnemonic).
        push_portrait(&mut out);
        push_cmd(&mut out, "TG", &[&opt.mat.to_string()]);
        push_cmd(&mut out, "J", &[&holder.to_string()]);
        push_tool_cmd(&mut out, "FX", &opt.force.to_string(), holder);
        push_tool_cmd(&mut out, "!", &opt.speed.to_string(), holder);
        push_fc(&mut out, opt, holder);
        push_cmd(&mut out, "TJ", &["0"]);
        if opt.acceleration > 0 {
            push_cmd(&mut out, "TJ", &[&opt.acceleration.to_string()]);
        }
        if opt.emits_autoblade_depth() {
            let depth = opt.depth.clamp(1, 10);
            // Autoblade depth is only valid on tool holder 1.
            push_cmd(&mut out, "TF", &[&depth.to_string(), "1"]);
        }
        if opt.track_enhance {
            // FY0 = on (inverted sense in Graphtec docs).
            push_cmd(&mut out, "FY", &["0"]);
            push_cmd(&mut out, "FU", &[&format!("{feed_u:.0}")]);
        } else {
            push_cmd(&mut out, "FY", &["1"]);
        }
        push_overcut(&mut out, opt, holder);
        push_cmd(&mut out, "\\", &["0", "0"]);
        push_cmd(
            &mut out,
            "Z",
            &[&format!("{feed_u:.0}"), &format!("{carriage_u:.0}")],
        );

        for _ in 0..passes {
            for path in paths {
                encode_path(&mut out, path)?;
            }
        }

        push_cmd(&mut out, "L", &["0"]);
        push_cmd(&mut out, "\\", &["0", "0"]);
        push_cmd(&mut out, "M", &["0", "0"]);
        push_cmd(&mut out, "J", &["0"]);
        push_cmd(&mut out, "FN", &["0"]);
        push_portrait(&mut out);
    }

    Ok(out)
}

/// `TB50,0` — Cameo portrait. Not expressible via [`push_cmd`] alone because
/// the mnemonic already ends in digits (`TB50` + `0` would become `TB500`).
fn push_portrait(out: &mut Vec<u8>) {
    out.extend_from_slice(b"TB50,0");
    out.push(0x03);
}

fn push_tool_cmd(out: &mut Vec<u8>, op: &str, value: &str, holder: u8) {
    push_cmd(out, op, &[value, &holder.to_string()]);
}

/// Cameo 4 Studio form: `FC{offset},1,{holder}`.
fn push_fc(out: &mut Vec<u8>, opt: &SilhouetteOptions, holder: u8) {
    let offset = opt.effective_tool_offset();
    push_cmd(out, "FC", &[&offset.to_string(), "1", &holder.to_string()]);
}

fn push_overcut(out: &mut Vec<u8>, opt: &SilhouetteOptions, holder: u8) {
    let holder_s = holder.to_string();
    if opt.overcut_enabled {
        let start = mm_to_overcut_tenths(opt.overcut_start_mm);
        let end = mm_to_overcut_tenths(opt.overcut_end_mm);
        push_cmd(out, "FE", &["0", &holder_s]);
        push_cmd(
            out,
            "FF",
            &[&start.to_string(), &end.to_string(), &holder_s],
        );
    } else {
        push_cmd(out, "FE", &["0", &holder_s]);
        push_cmd(out, "FF", &["0", "0", &holder_s]);
    }
}

/// Studio / Graphtec `FF` extents are in 0.1 mm units.
fn mm_to_overcut_tenths(mm: f64) -> u32 {
    ((mm.max(0.0) * 10.0).round() as u32).min(99)
}

fn encode_path(out: &mut Vec<u8>, path: &CutPath) -> Result<(), GpglError> {
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

    let (x0, y0) = to_device(pts[0]);
    push_cmd(out, "M", &[&format!("{x0:.2}"), &format!("{y0:.2}")]);
    for p in pts.iter().skip(1) {
        let (x, y) = to_device(*p);
        push_cmd(out, "D", &[&format!("{x:.2}"), &format!("{y:.2}")]);
    }
    Ok(())
}

/// Artboard (x right, y down) → Cameo GPGL (x into machine, y across carriage).
fn to_device(p: CutPointMm) -> (f64, f64) {
    (mm_to_units(p.y_mm), mm_to_units(p.x_mm))
}

fn mm_to_units(mm: f64) -> f64 {
    mm * UNITS_PER_MM
}

/// Append `op` + args joined by `,` + ETX.
///
/// First arg is concatenated directly onto `op` (Graphtec style: `FN0`, `TG1`,
/// `FX10,1`). Mnemonics that already end in digits (e.g. `TB50`) must not use
/// this for a following arg — see [`push_portrait`].
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

/// GPGL `ESC E` reply bodies (status digit + ETX), as devices typically send.
pub const STATUS_REPLY_READY: &[u8] = b"0\x03";
pub const STATUS_REPLY_MOVING: &[u8] = b"1\x03";
pub const STATUS_REPLY_UNLOADED: &[u8] = b"2\x03";
pub const STATUS_REPLY_PAUSED: &[u8] = b"3\x03";
pub const STATUS_REPLY_CANCELLED: &[u8] = b"4\x03";

/// Firmware query (`FG`).
///
/// Reply is an ASCII string terminated by ETX, e.g. `"CAMEO V1.10 \x03"`.
pub fn firmware_query() -> Vec<u8> {
    let mut v = b"FG".to_vec();
    v.push(0x03);
    v
}

/// Device name query (`TI`); newer Silhouette firmware.
pub fn device_name_query() -> Vec<u8> {
    let mut v = b"TI".to_vec();
    v.push(0x03);
    v
}

/// Trim trailing ETX / whitespace from an ASCII identity reply (`FG` / `TI`).
pub fn parse_identity_reply(resp: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(resp)
        .ok()?
        .trim_end_matches('\x03')
        .trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Panel-key simulation prefix (`ESC` + `NUL` + mask). Documented in
/// inkscape-silhouette: down=`0x01`, up=`0x02`, right=`0x04`, left=`0x08`,
/// none=`0x00`.
pub const PANEL_KEY_NONE: u8 = 0x00;
pub const PANEL_KEY_DOWN: u8 = 0x01;
pub const PANEL_KEY_UP: u8 = 0x02;
pub const PANEL_KEY_RIGHT: u8 = 0x04;
pub const PANEL_KEY_LEFT: u8 = 0x08;

/// Simulate a front-panel key press (or release with [`PANEL_KEY_NONE`]).
pub fn panel_key(mask: u8) -> [u8; 3] {
    [0x1b, 0x00, mask]
}

/// Home the cutter head (`TT`).
pub fn home_cmd() -> Vec<u8> {
    let mut v = b"TT".to_vec();
    v.push(0x03);
    v
}

/// Feed media by `units` (device units = 1/20 mm) via `FO`.
pub fn feed_cmd(units: i32) -> Vec<u8> {
    let mut v = format!("FO{units}").into_bytes();
    v.push(0x03);
    v
}

/// Parse a status response body (without requiring the trailing `\x03`).
pub fn parse_status(resp: &[u8]) -> Option<GpglStatus> {
    let s = std::str::from_utf8(resp)
        .ok()?
        .trim_end_matches('\x03')
        .trim();
    match s.as_bytes().first()? {
        b'0' => Some(GpglStatus::Ready),
        b'1' => Some(GpglStatus::Moving),
        b'2' => Some(GpglStatus::Unloaded),
        // On-device Pause / Cancel (Cameo 3+ captures; see inkscape-silhouette #72).
        b'3' => Some(GpglStatus::Paused),
        b'4' => Some(GpglStatus::Cancelled),
        _ => None,
    }
}

/// Device motion / load status from `\x1b\x05`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpglStatus {
    Ready,
    Moving,
    Unloaded,
    /// Front-panel Pause is latched (`0x33` / ASCII `3`).
    Paused,
    /// Job cancelled on the device after pause (`0x34` / ASCII `4`).
    Cancelled,
}

impl GpglStatus {
    /// Wire reply body including trailing ETX (`STATUS_REPLY_*`).
    pub const fn reply(self) -> &'static [u8] {
        match self {
            Self::Ready => STATUS_REPLY_READY,
            Self::Moving => STATUS_REPLY_MOVING,
            Self::Unloaded => STATUS_REPLY_UNLOADED,
            Self::Paused => STATUS_REPLY_PAUSED,
            Self::Cancelled => STATUS_REPLY_CANCELLED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lbl_core::{SilhouetteOptions, SilhouetteTool};

    fn sample_job(width_mm: f64, height_mm: f64) -> CutJobSpec {
        CutJobSpec {
            width_mm,
            height_mm,
            copies: 1,
            device_key: Some("cameo4".into()),
            silhouette: SilhouetteOptions::default(),
        }
    }

    #[test]
    fn rect_emits_move_and_draw() {
        let path = CutPath::rect_mm(10.0, 20.0, 30.0, 40.0);
        let bytes = encode_cut(&[path], &sample_job(304.8, 304.8)).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("FX10,1\u{3}"));
        assert!(s.contains("!5,1\u{3}"));
        assert!(s.contains("FC18,1,1\u{3}"));
        assert!(s.contains("TG1\u{3}"));
        assert!(s.contains("J1\u{3}"));
        assert!(s.contains("TF1,1\u{3}"));
        assert!(s.contains("TJ0\u{3}"));
        assert!(s.contains("TJ3\u{3}"));
        assert!(s.contains("FY1\u{3}"));
        // Portrait must be `TB50,0` — `push_cmd("TB50", ["0"])` wrongly emits `TB500`.
        assert!(s.contains("TB50,0\u{3}"));
        assert!(!s.contains("TB500\u{3}"));
        assert_eq!(s.matches("TB50,0\u{3}").count(), 2);
        assert!(s.contains('M'));
        assert!(s.contains('D'));
        assert!(bytes.contains(&0x03));
    }

    #[test]
    fn autoblade_emits_tf_pen_skips_and_zero_offset() {
        let path = CutPath::rect_mm(0.0, 0.0, 10.0, 10.0);
        let mut job = sample_job(100.0, 100.0);
        job.silhouette.tool = SilhouetteTool::Autoblade;
        job.silhouette.depth = 4;
        let bytes = encode_cut(std::slice::from_ref(&path), &job).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("TF4,1\u{3}"));

        job.silhouette.tool = SilhouetteTool::Pen;
        job.silhouette.tool_offset = 18;
        let bytes = encode_cut(&[path], &job).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(!s.contains("TF"));
        assert!(s.contains("FC0,1,1\u{3}"));
    }

    #[test]
    fn passes_repeat_path_geometry() {
        let path = CutPath {
            points: vec![
                CutPointMm {
                    x_mm: 0.0,
                    y_mm: 0.0,
                },
                CutPointMm {
                    x_mm: 5.0,
                    y_mm: 0.0,
                },
            ],
            closed: false,
        };
        let mut job = sample_job(100.0, 100.0);
        job.silhouette.passes = 2;
        let bytes = encode_cut(&[path], &job).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert_eq!(s.matches("M0.00,0.00\u{3}").count(), 2);
        assert_eq!(s.matches("D0.00,100.00\u{3}").count(), 2);
    }

    #[test]
    fn track_enhance_and_overcut() {
        let path = CutPath::rect_mm(0.0, 0.0, 10.0, 10.0);
        let mut job = sample_job(304.8, 304.8);
        job.silhouette.track_enhance = true;
        job.silhouette.overcut_enabled = true;
        job.silhouette.overcut_start_mm = 0.5;
        job.silhouette.overcut_end_mm = 0.2;
        let bytes = encode_cut(&[path], &job).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("FY0\u{3}"));
        assert!(s.contains("FU6096\u{3}"));
        assert!(s.contains("FF5,2,1\u{3}"));
    }

    #[test]
    fn artboard_right_maps_to_device_y() {
        // Artboard (10, 0) = 10 mm right of origin → device y = 200 SU, x = 0.
        let path = CutPath {
            points: vec![
                CutPointMm {
                    x_mm: 0.0,
                    y_mm: 0.0,
                },
                CutPointMm {
                    x_mm: 10.0,
                    y_mm: 0.0,
                },
            ],
            closed: false,
        };
        let bytes = encode_cut(&[path], &sample_job(100.0, 100.0)).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("M0.00,0.00\u{3}"));
        assert!(s.contains("D0.00,200.00\u{3}"));
    }

    #[test]
    fn artboard_down_maps_to_device_x() {
        // Artboard (0, 10) = 10 mm down → device x = 200 SU, y = 0.
        let path = CutPath {
            points: vec![
                CutPointMm {
                    x_mm: 0.0,
                    y_mm: 0.0,
                },
                CutPointMm {
                    x_mm: 0.0,
                    y_mm: 10.0,
                },
            ],
            closed: false,
        };
        let bytes = encode_cut(&[path], &sample_job(100.0, 100.0)).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("M0.00,0.00\u{3}"));
        assert!(s.contains("D200.00,0.00\u{3}"));
    }

    #[test]
    fn workspace_z_is_feed_then_carriage() {
        // 12×24 mat: width 304.8 (carriage), height 609.6 (feed) → Z12192,6096.
        let path = CutPath {
            points: vec![
                CutPointMm {
                    x_mm: 0.0,
                    y_mm: 0.0,
                },
                CutPointMm {
                    x_mm: 1.0,
                    y_mm: 0.0,
                },
            ],
            closed: false,
        };
        let bytes = encode_cut(&[path], &sample_job(304.8, 609.6)).unwrap();
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("Z12192,6096\u{3}"));
        assert!(s.contains("FN0\u{3}"));
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
        assert_eq!(parse_status(STATUS_REPLY_READY), Some(GpglStatus::Ready));
        assert_eq!(parse_status(b"1"), Some(GpglStatus::Moving));
        assert_eq!(
            parse_status(STATUS_REPLY_UNLOADED),
            Some(GpglStatus::Unloaded)
        );
        assert_eq!(parse_status(STATUS_REPLY_PAUSED), Some(GpglStatus::Paused));
        assert_eq!(parse_status(b"4"), Some(GpglStatus::Cancelled));
        assert_eq!(GpglStatus::Ready.reply(), STATUS_REPLY_READY);
        assert_eq!(GpglStatus::Moving.reply(), STATUS_REPLY_MOVING);
    }

    #[test]
    fn panel_key_and_home_bytes() {
        assert_eq!(panel_key(PANEL_KEY_UP), [0x1b, 0x00, 0x02]);
        assert_eq!(panel_key(PANEL_KEY_NONE), [0x1b, 0x00, 0x00]);
        assert_eq!(home_cmd(), b"TT\x03".to_vec());
        assert_eq!(feed_cmd(100), b"FO100\x03".to_vec());
    }

    #[test]
    fn identity_query_bytes() {
        assert_eq!(firmware_query(), b"FG\x03".to_vec());
        assert_eq!(device_name_query(), b"TI\x03".to_vec());
    }

    #[test]
    fn parse_identity_reply_trims_etx_and_whitespace() {
        assert_eq!(
            parse_identity_reply(b"CAMEO V1.10    \x03").as_deref(),
            Some("CAMEO V1.10")
        );
        assert_eq!(
            parse_identity_reply(b"Silhouette Cameo 4\x03").as_deref(),
            Some("Silhouette Cameo 4")
        );
        assert_eq!(parse_identity_reply(b"\x03"), None);
        assert_eq!(parse_identity_reply(b""), None);
        assert_eq!(parse_identity_reply(&[0xff, 0xfe]), None);
    }
}
