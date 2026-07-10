//! Core transpilation: rewrite custom elements and assemble the document.

use lbl_core::job::OutputMode;
use lbl_text::{google_fonts_link, resolve_slug};
use once_cell::sync::Lazy;
use regex::Regex;

use crate::assets;
use crate::assets::AssetsBase;
use crate::layout_fit::{apply_content_head_text_fit, apply_layout_fit};
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
    /// Default QR edge length in millimetres (used to scale per-element overrides).
    pub qr_size_mm: f64,
    /// Barcode bar height, in pixels.
    pub barcode_height_px: f64,
    /// Barcode single-module (narrowest bar) width, in pixels.
    pub barcode_module_width_px: f64,
    /// Inner padding between the label edge and its content, in pixels.
    pub padding_px: f64,
    /// Gap between sibling flex items, in pixels.
    pub element_gap_px: f64,
    /// Border drawn around the label, in pixels (0 = no border).
    pub border_width_px: f64,
    /// Corner radius for fixed die-cut labels in preview, in pixels (0 = square).
    pub corner_radius_px: f64,
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
            qr_size_mm: 15.0,
            barcode_height_px: 100.0,
            barcode_module_width_px: 2.0,
            padding_px: 20.0,
            element_gap_px: 8.0,
            border_width_px: 0.0,
            corner_radius_px: 0.0,
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
        element_gap_mm: f64,
        border_width_mm: f64,
        corner_radius_mm: f64,
        dpi: f64,
        supersample: u32,
    ) -> Self {
        let px_per_mm = dpi * supersample.max(1) as f64 / 25.4;
        Self {
            font_size_px: font_size_mm * px_per_mm,
            qr_size_px: qr_size_mm * px_per_mm,
            qr_size_mm,
            barcode_height_px: barcode_height_mm * px_per_mm,
            barcode_module_width_px: barcode_module_width_mm * px_per_mm,
            padding_px: padding_mm * px_per_mm,
            element_gap_px: element_gap_mm * px_per_mm,
            border_width_px: border_width_mm * px_per_mm,
            corner_radius_px: corner_radius_mm * px_per_mm,
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
            ".lbl-label,.lbl-row,.lbl-col{{gap:{gap:.2}px}}\n.lbl-label{{font-size:{fs:.2}px;line-height:1.3;padding:{pad:.2}px;border:{bw:.2}px solid #000}}\n.lbl-qr{{width:{qr:.2}px;height:{qr:.2}px}}\n",
            gap = self.element_gap_px.max(0.0),
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

/// Physical page size for vector PDF export, in millimetres (reading frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSizeMm {
    /// Page width across the reading frame.
    pub width_mm: f64,
    /// Page height along the feed, if fixed by the media.
    pub height_mm: Option<f64>,
}

impl PageSizeMm {
    /// CSS `@page` rule that pins the PDF to these dimensions.
    pub fn to_css(&self) -> String {
        match self.height_mm {
            Some(h) => format!(
                "@page{{size:{w:.4}mm {h:.4}mm;margin:0}}\n",
                w = self.width_mm,
                h = h
            ),
            None => format!("@page{{size:{w:.4}mm auto;margin:0}}\n", w = self.width_mm),
        }
    }
}

impl ViewportPx {
    /// CSS that pins the document to a known media viewport so previews and
    /// raster output show the full label width/length, not just the inked area.
    fn to_css(&self, mode: OutputMode, _label_fit: LabelFit) -> String {
        let mut css = String::new();
        if self.width.is_some() {
            css.push_str("html,body{min-width:100%;width:100%}\n");
        }
        if self.height.is_some() {
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
                    "bottom" | "end" | "right" => Some(Self::End),
                    "center" | "centre" | "middle" => Some(Self::Center),
                    "left" | "start" | "top" => Some(Self::Start),
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

    fn text_align(self) -> &'static str {
        match self {
            Self::Start => "left",
            Self::Center => "center",
            Self::End => "right",
        }
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
    } else if !inset.is_zero() || apply_main {
        let container = layout_shell_selector(mode);
        css.push_str(&format!(
            "{container}{{display:flex;flex-direction:column;box-sizing:border-box}}\n"
        ));
    }

    let mut label_rules = Vec::new();
    if apply_cross {
        label_rules.push(format!("align-items:{}", align.align_items()));
    }
    if apply_main {
        label_rules.push(format!("justify-content:{}", valign.justify_content()));
        if label_fit == LabelFit::Content {
            label_rules.push("flex:1 1 auto".into());
            label_rules.push("min-height:0".into());
        }
    }
    if !label_rules.is_empty() {
        css.push_str(&format!(".lbl-label{{{}}}\n", label_rules.join(";")));
    }

    css
}

/// Alignment for a lone `.lbl-text` child in fill mode (must follow
/// [`assets::LABEL_FIT_TEXT_CSS`]).
fn lone_text_align_css(align: LabelAlign, valign: LabelValign) -> String {
    format!(
        ".lbl-label>.lbl-text:only-child{{align-items:{};justify-content:{};text-align:{}}}\n",
        align.align_items(),
        valign.justify_content(),
        align.text_align(),
    )
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
    /// Multiplier applied to auto-fit text size in fill mode (`1.0` = maximum).
    pub font_fit_scale: f64,
    /// Inset from the physical media edge (calibration margin).
    pub media_inset: MediaInsetPx,
    /// Physical page size for vector PDF export (`@page` rule).
    pub page_size: Option<PageSizeMm>,
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
            font_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
            page_size: None,
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
static BARCODE_HEIGHT_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)\bheight\s*=\s*"([^"]*)""#).expect("barcode height attr regex"));
static BODY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?is)<body\b[^>]*>(.*)</body>").expect("body regex"));

