//! The `lbl` orchestrator library.
//!
//! This crate wires the individual pipeline stages (each its own library and
//! `lbl-*` binary) into composable flows. The [`pipeline`] module exposes the
//! chaining logic used by the `lbl` binary's high-level `print` and `preview`
//! flows.

pub mod pipeline;

pub use pipeline::{
    authoring_labels, encode_label, resolve_media, AuthoringLabel, PipelineOptions, Source,
};
