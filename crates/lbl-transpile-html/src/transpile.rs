//! Core transpilation: rewrite custom elements and assemble the document.

use lbl_core::job::OutputMode;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::assets;
use crate::assets::AssetsBase;

/// Visual sizing for a label, in **CSS pixels** (which map 1:1 to render
/// device dots at the render viewport's resolution).
///
/// Callers that think in physical units (mm) should convert with
/// [`LabelStyle::from_mm`], which folds in the target DPI and supersample
/// factor so the on-label size is resolution-independent.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelStyle {
    /// Base text size, in pixels.
    pub font_size_px: f64,
    /// QR code edge length, in pixels.
    pub qr_size_px: f64,
    /// Barcode bar height, in pixels.
    pub barcode_height_px: f64,
    /// Barcode single-module (narrowest bar) width, in pixels.
    pub barcode_module_width_px: f64,
    /// Inner padding between the label edge and its content, in pixels.
    pub padding_px: f64,
    /// Border drawn around the label, in pixels (0 = no border).
    pub border_width_px: f64,
}

impl Default for LabelStyle {
    fn default() -> Self {
        // Neutral defaults for standalone/browser use, where no print geometry
        // is known. The print pipeline overrides these via `from_mm`.
        Self {
            font_size_px: 32.0,
            qr_size_px: 160.0,
            barcode_height_px: 100.0,
            barcode_module_width_px: 2.0,
            padding_px: 20.0,
            border_width_px: 0.0,
        }
    }
}

impl LabelStyle {
    /// Resolve physical (mm) sizes to pixels for a render targeting `dpi` at the
    /// given `supersample` factor.
    ///
    /// The render viewport is `width_dots * supersample` pixels wide, with a
    /// device scale factor of 1, so 1mm corresponds to `dpi * supersample /
    /// 25.4` CSS pixels regardless of the label's width.
    #[allow(clippy::too_many_arguments)]
    pub fn from_mm(
        font_size_mm: f64,
        qr_size_mm: f64,
        barcode_height_mm: f64,
        barcode_module_width_mm: f64,
        padding_mm: f64,
        border_width_mm: f64,
        dpi: f64,
        supersample: u32,
    ) -> Self {
        let px_per_mm = dpi * supersample.max(1) as f64 / 25.4;
        Self {
            font_size_px: font_size_mm * px_per_mm,
            qr_size_px: qr_size_mm * px_per_mm,
            barcode_height_px: barcode_height_mm * px_per_mm,
            barcode_module_width_px: barcode_module_width_mm * px_per_mm,
            padding_px: padding_mm * px_per_mm,
            border_width_px: border_width_mm * px_per_mm,
        }
    }

    /// The `window.__LBL_STYLE` JSON consumed by the QR/barcode init scripts.
    fn to_js_config(&self) -> String {
        format!(
            "{{qr:{{width:{qr:.0}}},barcode:{{width:{bw:.2},height:{bh:.0},fontSize:{fs:.0}}}}}",
            qr = self.qr_size_px.max(1.0),
            bw = self.barcode_module_width_px.max(0.1),
            bh = self.barcode_height_px.max(1.0),
            fs = self.font_size_px.max(1.0),
        )
    }

    /// The extra CSS that applies font, padding, border and QR sizing.
    fn to_css(&self) -> String {
        // `box-sizing:border-box` (from the base CSS) keeps the label within the
        // media width even with padding and a border applied.
        format!(
            ".lbl-label{{font-size:{fs:.2}px;line-height:1.3;padding:{pad:.2}px;border:{bw:.2}px solid #000}}\n.lbl-qr{{width:{qr:.2}px;height:{qr:.2}px}}\n",
            fs = self.font_size_px.max(1.0),
            pad = self.padding_px.max(0.0),
            bw = self.border_width_px.max(0.0),
            qr = self.qr_size_px.max(1.0),
        )
    }
}

/// Options controlling transpilation.
#[derive(Debug, Clone)]
pub struct TranspileOptions {
    /// Target output mode (print vs preview).
    pub mode: OutputMode,
    /// Where third-party libraries are loaded from.
    pub assets_base: AssetsBase,
    /// Label index within a batch (preview gallery addressing).
    pub index: Option<usize>,
    /// Total number of labels in the batch (preview gallery addressing).
    pub count: Option<usize>,
    /// Font / QR / barcode sizing.
    pub style: LabelStyle,
}

