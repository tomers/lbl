//! The `lbl` orchestrator library.
//!
//! This crate wires the individual pipeline stages (each its own library and
//! `lbl-*` binary) into composable flows. The [`pipeline`] module exposes the
//! chaining logic used by the `lbl` binary's high-level `print` and `preview`
//! flows.

pub mod debug;
pub mod dispatch;
pub mod pipeline;
pub mod terminal;

pub use pipeline::{
    authoring_labels, encode_label, encode_label_traced, encode_sample_pattern,
    encode_sample_pattern_traced, resolve_label_align, resolve_label_fit,
    resolve_label_fit_scale, resolve_label_valign, resolve_media, resolve_media_inset,
    resolve_print_transport, render_viewport_px, AuthoringLabel, PipelineOptions, Source,
};
