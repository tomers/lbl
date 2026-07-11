//! The driver plugin contract for `lbl`.
//!
//! A [`Driver`] turns a dithered [`MonoBitmap`] into the byte stream a specific
//! printer protocol expects. Proprietary protocols (DYMO) and industry
//! standards (ESC/POS, ZPL, TSPL) implement the same trait, so the rest of the
//! toolchain treats them uniformly. `lbl-encode` selects a driver by
//! [`Protocol`] and calls [`Driver::encode`].
//!
//! Drivers live under `crates/drivers/` and are intentionally small and
//! self-contained so adding a new one is an isolated drop-in.

pub use lbl_core::bitmap::MonoBitmap;
pub use lbl_core::job::{CutMode, JobSpec};
pub use lbl_core::printer::{PrinterCapabilities, Protocol};

/// Errors a driver can produce while encoding.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// The bitmap or job is not supported by this driver/printer (e.g. too
    /// wide, unsupported feature).
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// Encoding failed for another reason.
    #[error("encode error: {0}")]
    Encode(String),
}

/// Everything a driver needs about the job and target printer to encode.
#[derive(Debug, Clone, Copy)]
pub struct EncodeContext<'a> {
    /// The job specification (media, cut request, copies).
    pub job: &'a JobSpec,
    /// Capabilities of the target printer.
    pub capabilities: &'a PrinterCapabilities,
    /// Optional red (low-energy) plane for Brother QL two-color media.
    ///
    /// When [`JobSpec::media`] has `two_color` set, callers should supply a
    /// same-sized plane (empty is valid for black-only artwork on red/black
    /// tape). Ignored by mono drivers.
    pub red: Option<&'a MonoBitmap>,
}

impl<'a> EncodeContext<'a> {
    /// Create a new mono encode context.
    pub fn new(job: &'a JobSpec, capabilities: &'a PrinterCapabilities) -> Self {
        Self {
            job,
            capabilities,
            red: None,
        }
    }

    /// Attach a red plane for two-color Brother QL encoding.
    pub fn with_red(mut self, red: &'a MonoBitmap) -> Self {
        self.red = Some(red);
        self
    }

    /// Whether this job should use Brother QL two-color raster (`w` rows).
    pub fn two_color(&self) -> bool {
        self.job.media.two_color
    }

    /// Effective cut mode: requested by the job *and* supported by the printer.
    pub fn cut_mode(&self) -> CutMode {
        if self.capabilities.supports_cut {
            self.job.cut_mode
        } else {
            CutMode::None
        }
    }

    /// Whether any cut is requested and supported.
    pub fn should_cut(&self) -> bool {
        self.cut_mode().requests_cut()
    }

    /// Whether a cut should fire after copy `index` (0-based) of `copies`.
    pub fn should_cut_after_copy(&self, index: u32, copies: u32) -> bool {
        self.cut_mode().should_cut_after_copy(index, copies)
    }

    /// Number of copies (at least 1).
    pub fn copies(&self) -> u32 {
        self.job.copies.max(1)
    }
}

/// A printer protocol driver: encodes a bitmap into protocol bytes.
pub trait Driver: Send + Sync {
    /// The protocol this driver implements.
    fn protocol(&self) -> Protocol;

    /// A short, stable driver name (for diagnostics).
    fn name(&self) -> &'static str;

    /// Encode `bitmap` into the printer-native byte stream for `ctx`.
    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError>;
}
