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

pub mod packbits;

pub use lbl_core::bitmap::MonoBitmap;
pub use lbl_core::job::{CutMode, JobSpec};
pub use lbl_core::printer::{DeviceCapabilities, Protocol};
pub use packbits::{compress as packbits_compress, is_blank_row};

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
    pub capabilities: &'a DeviceCapabilities,
    /// Optional secondary ink plane for dual-color media.
    ///
    /// The primary plane is the `bitmap` passed to [`Driver::encode`]. When
    /// [`JobSpec::media`] has [`two_color`](lbl_core::media::Media::two_color)
    /// set, callers should supply a same-sized secondary plane (an empty plane
    /// is valid when artwork uses only the primary ink). Mono drivers ignore
    /// this field.
    pub secondary: Option<&'a MonoBitmap>,
    /// Optional full-color PNG for inkjet / color graphic registration.
    ///
    /// When [`DeviceCapabilities::supports_color`] is set, the pipeline may
    /// attach the rendered label as PNG bytes. Drivers that only speak 1-bit
    /// rasters ignore this field and encode the mono `bitmap` instead.
    pub color_png: Option<&'a [u8]>,
}

impl<'a> EncodeContext<'a> {
    /// Create a new mono encode context.
    pub fn new(job: &'a JobSpec, capabilities: &'a DeviceCapabilities) -> Self {
        Self {
            job,
            capabilities,
            secondary: None,
            color_png: None,
        }
    }

    /// Attach a secondary ink plane for dual-color encoding.
    pub fn with_secondary(mut self, secondary: &'a MonoBitmap) -> Self {
        self.secondary = Some(secondary);
        self
    }

    /// Attach a full-color PNG for color graphic registration.
    pub fn with_color_png(mut self, png: &'a [u8]) -> Self {
        self.color_png = Some(png);
        self
    }

    /// Whether this job targets dual-color media (`Media::two_color`).
    pub fn two_color(&self) -> bool {
        self.job.media.two_color
    }

    /// Whether a color PNG is available and the printer supports color output.
    pub fn color(&self) -> bool {
        self.capabilities.supports_color && self.color_png.is_some()
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

/// How a bidirectional client should deliver encoded protocol bytes.
///
/// Server-side transports that already implement status pacing ignore this;
/// browser / client-print paths use it to pick a delivery strategy after encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClientHandshake {
    /// Write the byte stream; do not wait for printer status.
    #[default]
    FireAndForget,
    /// DYMO LabelManager D1 status-paced delivery.
    DymoD1,
    /// DYMO LabelWriter 550 `ESC A` handshake after each label.
    DymoLw,
    /// NIIMBOT packet status polling between chunks.
    NiimbotPoll,
    /// LetraTag GATT notify completion.
    LetraTagNotify,
    /// Graphtec / Silhouette GPGL cutter status-paced delivery.
    ///
    /// The cutter is a vector plotter, not a raster print head, so no builtin
    /// [`Driver`] reports this handshake; cut-delivery callers select it
    /// explicitly for a GPGL byte stream.
    Gpgl,
}

impl ClientHandshake {
    /// Stable wire / JSON id for client-print responses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FireAndForget => "fire_and_forget",
            Self::DymoD1 => "dymo_d1",
            Self::DymoLw => "dymo_lw",
            Self::NiimbotPoll => "niimbot_poll",
            Self::LetraTagNotify => "letratag_notify",
            Self::Gpgl => "gpgl",
        }
    }

    /// Parse a [`Self::as_str`] id back into a handshake (case-insensitive).
    pub fn from_id(id: &str) -> Option<Self> {
        match id.trim().to_ascii_lowercase().as_str() {
            "fire_and_forget" => Some(Self::FireAndForget),
            "dymo_d1" => Some(Self::DymoD1),
            "dymo_lw" => Some(Self::DymoLw),
            "niimbot_poll" => Some(Self::NiimbotPoll),
            "letratag_notify" => Some(Self::LetraTagNotify),
            "gpgl" => Some(Self::Gpgl),
            _ => None,
        }
    }
}

/// A printer protocol driver: encodes a bitmap into protocol bytes.
pub trait Driver: Send + Sync {
    /// The protocol this driver implements.
    fn protocol(&self) -> Protocol;

    /// A short, stable driver name (for diagnostics).
    fn name(&self) -> &'static str;

    /// Wire / CLI / API ids that resolve to this driver's protocol.
    ///
    /// Include the canonical protocol id, orthographic variants, serde /
    /// catalog spellings, and brand synonyms. Do **not** list printer model
    /// catalog keys (those belong on catalog resolution and
    /// [`Self::variant_for_printer_key`]).
    ///
    /// Ids are matched case-insensitively by [`Self::matches_id`]. Prefer
    /// lowercase entries here.
    fn aliases(&self) -> &'static [&'static str];

    /// Whether `id` is one of this driver's [`Self::aliases`] (ASCII
    /// case-insensitive; surrounding whitespace trimmed).
    fn matches_id(&self, id: &str) -> bool {
        let key = id.trim();
        if key.is_empty() {
            return false;
        }
        self.aliases()
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(key))
    }

    /// Encode `bitmap` into the printer-native byte stream for `ctx`.
    fn encode(&self, bitmap: &MonoBitmap, ctx: &EncodeContext) -> Result<Vec<u8>, DriverError>;

    /// Client delivery strategy after encode (status pacing, notify wait, …).
    ///
    /// Default: [`ClientHandshake::FireAndForget`]. Drivers that need
    /// bidirectional pacing override this so callers (server client-print,
    /// browser encode) do not hard-code protocol lists.
    fn handshake(&self) -> ClientHandshake {
        ClientHandshake::FireAndForget
    }

    /// Resolve a catalog printer key to a driver-variant string.
    ///
    /// Default: no mapping. Drivers with model-specific firmware/task profiles
    /// override this; the registry forwards keys without knowing which.
    fn variant_for_printer_key(&self, _key: &str) -> Option<&'static str> {
        None
    }

    /// Return a replacement driver for `variant`, or `None` to keep `self`.
    ///
    /// Default: no variants. Opaque strings are interpreted only by the driver
    /// that understands them.
    fn override_for_variant(&self, _variant: Option<&str>) -> Option<Box<dyn Driver>> {
        None
    }
}
