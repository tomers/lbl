//! Convert plain text (and CLI directives) into *authoring HTML*.
//!
//! `lbl-text` is the thin front-end of the pipeline: it turns a quick string
//! like `"hello, world!"` into the authoring HTML contract consumed by
//! `lbl-transpile-html` (and ultimately rendered and printed).
//!
//! # Inline mini-syntax (default)
//!
//! Directives are written inline with double braces:
//!
//! - `{{qr:https://example.com}}` -> a QR code
//! - `{{barcode:CODE128:12345}}` -> a barcode (symbology optional; defaults to
//!   CODE128, so `{{barcode:12345}}` also works)
//! - `{{image:./photo.jpg}}` -> an image by local path or remote URL
//! - `{{size:1.5:World}}` -> text at 1.5x the base font size (aliases:
//!   `font-size`, `fs`; scale also accepts `1.5x` or `150%`)
//!
//! ```
//! use lbl_text::Document;
//! let doc = Document::parse("ship to {{qr:https://x.y}}", false);
//! let html = doc.to_authoring_html();
//! assert!(html.contains("<qr>https://x.y</qr>"));
//! ```
//!
//! # Raw mode
//!
//! With raw mode enabled, inline syntax is **not** parsed; the text is treated
//! literally (so a label can contain the characters `{{...}}`). Flag-based
//! directives still apply and are appended after the text.

mod parse;

pub use parse::{barcode_from_spec, parse_directive, Block, Document};

/// The default barcode symbology when one isn't specified.
pub const DEFAULT_SYMBOLOGY: &str = "CODE128";
