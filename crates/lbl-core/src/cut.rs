//! Vector cut jobs for craft cutters / plotters (parallel to raster [`JobSpec`]).

use serde::{Deserialize, Serialize};

/// A point in millimeters on the artboard (origin top-left, +x right, +y down).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CutPointMm {
    pub x_mm: f64,
    pub y_mm: f64,
}

/// A polyline cut path in artboard millimeters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutPath {
    /// Vertices in order. Closed paths should repeat the first point as last,
    /// or set [`closed`](Self::closed).
    pub points: Vec<CutPointMm>,
    /// When true, the encoder closes the path back to the first point.
    #[serde(default)]
    pub closed: bool,
}

impl CutPath {
    /// Axis-aligned rectangle as a closed path (top-left origin artboard).
    pub fn rect_mm(x_mm: f64, y_mm: f64, width_mm: f64, height_mm: f64) -> Self {
        Self {
            points: vec![
                CutPointMm { x_mm, y_mm },
                CutPointMm {
                    x_mm: x_mm + width_mm,
                    y_mm,
                },
                CutPointMm {
                    x_mm: x_mm + width_mm,
                    y_mm: y_mm + height_mm,
                },
                CutPointMm {
                    x_mm,
                    y_mm: y_mm + height_mm,
                },
            ],
            closed: true,
        }
    }
}

/// Silhouette / Graphtec cut options (MVP subset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilhouetteOptions {
    /// Feed speed `!` (typically 1–10).
    #[serde(default = "default_speed")]
    pub speed: u8,
    /// Downward force `FX` (model-dependent ceiling).
    #[serde(default = "default_force")]
    pub force: u8,
    /// Cutting mat preset for `TG` (0 = none, 1 = 12×12, …).
    #[serde(default)]
    pub mat: u8,
    /// Blade cutter offset `FC` (≈18 for blade, 0 for pen).
    #[serde(default = "default_tool_offset")]
    pub tool_offset: u16,
    /// Landscape (`true`) vs portrait for `FN`.
    #[serde(default)]
    pub landscape: bool,
}

fn default_speed() -> u8 {
    5
}
fn default_force() -> u8 {
    10
}
fn default_tool_offset() -> u16 {
    18
}

impl Default for SilhouetteOptions {
    fn default() -> Self {
        Self {
            speed: default_speed(),
            force: default_force(),
            mat: 1,
            tool_offset: default_tool_offset(),
            landscape: false,
        }
    }
}

/// Specification for a cut-only job (no raster bitmap).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutJobSpec {
    /// Cuttable width in millimeters.
    pub width_mm: f64,
    /// Cuttable height / length in millimeters.
    pub height_mm: f64,
    /// How many times to cut the path set.
    #[serde(default = "default_copies")]
    pub copies: u32,
    /// Catalog device key (model-specific limits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_key: Option<String>,
    /// Silhouette / GPGL options.
    #[serde(default)]
    pub silhouette: SilhouetteOptions,
}

fn default_copies() -> u32 {
    1
}

impl Default for CutJobSpec {
    fn default() -> Self {
        Self {
            width_mm: 304.8,
            height_mm: 304.8,
            copies: 1,
            device_key: None,
            silhouette: SilhouetteOptions::default(),
        }
    }
}