impl Default for TranspileOptions {
    fn default() -> Self {
        Self {
            mode: OutputMode::Print,
            assets_base: AssetsBase::default(),
            index: None,
            count: None,
            style: LabelStyle::default(),
        }
    }
}

/// Which features were detected, so only the needed assets are injected.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Features {
    qr: bool,
    barcode: bool,
}

static QR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<qr\b([^>]*)>(.*?)</qr>").expect("qr regex"));
static BARCODE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<barcode\b([^>]*)>(.*?)</barcode>").expect("barcode regex"));
static TYPE_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\btype\s*=\s*"([^"]*)""#).expect("type attr regex"));
static BODY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<body\b[^>]*>(.*)</body>").expect("body regex"));

/// Transpile authoring HTML into a browser-ready document.
pub fn transpile(input: &str, opts: &TranspileOptions) -> String {
    let body = extract_body(input);

    let mut features = Features::default();
    let body = rewrite_qr(&body, &mut features);
    let body = rewrite_barcode(&body, &mut features);

    assemble(&body, features, opts)
}

/// Extract the inner body if a full document was given; otherwise treat the
/// whole input as the body fragment.
fn extract_body(input: &str) -> String {
    match BODY_RE.captures(input) {
        Some(caps) => caps[1].trim().to_string(),
        None => input.trim().to_string(),
    }
}

fn rewrite_qr(body: &str, features: &mut Features) -> String {
    QR_RE
        .replace_all(body, |caps: &regex::Captures| {
            features.qr = true;
            let payload = caps[2].trim();
            format!("<div class=\"lbl-qr\" data-qr=\"{}\"></div>", attr(payload))
        })
        .into_owned()
}

fn rewrite_barcode(body: &str, features: &mut Features) -> String {
    BARCODE_RE
        .replace_all(body, |caps: &regex::Captures| {
            features.barcode = true;
            let attrs = &caps[1];
            let symbology = TYPE_ATTR_RE
                .captures(attrs)
                .map(|c| c[1].to_string())
                .unwrap_or_else(|| "CODE128".to_string());
            let value = caps[2].trim();
            format!(
                "<div class=\"lbl-barcode\" data-symbology=\"{}\" data-value=\"{}\"></div>",
                attr(&symbology),
                attr(value)
            )
        })
        .into_owned()
}

fn assemble(body: &str, features: Features, opts: &TranspileOptions) -> String {
    let mut head = String::new();
    head.push_str("<meta charset=\"utf-8\">\n");
    head.push_str("<style>");
    head.push_str(assets::BASE_CSS);
    head.push_str(&opts.style.to_css());
    if opts.mode == OutputMode::Preview {
        head.push_str(assets::PREVIEW_CSS);
    }
    head.push_str("</style>\n");

    // Wrap body: preview mode adds an addressable, gallery-friendly container.
    let wrapped_body = if opts.mode == OutputMode::Preview {
        let index = opts.index.unwrap_or(0);
        let count = opts.count.unwrap_or(1);
        format!(
            "<div class=\"lbl-preview\" data-label-index=\"{index}\" data-label-count=\"{count}\">{body}</div>"
        )
    } else {
        body.to_string()
    };

    let mut scripts = String::new();
    if features.qr || features.barcode {
        scripts.push_str(&inline_script(&format!(
            "window.__LBL_STYLE={};",
            opts.style.to_js_config()
        )));
    }
    if features.qr {
        scripts.push_str(&script_src(&opts.assets_base.qrcode_url()));
        scripts.push_str(&inline_script(assets::QR_INIT_JS));
    }
    if features.barcode {
        scripts.push_str(&script_src(&opts.assets_base.jsbarcode_url()));
        scripts.push_str(&inline_script(assets::BARCODE_INIT_JS));
    }

    format!(
        "<!doctype html>\n<html>\n<head>\n{head}</head>\n<body>\n{wrapped_body}\n{scripts}</body>\n</html>\n"
    )
}

