//! Core transpilation: rewrite custom elements and assemble the document.

use lbl_core::job::OutputMode;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::assets;
use crate::assets::AssetsBase;
use crate::qr::{QrElementOverrides, QrErrorCorrection};

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
    /// QR error-correction level (redundancy).
    pub qr_error_correction: QrErrorCorrection,
    /// QR quiet zone, in modules (0 = none).
    pub qr_margin: u32,
    /// QR dark module color (hex).
    pub qr_dark: String,
    /// QR light module color (hex).
    pub qr_light: String,
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
            qr_error_correction: QrErrorCorrection::default(),
            qr_margin: 0,
            qr_dark: "#000000".into(),
            qr_light: "#ffffff".into(),
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
            qr_error_correction: QrErrorCorrection::default(),
            qr_margin: 0,
            qr_dark: "#000000".into(),
            qr_light: "#ffffff".into(),
        }
    }

    /// The `window.__LBL_STYLE` JSON consumed by the QR/barcode init scripts.
    fn to_js_config(&self) -> String {
        serde_json::json!({
            "qr": {
                "width": self.qr_size_px.max(1.0).round() as u32,
                "errorCorrectionLevel": self.qr_error_correction.as_str(),
                "margin": self.qr_margin,
                "color": {
                    "dark": self.qr_dark,
                    "light": self.qr_light,
                },
            },
            "barcode": {
                "width": self.barcode_module_width_px.max(0.1),
                "height": self.barcode_height_px.max(1.0).round() as u32,
                "fontSize": self.font_size_px.max(1.0).round() as u32,
            },
        })
        .to_string()
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

/// Render viewport size in CSS pixels (matches the rasterizer's device dots
/// times its supersample factor).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewportPx {
    /// Viewport width, if fixed by the media.
    pub width: Option<f64>,
    /// Viewport height, if fixed by the media.
    pub height: Option<f64>,
}

impl ViewportPx {
    /// CSS that pins the document to a known media viewport so previews and
    /// raster output show the full label width/length, not just the inked area.
    fn to_css(&self, mode: OutputMode, label_fit: LabelFit) -> String {
        let mut css = String::new();
        if self.width.is_some() {
            css.push_str("html,body{min-width:100%;width:100%}\n");
        }
        if label_fit == LabelFit::Fill && self.height.is_some() {
            css.push_str("html,body{min-height:100%;height:100%}\n");
        }
        if mode == OutputMode::Preview {
            if let Some(w) = self.width {
                css.push_str(&format!(
                    ".lbl-preview{{width:{w:.2}px;box-sizing:border-box}}\n"
                ));
            }
            if let Some(h) = self.height {
                css.push_str(&format!(".lbl-preview{{height:{h:.2}px}}\n"));
            }
        }
        css
    }
}

/// Calibration inset from the physical media edge, in millimetres.
///
/// Keeps fit/alignment inside the printable area when a printer feeds or cuts
/// slightly off. Distinct from [`LabelStyle`] padding, which is inside
/// `.lbl-label`. Side names follow the label reading frame (portrait: start =
/// top, cross-start = left).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MediaInset {
    /// Uniform inset on all sides.
    pub all_mm: f64,
    /// Both cross-axis sides (left + right in portrait).
    pub horizontal_mm: Option<f64>,
    /// Both main-axis sides (top + bottom in portrait).
    pub vertical_mm: Option<f64>,
    /// Main-axis start (top in portrait).
    pub start_mm: Option<f64>,
    /// Main-axis end (bottom in portrait).
    pub end_mm: Option<f64>,
    /// Cross-axis start (left in portrait).
    pub cross_start_mm: Option<f64>,
    /// Cross-axis end (right in portrait).
    pub cross_end_mm: Option<f64>,
}

/// Resolved inset on each side, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaInsetSides {
    start: f64,
    end: f64,
    cross_start: f64,
    cross_end: f64,
}

impl MediaInset {
    /// Resolve side values; more specific fields override axis/uniform defaults.
    pub fn resolve(self) -> MediaInsetSides {
        let base = self.all_mm.max(0.0);
        let horizontal = self.horizontal_mm.unwrap_or(base).max(0.0);
        let vertical = self.vertical_mm.unwrap_or(base).max(0.0);
        MediaInsetSides {
            start: self.start_mm.unwrap_or(vertical),
            end: self.end_mm.unwrap_or(vertical),
            cross_start: self.cross_start_mm.unwrap_or(horizontal),
            cross_end: self.cross_end_mm.unwrap_or(horizontal),
        }
    }