/// Transpile authoring HTML into a browser-ready document.
pub fn transpile(input: &str, opts: &TranspileOptions) -> String {
    let body = extract_body(input);

    let mut features = Features::default();
    let px_per_mm = opts.style.qr_size_px / opts.style.qr_size_mm.max(f64::EPSILON);
    let body = rewrite_qr(&body, &mut features, px_per_mm);
    let body = rewrite_barcode(&body, &mut features);

    let (body, fill_css) = match opts.label_fit {
        LabelFit::Fill => {
            let fit = apply_layout_fit(&body, opts);
            (fit.body, fit.css)
        }
        LabelFit::Content => {
            let fit = apply_content_head_text_fit(&body, opts);
            (fit.body, fit.css)
        }
    };

    assemble(&body, features, opts, &fill_css)
}

/// Extract the inner body if a full document was given; otherwise treat the
/// whole input as the body fragment.
fn extract_body(input: &str) -> String {
    match BODY_RE.captures(input) {
        Some(caps) => caps[1].trim().to_string(),
        None => input.trim().to_string(),
    }
}

fn rewrite_qr(body: &str, features: &mut Features, px_per_mm: f64) -> String {
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
            if let Some(mm) = overrides.size_mm {
                let px = (mm * px_per_mm).round().max(1.0) as u32;
                out.push_str(&format!(
                    " data-width=\"{px}\" style=\"width:{px}px;height:{px}px;flex:0 0 auto\""
                ));
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
            let height_attr = BARCODE_HEIGHT_ATTR_RE
                .captures(attrs)
                .map(|c| lbl_text::BarcodeHeightMode::parse(&c[1]))
                .unwrap_or_default();
            let value = caps[2].trim();
            let height_data = if height_attr == lbl_text::BarcodeHeightMode::Stretch {
                r#" data-barcode-height="stretch""#
            } else {
                ""
            };
            format!(
                "<div class=\"lbl-barcode\" data-symbology=\"{}\" data-value=\"{}\"{height_data}></div>",
                attr(&symbology),
                attr(value)
            )
        })
        .into_owned()
}

fn preview_corner_radius_css(style: &LabelStyle) -> String {
    if style.corner_radius_px <= f64::EPSILON {
        return String::new();
    }
    format!(
        ".lbl-preview{{border-radius:{:.2}px;overflow:hidden}}\n",
        style.corner_radius_px
    )
}

