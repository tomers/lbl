//! The `lbl` orchestrator library.
//!
//! This crate wires the individual pipeline stages (each its own library and
//! `lbl-*` binary) into composable flows. The [`pipeline`] module exposes the
//! chaining logic used by the `lbl` binary's high-level `print` and `preview`
//! flows.

pub mod debug;
pub mod dispatch;
pub mod pipeline;
pub mod preprocess;
pub mod preview;
pub mod print_stats;
pub mod terminal;

pub use pipeline::{
    authoring_labels, encode_label, encode_label_from_rgba, encode_label_traced, encode_labels,
    encode_sample_pattern, encode_sample_pattern_traced, frame_html_preview_stock,
    pad_preview_encode_feed, page_size_mm, preview_stock_frame, render_label_raster,
    render_viewport_px, render_viewport_vector, resolve_label_align, resolve_label_fit,
    resolve_label_fit_scale, resolve_label_valign, resolve_media, resolve_media_inset,
    resolve_print_transport, resolve_style_vector, resolve_template_format, transpile_label_html,
    AuthoringLabel, BatchSelection, EncodeFromRgbaResult, EncodeLabelsOptions, EncodeLabelsResult,
    LabelRaster, PipelineOptions, PreviewFeedPad, PreviewStockFrame, Source, TemplateFormat,
    TranspiledLabelHtml, VECTOR_CSS_DPI,
};
pub use preprocess::{
    estimate_job, estimate_render_dimensions, hires_pixels_per_label, job_input,
    machine_capacity_factor, suggest_supersample, JobPreprocessInput, PreprocessEstimate,
    BATCH_WARN_INTERVAL, WARN_WEIGHT_THRESHOLD,
};
pub use print_stats::{
    feed_dots_for_trace, format_duration, format_efficiency, format_throughput, total_feed_mm,
    LabelFeedDots, PrintRunTimings, PrintSummaryInput,
};