    /// Convert to CSS pixels for the render viewport's DPI and supersample.
    pub fn to_px(self, dpi: f64, supersample: u32) -> MediaInsetPx {
        let px_per_mm = dpi * supersample.max(1) as f64 / 25.4;
        let sides = self.resolve();
        MediaInsetPx {
            start: sides.start * px_per_mm,
            end: sides.end * px_per_mm,
            cross_start: sides.cross_start * px_per_mm,
            cross_end: sides.cross_end * px_per_mm,
        }
    }
}

/// Physical media inset in CSS pixels (top/right/bottom/left in portrait).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MediaInsetPx {
    /// Main-axis start (CSS padding-top in portrait).
    pub start: f64,
    /// Main-axis end (CSS padding-bottom in portrait).
    pub end: f64,
    /// Cross-axis start (CSS padding-left in portrait).
    pub cross_start: f64,
    /// Cross-axis end (CSS padding-right in portrait).
    pub cross_end: f64,
}

impl MediaInsetPx {
    pub fn is_zero(self) -> bool {
        self.start <= f64::EPSILON
            && self.end <= f64::EPSILON
            && self.cross_start <= f64::EPSILON
            && self.cross_end <= f64::EPSILON
    }

    fn css_padding(self) -> String {
        format!(
            "padding:{:.2}px {:.2}px {:.2}px {:.2}px",
            self.start, self.cross_end, self.end, self.cross_start
        )
    }
}

fn layout_shell_selector(mode: OutputMode) -> &'static str {
    if mode == OutputMode::Preview {
        ".lbl-preview"
    } else {
        "body"
    }
}

/// Inset the layout shell so fit/align stay inside the calibrated printable area.
fn media_inset_css(inset: MediaInsetPx, mode: OutputMode) -> String {
    if inset.is_zero() {
        return String::new();
    }
    let shell = layout_shell_selector(mode);
    format!("{shell}{{{};box-sizing:border-box}}\n", inset.css_padding())
}

/// Cross-axis alignment of content within `.lbl-label` when the media viewport
/// width is known (horizontal centering on portrait labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelAlign {
    /// Align to the start edge (left in LTR).
    Start,
    /// Center on the cross axis.
    #[default]
    Center,
    /// Align to the end edge (right in LTR).
    End,
}

/// Main-axis alignment within the fit box (vertical centering on portrait labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelValign {
    Start,
    #[default]
    Center,
    End,
}

macro_rules! impl_axis_align {
    ($t:ty) => {
        impl $t {
            /// Parse a config / CLI value (`start`, `center`, `end`, plus `top` /
            /// `bottom` or `left` / `right` aliases where sensible).
            pub fn parse(s: &str) -> Option<Self> {
                match s.trim().to_ascii_lowercase().as_str() {
                    "start" | "left" | "top" => Some(Self::Start),
                    "center" | "centre" | "middle" => Some(Self::Center),
                    "end" | "right" | "bottom" => Some(Self::End),
                    _ => None,
                }
            }

            fn flex_keyword(self) -> &'static str {
                match self {
                    Self::Start => "flex-start",
                    Self::Center => "center",
                    Self::End => "flex-end",
                }
            }
        }
    };
}

impl_axis_align!(LabelAlign);
impl_axis_align!(LabelValign);

impl LabelAlign {
    fn align_items(self) -> &'static str {
        self.flex_keyword()
    }
}

impl LabelValign {
    fn justify_content(self) -> &'static str {
        self.flex_keyword()
    }
}

/// Parse a fit-scale multiplier: `0.8`, `80%`, or bare `80` (treated as a
/// percentage). Clamped to `(0.01, 1.0]`.
pub fn parse_fit_scale(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        return pct
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| (v / 100.0).clamp(0.01, 1.0));
    }
    let n: f64 = s.parse().ok()?;
    if n > 1.0 {
        Some((n / 100.0).clamp(0.01, 1.0))
    } else {
        Some(n.clamp(0.01, 1.0))
    }
}

/// Layout CSS for fit mode sizing, scaling, and axis alignment.
fn label_layout_css(
    label_fit: LabelFit,
    viewport: Option<&ViewportPx>,
    mode: OutputMode,
    align: LabelAlign,
    valign: LabelValign,
    scale: f64,
    inset: MediaInsetPx,
) -> String {
    let apply_cross = label_fit == LabelFit::Fill || viewport.is_some_and(|v| v.width.is_some());
    let apply_main = label_fit == LabelFit::Fill || viewport.is_some_and(|v| v.height.is_some());

    let mut css = media_inset_css(inset, mode);

    if label_fit == LabelFit::Fill {
        let pct = scale.clamp(0.01, 1.0) * 100.0;
        css.push_str(&format!(
            ".lbl-label{{height:{pct:.4}%;width:{pct:.4}%;flex-shrink:0;box-sizing:border-box}}\n"
        ));
        let container = layout_shell_selector(mode);
        css.push_str(&format!(
            "{container}{{display:flex;flex-direction:column;justify-content:{};align-items:{}}}\n",
            valign.justify_content(),
            align.align_items(),
        ));
    } else if !inset.is_zero() {
        // Continuous/content mode: inset the shell even without a fill box.
        let container = layout_shell_selector(mode);
        css.push_str(&format!(
            "{container}{{display:flex;flex-direction:column}}\n"
        ));
    }

    let mut label_rules = Vec::new();
    if apply_cross {
        label_rules.push(format!("align-items:{}", align.align_items()));
    }
    if apply_main {
        label_rules.push(format!("justify-content:{}", valign.justify_content()));
    }
    if !label_rules.is_empty() {
        css.push_str(&format!(".lbl-label{{{}}}\n", label_rules.join(";")));
    }

    css
}

