//! Convert plain text (and CLI directives) into *authoring HTML*.
//!
//! `lbl-text` is the thin front-end of the pipeline: it turns a quick string
//! like `"hello, world!"` into the authoring HTML contract consumed by
//! `lbl-transpile-html` (and ultimately rendered and printed).
//!
//! # Inline mini-syntax (default)
//!
//! Directives are written inline with double square brackets, so they never
//! collide with `{{ … }}` template interpolation (`lbl-template`) run before
//! this front-end:
//!
//! - `[[qr:https://example.com]]` -> a QR code (payload only; use
//!   `[[qr ec=low]]payload[[/qr]]` when options are needed)
//! - `[[barcode:CODE128:12345]]` -> a barcode (symbology optional; defaults to
//!   CODE128, so `[[barcode:12345]]` also works)
//! - `[[image:./photo.jpg]]` -> an image by local path or remote URL
//! - `[[date:%Y-%m-%d]]` / `[[time:%H:%M]]` / `[[datetime:%Y-%m-%d %H:%M]]` ->
//!   a date/time stamp (`<stamp>`); resolved to local wall-clock text once per
//!   preview/print job via [`resolve_stamps_at`]
//! - `[[size:1.5:World]]` -> text at 1.5x the base font size (aliases:
//!   `font-size`, `fs`; scale also accepts `1.5x` or `150%`)
//! - `[[font:roboto:Hello]]` -> text in a named font (aliases: `font-family`,
//!   `ff`; see [`fonts::catalog`] for supported slugs)
//! - `[[color:#ff0000:Hello]]` -> colored text (aliases: `fg`, `foreground`,
//!   `text-color`, `tc`; hex `#rgb` or `#rrggbb`)
//!
//! ```
//! use lbl_text::Document;
//! let doc = Document::parse("ship to [[qr:https://x.y]]", false);
//! let html = doc.to_authoring_html();
//! assert!(html.contains("<qr>https://x.y</qr>"));
//! ```
//!
//! # Raw mode
//!
//! With raw mode enabled, inline syntax is **not** parsed; the text is treated
//! literally (so a label can contain the characters `[[...]]`). Flag-based
//! directives still apply and are appended after the text.

mod fonts;
mod parse;
mod qr;
mod stamp;

pub use fonts::{
    catalog, default_font_assets_base_url, face_paths_for_slugs, font_asset_url,
    font_assets_base_url_from_env, font_face_css_inline, font_face_css_remote, fonts, resolve_slug,
    FontCatalog, FontDef, FontEntry, FontFaceFile,
};
pub use parse::{
    barcode_from_spec, parse_directive, scan_directive_at, BarcodeHeightMode, Block, Document,
};
pub use qr::{QrErrorCorrection, QrOptions};
pub use stamp::{format_stamp, resolve_stamps, resolve_stamps_at, StampKind};

/// The default barcode symbology when one isn't specified.
pub const DEFAULT_SYMBOLOGY: &str = "CODE128";
