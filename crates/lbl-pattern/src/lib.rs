//! Generate printer calibration patterns as 1-bit [`MonoBitmap`]s.
//!
//! The sample pattern matches the classic LabelManager calibration layout:
//! staggered corner lines, vertical rules, fine and
//! dyadic checkerboards, and a solid block, composed horizontally. The raster is
//! emitted at exact device dots and is intended to pass straight to
//! [`lbl-encode`] without rescaling or dithering.

mod sample;

pub use sample::{
    orient_sample_pattern, resolve_head_dots, sample_pattern, sample_pattern_for_media,
    sample_pattern_sized,
};