/// How `.lbl-label` is sized within the render viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelFit {
    /// Size to content (typical for continuous media).
    Content,
    /// Fill the viewport and center content on the main axis (typical for
    /// fixed die-cut labels).
    Fill,
}

/// Configurable label-fit policy before media is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelFitSetting {
    /// Fill fixed-length media; shrink on continuous media.
    #[default]
    Auto,
    Fill,
    Content,
}

impl LabelFitSetting {
    /// Parse a config / CLI value (`auto`, `fill`, `content`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "fill" => Some(Self::Fill),
            "content" => Some(Self::Content),
            _ => None,
        }
    }

    /// Resolve to a concrete [`LabelFit`] given whether the media has a fixed
    /// length.
    pub fn resolve(self, fixed_media_length: bool) -> LabelFit {
        match self {
            Self::Auto => {
                if fixed_media_length {
                    LabelFit::Fill
                } else {
                    LabelFit::Content
                }
            }
            Self::Fill => LabelFit::Fill,
            Self::Content => LabelFit::Content,
        }
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
    /// How the label root fills the render viewport.
    pub label_fit: LabelFit,
    /// Physical media viewport, when known (preview gallery and rasterization).
    pub viewport: Option<ViewportPx>,
    /// Cross-axis alignment when the viewport width is known or the label fills
    /// fixed-length media.
    pub label_align: LabelAlign,
    /// Main-axis alignment within the fit box (fill mode or fixed-length media).
    pub label_valign: LabelValign,
    /// Fraction of the viewport used by the fit box in fill mode (`1.0` = 100%).
    pub label_fit_scale: f64,
    /// Inset from the physical media edge (calibration margin).
    pub media_inset: MediaInsetPx,
}

