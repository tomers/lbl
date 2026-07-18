//! The job specification that flows through the pipeline.

use serde::{Deserialize, Serialize};

use crate::media::Media;

/// The target the transpiler/renderer produces output for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    /// Deterministic, exact-media output destined for the rasterizer/printer.
    #[default]
    Print,
    /// Screen-oriented output for browser/backend preview and the batch gallery.
    Preview,
}

/// When (if at all) the printer should cut during a job.
///
/// Matches the Brother P-touch cut matrix and maps sensibly onto other cutters:
/// cut after every label, only after the last label/copy, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CutMode {
    /// Do not cut.
    #[default]
    None,
    /// Cut after every label / copy.
    Every,
    /// Cut only after the last label / copy in the job.
    End,
}

impl CutMode {
    /// Parse a CLI/API-friendly name (`none`, `every`, `end`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" | "off" | "false" | "0" => Some(Self::None),
            "every" | "each" | "auto" | "true" | "1" => Some(Self::Every),
            "end" | "last" | "at-end" | "at_end" => Some(Self::End),
            _ => None,
        }
    }

    /// Whether any cut is requested.
    pub fn requests_cut(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether a cut should fire after copy `index` (0-based) of `copies`.
    pub fn should_cut_after_copy(self, index: u32, copies: u32) -> bool {
        let copies = copies.max(1);
        match self {
            Self::None => false,
            Self::Every => true,
            Self::End => index + 1 == copies,
        }
    }
}

/// DYMO LabelWriter 550-series text vs graphics engine mode (`ESC h` / `ESC i`).
///
/// Both modes use the same 300 dpi raster; graphics mode asks the engine for
/// settings tuned to barcodes/images (often slower). See the LW550 Tech Ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LwOutputMode {
    /// Text-oriented engine settings (`ESC h`). Default.
    #[default]
    Text,
    /// Barcode / graphics-oriented engine settings (`ESC i`).
    Graphics,
}

impl LwOutputMode {
    /// Parse a CLI/API-friendly name (`text`, `graphics`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "text" | "txt" => Some(Self::Text),
            "graphics" | "graphic" | "barcode" | "barcodes" => Some(Self::Graphics),
            _ => None,
        }
    }
}

/// DYMO LabelWriter 550-series feed speed (`ESC T`).
///
/// High speed is not available on the 5XL and is roll-dependent on 550 / Turbo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LwSpeed {
    /// Normal speed (`ESC T 0x10`). Default.
    #[default]
    Normal,
    /// High speed (`ESC T 0x20`).
    High,
}

impl LwSpeed {
    /// Parse a CLI/API-friendly name (`normal`, `high`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "normal" | "std" | "standard" => Some(Self::Normal),
            "high" | "fast" => Some(Self::High),
            _ => None,
        }
    }
}

/// A single print job: the media to print on and whether to cut afterward.
///
/// The HTML/raster content is carried alongside this spec by each stage; this
/// struct captures the device-facing parameters that all stages agree on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobSpec {
    /// Resolved media for this job.
    pub media: Media,
    /// Output mode for rendering/transpilation.
    #[serde(default)]
    pub mode: OutputMode,
    /// When to cut (honored only if the printer supports cutting).
    #[serde(default)]
    pub cut_mode: CutMode,
    /// Number of copies.
    #[serde(default = "one")]
    pub copies: u32,
    /// Optional print density / heat level (driver-specific).
    ///
    /// Typical ranges: NIIMBOT 1–5, DYMO LabelWriter percent 1–200. When
    /// omitted, each driver uses its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<u8>,
    /// DYMO LW550 text vs graphics engine mode. Ignored by other protocols.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lw_output_mode: Option<LwOutputMode>,
    /// DYMO LW550 feed speed. Ignored by other protocols / unsupported chassis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lw_speed: Option<LwSpeed>,
}

fn one() -> u32 {
    1
}

impl JobSpec {
    /// Create a print job for the given media with sensible defaults.
    pub fn new(media: Media) -> Self {
        Self {
            media,
            mode: OutputMode::Print,
            cut_mode: CutMode::None,
            copies: 1,
            density: None,
            lw_output_mode: None,
            lw_speed: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_mode_parse_aliases() {
        assert_eq!(CutMode::parse("none"), Some(CutMode::None));
        assert_eq!(CutMode::parse("every"), Some(CutMode::Every));
        assert_eq!(CutMode::parse("end"), Some(CutMode::End));
        assert_eq!(CutMode::parse("at-end"), Some(CutMode::End));
        assert_eq!(CutMode::parse("true"), Some(CutMode::Every));
        assert_eq!(CutMode::parse("bogus"), None);
    }

    #[test]
    fn cut_mode_after_copy() {
        assert!(!CutMode::None.should_cut_after_copy(0, 3));
        assert!(CutMode::Every.should_cut_after_copy(0, 3));
        assert!(CutMode::Every.should_cut_after_copy(2, 3));
        assert!(!CutMode::End.should_cut_after_copy(0, 3));
        assert!(CutMode::End.should_cut_after_copy(2, 3));
    }

    #[test]
    fn lw_output_mode_parse() {
        assert_eq!(LwOutputMode::parse("text"), Some(LwOutputMode::Text));
        assert_eq!(
            LwOutputMode::parse("graphics"),
            Some(LwOutputMode::Graphics)
        );
        assert_eq!(LwOutputMode::parse("barcode"), Some(LwOutputMode::Graphics));
        assert_eq!(LwOutputMode::parse("bogus"), None);
    }

    #[test]
    fn lw_speed_parse() {
        assert_eq!(LwSpeed::parse("normal"), Some(LwSpeed::Normal));
        assert_eq!(LwSpeed::parse("high"), Some(LwSpeed::High));
        assert_eq!(LwSpeed::parse("fast"), Some(LwSpeed::High));
        assert_eq!(LwSpeed::parse("bogus"), None);
    }
}
