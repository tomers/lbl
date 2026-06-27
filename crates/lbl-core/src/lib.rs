//! Shared types for the `lbl` label-printing toolchain.
//!
//! `lbl-core` is the foundational crate every pipeline stage and binary depends
//! on. It defines the vocabulary of the toolchain: physical [`units`], page
//! [`geometry`], [`media`] descriptions, [`printer`] models/transports, and the
//! [`job`] specification that flows through the GCC-style pipeline
//! (`text -> template -> transpile -> render -> dither -> encode -> spool`).
//!
//! See the architecture document in `docs/architecture/ARCHITECTURE.md` for how
//! these types connect the stages together.

pub mod error;
pub mod geometry;
pub mod job;
pub mod media;
pub mod printer;
pub mod units;

pub use error::{CoreError, Result};
pub use geometry::{Margins, Size};
pub use job::{JobSpec, OutputMode};
pub use media::{Adhesive, Material, Media, MediaColor, MediaLength};
pub use printer::{
    PrinterCapabilities, PrinterId, PrinterModel, PrinterProfile, Protocol, Transport,
};
pub use units::{Dots, Dpi, Millimeters};
