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

/// Loaded craft-cutter tool (drives Autoblade depth and cutter offset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SilhouetteTool {
    /// Autoblade — depth is set by the machine via `TF`.
    #[default]
    Autoblade,
    /// Manual ratchet blade — depth is advisory (user dials the blade).
    Ratchet,
    /// Deep-cut blade (manual depth).
    DeepCut,
    /// Pen / sketch — cutter offset 0.
    Pen,
}

/// Silhouette / Graphtec cut options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SilhouetteOptions {
    /// Feed speed `!` (Cameo-class: 1–30).
    #[serde(default = "default_speed")]
    pub speed: u8,
    /// Downward force `FX` (typically 1–33).
    #[serde(default = "default_force")]
    pub force: u8,
    /// Cutting mat preset for `TG` (0 = none, 1 = 12×12, 2 = 12×24, 8 = 15×15, 9 = 24×24).
    #[serde(default = "default_mat")]
    pub mat: u8,
    /// Blade cutter offset `FC` (≈18 for blade, 0 for pen). Ignored when [`tool`](Self::tool) is [`SilhouetteTool::Pen`] (forced to 0).
    #[serde(default = "default_tool_offset")]
    pub tool_offset: u16,
    /// Autoblade / recommended blade depth (`TF` when tool is Autoblade), 1–10.
    #[serde(default = "default_depth")]
    pub depth: u8,
    /// How many times to cut each path (Studio "Passes" / double cut). No GPGL opcode — paths are re-emitted.
    #[serde(default = "default_passes")]
    pub passes: u8,
    /// Tool type (Autoblade / ratchet / deep-cut / pen).
    #[serde(default)]
    pub tool: SilhouetteTool,
    /// Carriage tool holder `J` (1 or 2 on dual-carriage Cameo).
    #[serde(default = "default_tool_holder")]
    pub tool_holder: u8,
    /// Track enhancing (`FY0` on / `FY1` off).
    #[serde(default)]
    pub track_enhance: bool,
    /// Acceleration preset `TJ` (0–3 typical; Studio often emits `TJ0` then `TJ3`).
    #[serde(default = "default_acceleration")]
    pub acceleration: u8,
    /// Enable line-segment overcut / corner sharpen (`FE` / `FF`).
    #[serde(default)]
    pub overcut_enabled: bool,
    /// Overcut start extension in millimeters (encoded as 0.1 mm units in `FF`).
    #[serde(default = "default_overcut_mm")]
    pub overcut_start_mm: f64,
    /// Overcut end extension in millimeters.
    #[serde(default = "default_overcut_mm")]
    pub overcut_end_mm: f64,
}

fn default_speed() -> u8 {
    5
}
fn default_force() -> u8 {
    10
}
fn default_mat() -> u8 {
    1
}
fn default_tool_offset() -> u16 {
    18
}
fn default_depth() -> u8 {
    1
}
fn default_passes() -> u8 {
    1
}
fn default_tool_holder() -> u8 {
    1
}
fn default_acceleration() -> u8 {
    3
}
fn default_overcut_mm() -> f64 {
    0.1
}

impl SilhouetteOptions {
    /// Effective `FC` offset: pens always use 0.
    pub fn effective_tool_offset(&self) -> u16 {
        match self.tool {
            SilhouetteTool::Pen => 0,
            _ => self.tool_offset,
        }
    }

    /// Whether the encoder should emit Autoblade depth (`TF`).
    pub fn emits_autoblade_depth(&self) -> bool {
        matches!(self.tool, SilhouetteTool::Autoblade)
    }
}

impl Default for SilhouetteOptions {
    fn default() -> Self {
        Self {
            speed: default_speed(),
            force: default_force(),
            mat: default_mat(),
            tool_offset: default_tool_offset(),
            depth: default_depth(),
            passes: default_passes(),
            tool: SilhouetteTool::default(),
            tool_holder: default_tool_holder(),
            track_enhance: false,
            acceleration: default_acceleration(),
            overcut_enabled: false,
            overcut_start_mm: default_overcut_mm(),
            overcut_end_mm: default_overcut_mm(),
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
