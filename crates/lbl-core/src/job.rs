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
    /// Request a cut after this job (honored only if the printer supports it).
    #[serde(default)]
    pub cut: bool,
    /// Number of copies.
    #[serde(default = "one")]
    pub copies: u32,
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
            cut: false,
            copies: 1,
        }
    }
}