impl Default for TranspileOptions {
    fn default() -> Self {
        Self {
            mode: OutputMode::Print,
            assets_base: AssetsBase::default(),
            index: None,
            count: None,
            style: LabelStyle::default(),
            label_fit: LabelFit::Content,
            viewport: None,
            label_align: LabelAlign::default(),
            label_valign: LabelValign::default(),
            label_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
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
            let overrides = QrElementOverrides::from_tag_attrs(&caps[1]);
            let payload = caps[2].trim();
            let mut out = format!("<div class=\"lbl-qr\" data-qr=\"{}\"", attr(payload));
            if let Some(ec) = overrides.error_correction {
                out.push_str(&format!(" data-ec=\"{}\"", attr(ec.as_str())));
            }
            if let Some(m) = overrides.margin {
                out.push_str(&format!(" data-margin=\"{m}\""));
            }
            if let Some(d) = overrides.dark {
                out.push_str(&format!(" data-dark=\"{}\"", attr(&d)));
            }
            if let Some(l) = overrides.light {
                out.push_str(&format!(" data-light=\"{}\"", attr(&l)));
            }
            out.push_str("></div>");
            out
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
    if opts.label_fit == LabelFit::Fill {
        head.push_str(assets::LABEL_FIT_FILL_CSS);
        head.push_str(assets::LABEL_FIT_TEXT_CSS);
        if let Some(fit_css) = crate::text_fit::lone_text_fit_css(body, opts) {
            head.push_str(&fit_css);
        }
        if opts.mode == OutputMode::Preview {
            head.push_str(assets::LABEL_FIT_FILL_PREVIEW_CSS);
        }
    }
    head.push_str(&label_layout_css(
        opts.label_fit,
        opts.viewport.as_ref(),
        opts.mode,
        opts.label_align,
        opts.label_valign,
        opts.label_fit_scale,
        opts.media_inset,
    ));
    if let Some(viewport) = &opts.viewport {
        head.push_str(&viewport.to_css(opts.mode, opts.label_fit));
    }
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
    fn fill_mode_injects_text_fit_css() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(100.0),
                height: Some(200.0),
            }),
            ..Default::default()
        };
        let out = transpile(
            "<div class=\"lbl-label\"><div class=\"lbl-text\">hi</div></div>",
            &opts,
        );
        assert!(out.contains("container-type:size"), "{out}");
        assert!(out.contains("calc(100cqh / 1.1)"), "{out}");
    }

    #[test]
    fn fill_mode_computes_lone_text_font_from_viewport() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_px: 0.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let out = transpile(
            "<div class=\"lbl-label\"><div class=\"lbl-text\">#1</div></div>",
            &opts,
        );
        assert!(
            out.contains(".lbl-label>.lbl-text:only-child{font-size:129."),
            "{out}"
        );
    }

    #[test]
    fn viewport_css_sizes_preview_and_stretches_document() {
        let opts = TranspileOptions {
            mode: OutputMode::Preview,
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(400.0),
                height: Some(200.0),
            }),
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(out.contains(".lbl-preview{width:400.00px"), "{out}");
        assert!(out.contains(".lbl-preview{height:200.00px"), "{out}");
        assert!(out.contains("min-width:100%"), "{out}");
        assert!(
            out.contains(".lbl-preview{width:100%;height:100%}"),
            "{out}"
        );
    }

    #[test]
    fn media_inset_resolves_specific_over_axis_defaults() {
        let inset = MediaInset {
            all_mm: 1.0,
            horizontal_mm: Some(2.0),
            vertical_mm: Some(3.0),
            start_mm: Some(4.0),
            cross_end_mm: Some(5.0),
            ..Default::default()
        };
        let sides = inset.resolve();
        assert_eq!(sides.start, 4.0);
        assert_eq!(sides.end, 3.0);
        assert_eq!(sides.cross_start, 2.0);
        assert_eq!(sides.cross_end, 5.0);
    }

    #[test]
    fn media_inset_css_insets_layout_shell() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            media_inset: MediaInsetPx {
                start: 10.0,
                end: 20.0,
                cross_start: 30.0,
                cross_end: 40.0,
            },
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(
            out.contains("padding:10.00px 40.00px 20.00px 30.00px"),
            "{out}"
        );
        assert!(out.contains("body{display:flex"), "{out}");
    }

    #[test]
    fn fill_mode_scale_and_valign_are_configurable() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            label_fit_scale: 0.8,
            label_valign: LabelValign::Start,
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(out.contains("height:80.0000%"), "{out}");
        assert!(out.contains("width:80.0000%"), "{out}");
        assert!(out.contains("body{display:flex"), "{out}");
        assert!(out.contains("justify-content:flex-start"), "{out}");
    }

    #[test]
    fn parse_fit_scale_accepts_fraction_and_percent() {
        assert_eq!(parse_fit_scale("0.8"), Some(0.8));
        assert_eq!(parse_fit_scale("80%"), Some(0.8));
        assert_eq!(parse_fit_scale("80"), Some(0.8));
    }

    #[test]
    fn fill_mode_injects_viewport_css() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(out.contains("height:100.0000%"), "{out}");
        assert!(out.contains("justify-content:center"), "{out}");
        assert!(out.contains("align-items:center"), "{out}");
    }

    #[test]
    fn label_align_start_is_configurable() {
        let opts = TranspileOptions {
            viewport: Some(ViewportPx {
                width: Some(400.0),
                height: None,
            }),
            label_align: LabelAlign::Start,
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(out.contains("align-items:flex-start"), "{out}");
    }

    #[test]
    fn content_mode_omits_viewport_css() {
        let out = transpile(
            "<div class=\"lbl-label\">hi</div>",
            &TranspileOptions::default(),
        );
        assert!(!out.contains("html,body{height:100%"), "{out}");
        assert!(!out.contains(".lbl-label{height:100%"), "{out}");
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
                ..Default::default()
            },
            ..Default::default()
        };
        let out = transpile("<qr>x</qr><barcode>9</barcode>", &opts);
        assert!(out.contains("font-size:96.00px"), "{out}");
        assert!(out.contains("padding:24.00px"), "{out}");
        assert!(out.contains("border:6.00px solid #000"), "{out}");
        assert!(
            out.contains(".lbl-qr{width:300.00px;height:300.00px}"),
            "{out}"
        );
        assert!(out.contains("window.__LBL_STYLE="), "{out}");
        assert!(out.contains(r#""errorCorrectionLevel":"M""#), "{out}");
        assert!(out.contains(r#""margin":0"#), "{out}");
        assert!(out.contains(r#""width":300"#), "{out}"); // qr width in js
        assert!(out.contains(r#""height":200"#), "{out}"); // barcode height in js
    }

    #[test]
    fn qr_element_attrs_become_data_attributes() {
        let out = transpile(
            r#"<qr ec="H" margin="2">x</qr>"#,
            &TranspileOptions::default(),
        );
        assert!(out.contains(r#"data-ec="H""#), "{out}");
        assert!(out.contains(r#"data-margin="2""#), "{out}");
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