static LBL_FONT_ATTR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"data-lbl-font="([^"]+)""#).expect("lbl font attr regex"));

/// CSS rules and optional Google Fonts link for `data-lbl-font` spans in `body`.
fn font_assets_for_body(body: &str) -> (String, Option<String>) {
    let mut slugs: Vec<String> = Vec::new();
    for caps in LBL_FONT_ATTR_RE.captures_iter(body) {
        let slug = caps[1].to_string();
        if resolve_slug(&slug).is_some() && !slugs.iter().any(|s| s == &slug) {
            slugs.push(slug);
        }
    }

    let mut css = String::new();
    for slug in &slugs {
        if let Some(def) = resolve_slug(slug) {
            css.push_str(&format!(
                ".lbl-label [data-lbl-font=\"{slug}\"]{{font-family:{css}}}\n",
                slug = slug,
                css = def.css
            ));
        }
    }

    let link = google_fonts_link(&slugs.iter().map(String::as_str).collect::<Vec<_>>());
    (css, link)
}

fn assemble(body: &str, features: Features, opts: &TranspileOptions, fill_css: &str) -> String {
    let mut head = String::new();
    head.push_str("<meta charset=\"utf-8\">\n");
    head.push_str("<style>");
    if let Some(page) = opts.page_size {
        head.push_str(&page.to_css());
    }
    head.push_str(assets::BASE_CSS);
    head.push_str(&opts.style.to_css());
    let (font_css, font_link) = font_assets_for_body(body);
    head.push_str(&font_css);
    if opts.label_fit == LabelFit::Fill {
        head.push_str(&format!(
            ".lbl-label{{--lbl-font-fit-scale:{:.4}}}\n",
            opts.font_fit_scale.clamp(0.01, 5.0)
        ));
        head.push_str(assets::LABEL_FIT_FILL_CSS);
        head.push_str(assets::LABEL_FIT_TEXT_CSS);
        head.push_str(assets::LABEL_FIT_ROW_TEXT_CSS);
        head.push_str(assets::LABEL_FIT_CODE_CSS);
        head.push_str(&lone_text_align_css(opts.label_align, opts.label_valign));
        if opts.mode == OutputMode::Preview {
            head.push_str(assets::LABEL_FIT_FILL_PREVIEW_CSS);
        }
    }
    head.push_str(fill_css);
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
        if opts
            .viewport
            .as_ref()
            .is_some_and(|viewport| viewport.height.is_some())
        {
            head.push_str(&preview_corner_radius_css(&opts.style));
        }
    }
    head.push_str("</style>\n");
    if let Some(url) = font_link {
        head.push_str(&format!(
            "<link rel=\"stylesheet\" href=\"{}\">\n",
            attr(&url)
        ));
    }

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
    fn font_directive_injects_css_and_google_link() {
        let out = transpile(
            "<div class=\"lbl-label\"><span class=\"lbl-text\" data-lbl-font=\"roboto\">Hi</span><span class=\"lbl-text\" data-lbl-font=\"bebas-neue\">Title</span></div>",
            &TranspileOptions::default(),
        );
        assert!(out.contains("font-family:'Roboto',sans-serif"), "{out}");
        assert!(out.contains("font-family:'Bebas Neue',sans-serif"), "{out}");
        assert!(out.contains("fonts.googleapis.com"), "{out}");
    }

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
    fn colored_barcode_stays_inside_color_span() {
        let body = r#"<div class="lbl-label"><span class="lbl-text"><span class="lbl-text-inlines"><span style="color:#e90b0b">aa <barcode type="CODE128">12345</barcode> cc</span></span></span></div>"#;
        let out = transpile(body, &TranspileOptions::default());
        assert!(
            out.contains(
                r#"<span style="color:#e90b0b">aa <div class="lbl-barcode" data-symbology="CODE128" data-value="12345"></div> cc</span>"#
            ),
            "{out}"
        );
        assert!(
            out.contains("lineColor=color") || out.contains("opts.lineColor"),
            "{out}"
        );
    }

    #[test]
    fn fill_lone_barcode_gets_data_fit() {
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
            r#"<div class="lbl-label"><barcode>12345</barcode></div>"#,
            &opts,
        );
        assert!(out.contains("data-fit-width="), "{out}");
        assert!(out.contains("data-fit-height="), "{out}");
        assert!(
            out.contains("baseFont*(opts.height/baseH)"),
            "barcode caption should scale with fit height: {out}"
        );
        let w: f64 = out
            .split("data-fit-width=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(w > 100.0, "lone barcode should grow, got {w}");
    }

    #[test]
    fn fill_lone_qr_gets_data_fit() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_px: 0.0,
                qr_size_px: 40.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let out = transpile(r#"<div class="lbl-label"><qr>x</qr></div>"#, &opts);
        assert!(out.contains("data-fit-width="), "{out}");
        let w: f64 = out
            .split("data-fit-width=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(w > 40.0, "lone qr should grow beyond config, got {w}");
    }

    #[test]
    fn fill_text_barcode_row_grows_both() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_px: 0.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let out = transpile(
            r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">aa </span></span><barcode>12346</barcode><span class="lbl-text"><span class="lbl-text-inlines"> bb</span></span></div></div>"#,
            &opts,
        );
        assert!(out.contains(".lbl-row>.lbl-text{font-size:"), "{out}");
        assert!(out.contains("data-fit-width="), "{out}");
        assert!(out.contains("data-fit-height="), "{out}");
    }

    #[test]
    fn fill_text_qr_row_grows_qr() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_px: 0.0,
                qr_size_px: 40.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let out = transpile(
            r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><qr>x</qr></div></div>"#,
            &opts,
        );
        assert!(out.contains("data-fit-width="), "{out}");
        assert!(out.contains(".lbl-row>.lbl-text{font-size:"), "{out}");
        let w: f64 = out
            .split("data-fit-width=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(w > 40.0, "row qr should grow, got {w}");
    }

    #[test]
    fn fill_barcode_qr_row_shares_width() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_px: 0.0,
                qr_size_px: 40.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let out = transpile(
            r#"<div class="lbl-label"><div class="lbl-row lbl-center"><barcode>1</barcode><qr>x</qr></div></div>"#,
            &opts,
        );
        let fits: Vec<&str> = out.split("data-fit-width=\"").skip(1).collect();
        assert!(fits.len() >= 2, "expected both codes fitted: {out}");
    }

    #[test]
    fn content_mode_skips_data_fit() {
        let out = transpile(
            r#"<div class="lbl-label"><barcode>12345</barcode></div>"#,
            &TranspileOptions {
                label_fit: LabelFit::Content,
                viewport: Some(ViewportPx {
                    width: Some(354.0),
                    height: Some(142.0),
                }),
                ..Default::default()
            },
        );
        assert!(!out.contains("data-fit-width="), "{out}");
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
    fn preview_fixed_media_has_rounded_corners() {
        let opts = TranspileOptions {
            mode: OutputMode::Preview,
            viewport: Some(ViewportPx {
                width: Some(100.0),
                height: Some(200.0),
            }),
            style: LabelStyle::from_mm(2.0, 15.0, 12.0, 0.33, 2.0, 2.0, 0.0, 2.0, 300.0, 2),
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(out.contains("border-radius:47.24px"), "{out}");
        assert!(out.contains("overflow:hidden"), "{out}");
    }

    #[test]
    fn preview_continuous_media_has_no_rounded_corners() {
        let opts = TranspileOptions {
            mode: OutputMode::Preview,
            viewport: Some(ViewportPx {
                width: Some(100.0),
                height: None,
            }),
            ..Default::default()
        };
        let out = transpile("<div class=\"lbl-label\">hi</div>", &opts);
        assert!(!out.contains("border-radius"), "{out}");
    }

    #[test]
    fn content_mode_sizes_lone_text_to_continuous_head_height() {
        let opts = TranspileOptions {
            mode: OutputMode::Preview,
            label_fit: LabelFit::Content,
            viewport: Some(ViewportPx {
                width: None,
                height: Some(170.0),
            }),
            style: LabelStyle::from_mm(2.0, 15.0, 12.0, 0.33, 2.0, 2.0, 0.0, 2.0, 180.0, 2),
            ..Default::default()
        };
        let body =
            r#"<div class="lbl-label"><div class="lbl-text">01234567890123456789</div></div>"#;
        let fit = apply_content_head_text_fit(body, &opts);
        assert!(fit.font_px.unwrap_or(0.0) > 60.0, "{:?}", fit.font_px);
        let out = transpile(body, &opts);
        assert!(
            out.contains(".lbl-label>.lbl-text:only-child{font-size:"),
            "{out}"
        );
    }

    #[test]
    fn print_mode_has_no_preview_chrome() {
        let out = transpile("<div>hi</div>", &TranspileOptions::default());
        assert!(!out.contains("lbl-preview"));
    }

    #[test]
    fn vector_page_size_injects_at_page_rule() {
        let opts = TranspileOptions {
            page_size: Some(PageSizeMm {
                width_mm: 40.0,
                height_mm: Some(30.0),
            }),
            ..Default::default()
        };
        let out = transpile("<div>hi</div>", &opts);
        assert!(
            out.contains("@page{size:40.0000mm 30.0000mm;margin:0}"),
            "{out}"
        );
    }

    #[test]
    fn qr_uses_svg_init() {
        let out = transpile("<qr>x</qr>", &TranspileOptions::default());
        assert!(out.contains("type:'svg'"), "{out}");
    }

    #[test]
    fn markdown_typography_resets_block_margins() {
        let out = transpile(
            r#"<div class="lbl-label"><h1>Title</h1><p>Body</p></div>"#,
            &TranspileOptions::default(),
        );
        assert!(
            out.contains(
                ".lbl-label :is(h1,h2,h3,h4,h5,h6,p,ul,ol,blockquote,strong,b,em){margin:0}"
            ),
            "{out}"
        );
        assert!(out.contains(".lbl-label h1{font-size:1.35em"), "{out}");
    }

    #[test]
    fn fill_mode_scales_lone_text_by_font_fit_scale() {
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
            font_fit_scale: 0.5,
            ..Default::default()
        };
        let out = transpile(
            "<div class=\"lbl-label\"><div class=\"lbl-text\">#1</div></div>",
            &opts,
        );
        assert!(
            out.contains(".lbl-label>.lbl-text:only-child{font-size:64."),
            "{out}"
        );
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
    fn fill_mode_fits_single_line_span_text() {
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
            "<div class=\"lbl-label\"><span class=\"lbl-text\">30×20</span></div>",
            &opts,
        );
        assert!(
            out.contains(".lbl-label>.lbl-text:only-child{font-size:"),
            "{out}"
        );
        let fit = out
            .rsplit(".lbl-label>.lbl-text:only-child{font-size:")
            .next()
            .and_then(|s| s.split("px}").next())
            .and_then(|s| s.parse::<f64>().ok())
            .expect("computed lone-text font size");
        assert!(
            fit > 20.0 && fit < 142.0 / crate::text_fit::LINE_HEIGHT,
            "fit={fit}"
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
    fn lone_text_respects_label_align() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(400.0),
                height: Some(200.0),
            }),
            label_align: LabelAlign::Start,
            label_valign: LabelValign::Center,
            ..Default::default()
        };
        let out = transpile(
            r#"<div class="lbl-label"><span class="lbl-text">align left</span></div>"#,
            &opts,
        );
        assert!(
            out.contains(
                ".lbl-label>.lbl-text:only-child{align-items:flex-start;justify-content:center;text-align:left}"
            ),
            "{out}"
        );
    }

    #[test]
    fn lone_text_label_align_end() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(400.0),
                height: Some(200.0),
            }),
            label_align: LabelAlign::End,
            ..Default::default()
        };
        let out = transpile(
            r#"<div class="lbl-label"><span class="lbl-text">align right</span></div>"#,
            &opts,
        );
        assert!(out.contains("text-align:right"), "{out}");
        assert!(out.contains("align-items:flex-end"), "{out}");
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
    fn content_mode_valign_end_stretches_label_root() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Content,
            viewport: Some(ViewportPx {
                width: Some(400.0),
                height: Some(200.0),
            }),
            label_valign: LabelValign::End,
            ..Default::default()
        };
        let out = transpile(
            r#"<div class="lbl-label"><span class="lbl-text">bottom</span></div>"#,
            &opts,
        );
        assert!(
            out.contains("html,body{min-height:100%;height:100%}"),
            "{out}"
        );
        assert!(
            out.contains("body{display:flex;flex-direction:column;box-sizing:border-box}"),
            "{out}"
        );
        assert!(
            out.contains("justify-content:flex-end") && out.contains("flex:1 1 auto"),
            "{out}"
        );
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
    fn qr_size_mm_sets_container_and_data_width() {
        let out = transpile(r#"<qr size_mm="10">x</qr>"#, &TranspileOptions::default());
        assert!(out.contains(r#"data-width="107""#), "{out}");
        assert!(
            out.contains(r#"style="width:107px;height:107px;flex:0 0 auto""#),
            "{out}"
        );
    }

    #[test]
    fn from_mm_scales_with_dpi_and_supersample() {
        // 3mm at 300dpi, supersample 3 -> 3 * 300 * 3 / 25.4 = ~106.3px.
        let s = LabelStyle::from_mm(3.0, 15.0, 12.0, 0.33, 2.0, 2.0, 0.0, 2.0, 300.0, 3);
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
