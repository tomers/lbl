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

/// Twin Turbo (and similar dual-bay) roll selection (`ESC q`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LwRollSelect {
    /// Firmware toggles between rolls (`ESC q '0'`).
    #[default]
    Auto,
    /// Left roll (`ESC q '1'`).
    Left,
    /// Right roll (`ESC q '2'`).
    Right,
}

impl LwRollSelect {
    /// Parse a CLI/API-friendly name (`auto`, `left`, `right`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "auto" | "automatic" | "0" => Some(Self::Auto),
            "left" | "l" | "1" => Some(Self::Left),
            "right" | "r" | "2" => Some(Self::Right),
            _ => None,
        }
    }

    /// Wire byte for classic LabelWriter `ESC q`.
    pub fn wire_byte(self) -> u8 {
        match self {
            Self::Auto => b'0',
            Self::Left => b'1',
            Self::Right => b'2',
        }
    }
}

/// DYMO LabelWriter protocol options (550-series + classic 450-series).
///
/// LW550 reads `output_mode` / `speed` and ignores `roll`. Classic LW reads
/// `output_mode` / `roll` and ignores `speed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DymoLwOptions {
    /// Text vs graphics engine mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<LwOutputMode>,
    /// Feed speed. High speed is chassis- and roll-dependent (LW550).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<LwSpeed>,
    /// Twin Turbo roll select (classic LW `ESC q`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roll: Option<LwRollSelect>,
}

impl DymoLwOptions {
    /// Whether all fields are unset.
    pub fn is_empty(&self) -> bool {
        self.output_mode.is_none() && self.speed.is_none() && self.roll.is_none()
    }
}

/// Protocol-specific options carried with a job.
///
/// Shared parameters (copies, cut, density) stay on [`JobSpec`]. Each driver
/// reads only its own bag and ignores the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DriverOptions {
    /// DYMO LabelWriter 550-series options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dymo: Option<DymoLwOptions>,
}

impl DriverOptions {
    /// Whether no protocol bag carries any value.
    pub fn is_empty(&self) -> bool {
        self.dymo.as_ref().is_none_or(DymoLwOptions::is_empty)
    }

    /// Build options with a DYMO LW bag when any field is set.
    pub fn from_dymo(output_mode: Option<LwOutputMode>, speed: Option<LwSpeed>) -> Self {
        Self::from_dymo_full(output_mode, speed, None)
    }

    /// Build options with the full DYMO LW bag when any field is set.
    pub fn from_dymo_full(
        output_mode: Option<LwOutputMode>,
        speed: Option<LwSpeed>,
        roll: Option<LwRollSelect>,
    ) -> Self {
        let opts = DymoLwOptions {
            output_mode,
            speed,
            roll,
        };
        if opts.is_empty() {
            return Self::default();
        }
        Self { dymo: Some(opts) }
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
    /// 0-based index of this label within a multi-label batch encode.
    ///
    /// Used by protocols that share one job stream across labels (Brother PT/QL
    /// raster): only index 0 emits invalidate/init, and only the last label
    /// terminates with print-and-feed (`0x1A`) / no-chain. Independent full
    /// jobs per label force a head-to-cutter leader scrap before each print.
    #[serde(default)]
    pub batch_index: u32,
    /// Total labels in the batch encode (`1` = standalone job).
    #[serde(default = "one")]
    pub batch_total: u32,
    /// Optional print density / heat level (driver-specific).
    ///
    /// Typical ranges: NIIMBOT 1–5, DYMO LabelWriter percent 1–200. When
    /// omitted, each driver uses its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<u8>,
    /// Protocol-specific options (each driver reads only its own bag).
    #[serde(default, skip_serializing_if = "DriverOptions::is_empty")]
    pub driver: DriverOptions,
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
            batch_index: 0,
            batch_total: 1,
            density: None,
            driver: DriverOptions::default(),
        }
    }

    /// Effective batch size (at least 1).
    pub fn batch_total(&self) -> u32 {
        self.batch_total.max(1)
    }

    /// Clamp [`Self::batch_index`] into `0..batch_total`.
    pub fn batch_index(&self) -> u32 {
        self.batch_index.min(self.batch_total().saturating_sub(1))
    }

    /// Whether this encode is the first page of a multi-label batch (or a
    /// standalone job).
    pub fn batch_first(&self) -> bool {
        self.batch_index() == 0
    }

    /// Whether this encode is the last page of a multi-label batch (or a
    /// standalone job).
    pub fn batch_last(&self) -> bool {
        self.batch_index() + 1 >= self.batch_total()
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

    #[test]
    fn driver_options_from_dymo_omits_empty() {
        assert!(DriverOptions::from_dymo(None, None).is_empty());
        assert!(!DriverOptions::from_dymo(Some(LwOutputMode::Graphics), None).is_empty());
    }
}
