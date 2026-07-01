//! Templating preprocessor for the `lbl` pipeline.
//!
//! `lbl-template` is the "preprocessor" stage: it renders a template against
//! data (JSON, TOML, or YAML) to produce one or more authoring-HTML labels, and
//! fetches/integrates external image resources so the output is self-contained.
//!
//! - Templating uses [minijinja](https://docs.rs/minijinja) (Jinja2 semantics).
//! - Data and template can live in one file via frontmatter (a JSX-like
//!   single-file format); see [`frontmatter`].
//! - A data array (or a `--each` JSON-pointer into the data) expands into a
//!   batch of N labels.
//! - [`resources`] fetches `<img>` references (local path or URL) and inlines
//!   them as `data:` URIs.
//!
//! ```
//! use lbl_template::{Engine, RenderOptions};
//! use serde_json::json;
//!
//! let labels = Engine::new()
//!     .render("<div>{{ name }}</div>", Some(json!({"name":"Sam"})), &RenderOptions::default())
//!     .unwrap();
//! assert_eq!(labels[0].html, "<div>Sam</div>");
//! ```

pub mod data;
pub mod engine;
pub mod frontmatter;
pub mod resources;
pub mod selection;

pub use data::DataFormat;
pub use engine::{resolve_batch, BatchSource, Engine, RenderOptions, RenderedLabel};
pub use resources::{DefaultResolver, MapResolver, ResourceResolver};
pub use selection::{
    flatten_values, record_matches_query, select_batch_indices, BatchSelection,
};

/// Errors produced by the templating stage.
#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    /// Failed to parse or interpret the data.
    #[error("data error: {0}")]
    Data(String),

    /// Failed to compile or render the template.
    #[error("template render error: {0}")]
    Render(String),

    /// Failed to fetch or inline a resource.
    #[error("resource error: {0}")]
    Resource(String),

    /// An I/O error reading inputs.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