fn script_src(url: &str) -> String {
    format!("<script src=\"{}\"></script>\n", attr(url))
}

fn inline_script(js: &str) -> String {
    format!("<script>{js}</script>\n")
}

/// Escape a value for use inside a double-quoted HTML attribute.
fn attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_is_rewritten_and_lib_injected() {
        let out = transpile("<qr>https://x.y</qr>", &TranspileOptions::default());
        assert!(out.contains("class=\"lbl-qr\""));
        assert!(out.contains("data-qr=\"https://x.y\""));
        assert!(out.contains("qrcode.min.js"));
        assert!(
            !out.contains("JsBarcode.all.min.js"),
            "barcode lib should not be injected"
        );
    }

    #[test]
    fn barcode_symbology_extracted() {
        let out = transpile(
            "<barcode type=\"EAN13\">123</barcode>",
            &TranspileOptions::default(),
        );
        assert!(out.contains("data-symbology=\"EAN13\""));
        assert!(out.contains("data-value=\"123\""));
        assert!(out.contains("JsBarcode.all.min.js"));
    }

    #[test]
    fn barcode_defaults_symbology() {
        let out = transpile("<barcode>999</barcode>", &TranspileOptions::default());
        assert!(out.contains("data-symbology=\"CODE128\""));
    }

    #[test]
    fn preview_mode_wraps_with_addressable_container() {
        let opts = TranspileOptions {
            mode: OutputMode::Preview,
            index: Some(7),
            count: Some(200),
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(out.contains("class=\"lbl-preview\""));
        assert!(out.contains("data-label-index=\"7\""));
        assert!(out.contains("data-label-count=\"200\""));
        assert!(out.contains("background")); // preview CSS present
    }

    #[test]
    fn print_mode_has_no_preview_chrome() {
        let out = transpile("<div>hi</div>", &TranspileOptions::default());
        assert!(!out.contains("lbl-preview"));
    }

    #[test]
    fn local_assets_base_used() {
        let opts = TranspileOptions {
            assets_base: AssetsBase::Local("/assets".into()),
            ..Default::default()
        };
        let out = transpile("<qr>x</qr>", &opts);
        assert!(out.contains("/assets/qrcode.min.js"));
    }

    #[test]
    fn style_css_and_js_are_injected() {
        let opts = TranspileOptions {
            style: LabelStyle {
                font_size_px: 96.0,
                qr_size_px: 300.0,
                barcode_height_px: 200.0,
                barcode_module_width_px: 4.0,
                padding_px: 24.0,
                border_width_px: 6.0,
            },
            ..Default::default()
        };
        let out = transpile("<qr>x</qr><barcode>9</barcode>", &opts);
        assert!(out.contains("font-size:96.00px"), "{out}");
        assert!(out.contains("padding:24.00px"), "{out}");
        assert!(out.contains("border:6.00px solid #000"), "{out}");
        assert!(out.contains(".lbl-qr{width:300.00px;height:300.00px}"), "{out}");
        assert!(out.contains("window.__LBL_STYLE="), "{out}");
        assert!(out.contains("width:300"), "{out}"); // qr width in js
        assert!(out.contains("height:200"), "{out}"); // barcode height in js
    }

    #[test]
    fn from_mm_scales_with_dpi_and_supersample() {
        // 3mm at 300dpi, supersample 3 -> 3 * 300 * 3 / 25.4 = ~106.3px.
        let s = LabelStyle::from_mm(3.0, 15.0, 12.0, 0.33, 2.0, 0.0, 300.0, 3);
        assert!((s.font_size_px - 106.299).abs() < 0.1, "{}", s.font_size_px);
        // 2mm padding at the same density.
        assert!((s.padding_px - 70.866).abs() < 0.1, "{}", s.padding_px);
    }

    #[test]
    fn full_document_body_is_extracted() {
        let input = "<!doctype html><html><head></head><body><qr>z</qr></body></html>";
        let out = transpile(input, &TranspileOptions::default());
        // Only one body should remain (no nesting of original head/body).
        assert_eq!(out.matches("<body>").count(), 1);
        assert!(out.contains("data-qr=\"z\""));
    }
}
