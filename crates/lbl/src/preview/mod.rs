//! Browser HTML preview for `--protocol html`.

mod assets;
mod browser;
mod build;
mod context;
mod write;

pub use browser::{print_open_hint, serve_and_open};
pub use build::{input_from_run, PreviewSourceArgs};
pub use context::{HtmlPreviewContext, HtmlPreviewInput};
pub use write::{resolve_html_preview_paths, write_html_preview, HtmlPreviewPaths};
