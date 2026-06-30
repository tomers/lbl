//! Convert Markdown (and CLI directives) into *authoring HTML*.
//!
//! `lbl-markdown` is a front-end of the pipeline, sibling to `lbl-text`: it
//! turns a Markdown document into the authoring HTML contract consumed by
//! `lbl-transpile-html` (and ultimately rendered and printed).
//!
//! Markdown is rendered to HTML the usual way (headings, lists, emphasis,
//! links, ...), but the same inline mini-syntax as `lbl-text` is **still
//! applied** anywhere in the document:
//!
//! - `{{qr:https://example.com}}` -> a QR code (`{{qr ec=low}}…{{/qr}}` for options)
//! - `{{barcode:CODE128:12345}}` -> a barcode (symbology optional; defaults to
//!   CODE128, so `{{barcode:12345}}` also works)
//! - `{{image:./photo.jpg}}` -> an image by local path or remote URL
//!
//! ```
//! use lbl_markdown::MarkdownDocument;
//! let doc = MarkdownDocument::parse("# Ship to\n\n{{qr:https://x.y}}");
//! let html = doc.to_authoring_html();
//! assert!(html.contains("<h1>Ship to</h1>"));
//! assert!(html.contains("<qr>https://x.y</qr>"));
//! ```
//!
//! Directives are extracted *before* the Markdown is parsed and stitched back
//! in afterwards, so the directive payloads are emitted verbatim (byte-for-byte
//! identical to `lbl-text`) and are never mangled by Markdown processing.

mod render;

pub use render::MarkdownDocument;
