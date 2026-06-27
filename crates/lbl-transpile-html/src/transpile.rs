//! Core transpilation: rewrite custom elements and assemble the document.

use lbl_core::job::OutputMode;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::assets;
use crate::assets::AssetsBase;

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
}

impl Default for TranspileOptions {
    fn default() -> Self {
        Self {
            mode: OutputMode::Print,
            assets_base: AssetsBase::default(),
            index: None,
            count: None,
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
    fn full_document_body_is_extracted() {
        let input = "<!doctype html><html><head></head><body><qr>z</qr></body></html>";
        let out = transpile(input, &TranspileOptions::default());
        // Only one body should remain (no nesting of original head/body).
        assert_eq!(out.matches("<body>").count(), 1);
        assert!(out.contains("data-qr=\"z\""));
    }
}
