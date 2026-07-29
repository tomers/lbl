//! Shared types for the `lbl` label-printing toolchain.
//!
//! `lbl-core` is the foundational crate every pipeline stage and binary depends
//! on. It defines the vocabulary of the toolchain: physical [`units`], page
//! [`geometry`], [`media`] descriptions, [`printer`] models/transports, and the
//! [`job`] specification that flows through the pipeline
//! (`text -> template -> transpile -> render -> dither -> encode -> spool`).
//!
//! See the architecture document in `docs/src/architecture.md` for how these
//! types connect the stages together.

pub mod bitmap;
pub mod cut;
pub mod error;
pub mod feed_plan;
pub mod geometry;
pub mod job;
pub mod media;
pub mod orientation;
pub mod printer;
pub mod units;

pub use bitmap::MonoBitmap;
pub use cut::{CutJobSpec, CutPath, CutPointMm, SilhouetteOptions};
pub use error::{CoreError, Result};
pub use feed_plan::{
    resolve_feed_plan, resolve_virtual_feed_gaps, FeedPlan, FeedPlanError, PaddingSidesMm,
    VirtualFeedGaps, LEAD_PADDING_BELOW_CUTTER_GAP, LEAD_PADDING_BELOW_MIN,
};
pub use geometry::{Margins, Size};
pub use job::{
    CutKind, CutMode, DriverOptions, DymoLwOptions, JobSpec, LwOutputMode, LwSpeed, OutputMode,
};
pub use media::{Adhesive, Material, Media, MediaColor, MediaLength, MediaSense};
pub use orientation::{Orientation, Rotation};
pub use printer::{DeviceCapabilities, DeviceId, DeviceModel, DeviceProfile, Protocol, Transport};
pub use units::{Dots, Dpi, Millimeters, CSS_LAYOUT_REFERENCE_DPI};
