//! Transpile *authoring HTML* into *browser-ready HTML*.
//!
//! Authoring HTML uses compact custom concepts:
//! - `<qr>PAYLOAD</qr>` — a QR code
//! - `<barcode type="CODE128">DATA</barcode>` — a barcode
//! - flex utility classes (`lbl-row`, `lbl-col`, `lbl-center`, `lbl-grow`,
//!   `lbl-justify-*`, `lbl-items-*`, `lbl-slot`, ...)
//!
//! Transpilation rewrites those custom elements into placeholder `<div>`s,
//! injects the flex/base CSS, and pulls in the third-party JS libraries that
//! draw the QR/barcodes in the browser (or headless Chromium during rendering).
//!
//! Two output modes are supported (see [`lbl_core::job::OutputMode`]):
//! - **Print**: a bare, deterministic document for the rasterizer.
//! - **Preview**: a screen-oriented, gallery-friendly document wrapped in an
//!   addressable `.lbl-preview[data-label-index]` container.
//!
//! ```
//! use lbl_transpile_html::{transpile, TranspileOptions};
//! let html = transpile("<qr>hello</qr>", &TranspileOptions::default());
//! assert!(html.contains("class=\"lbl-qr\""));
//! ```

mod assets;
mod layout_fit;
mod qr;
mod symbology;
mod text_fit;
mod transpile;

pub use assets::AssetsBase;
pub use qr::{QrElementOverrides, QrErrorCorrection};
pub use symbology::{resolve_symbology, BarcodeRenderer, SymbologyInfo};
pub use text_fit::{fitted_font_px, injected_fit_font_px, injected_label_min_width_px};
pub use transpile::{
    parse_fit_scale, transpile, LabelAlign, LabelFit, LabelFitSetting, LabelStyle, LabelValign,
    MediaInset, MediaInsetPx, PageSizeMm, TranspileOptions, ViewportPx,
};
