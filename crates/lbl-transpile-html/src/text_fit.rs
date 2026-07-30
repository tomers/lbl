//! Compute the largest font size for a lone `.lbl-text` block on fixed media.

use once_cell::sync::Lazy;
use regex::Regex;
use unicode_width::UnicodeWidthChar;

use crate::transpile::{TranspileOptions, ViewportPx};

static ANY_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").expect("any tag regex"));

/// Line height for lone-text auto-fit (must match [`crate::assets::LABEL_FIT_TEXT_CSS`]).
pub const LINE_HEIGHT: f64 = 1.1;

/// Default line height on `.lbl-vertical` when unset (must match CSS fallback).
pub const VERTICAL_LINE_HEIGHT: f64 = 1.0;

/// Nominal em width of one upright vertical glyph column.
const VERTICAL_COL_EM: f64 = 1.0;

/// Ink often extends past estimated advance width (diagonal strokes, serifs).
/// Mid-scale auto-fit (internal scale `1.0`) uses this; toward aggressive
/// ink-tight the fit interpolates to margin `1.0` (full printable width).
pub(crate) const VISUAL_WIDTH_MARGIN: f64 = 1.35;

/// Aggressive ink-tight end of the mid→tight curve (width margin spent,
/// line-height fully tightened). May clip real glyph ink; reachable above
/// 100% at user scale `TIGHT_INTERNAL_SCALE / SAFE_FULL_INTERNAL_SCALE` (≈133%).
const TIGHT_INTERNAL_SCALE: f64 = 2.0;

/// User-facing `font_fit_scale = 1.0` maps here: largest **clipping-safe**
/// fill (former UI ~75% when 100% was ink-tight). Priority: no clip over max
/// utilization.
const SAFE_FULL_INTERNAL_SCALE: f64 = 1.5;

/// Estimated em width of one terminal column at the transpiled font size.
/// Used with [`VISUAL_WIDTH_MARGIN`] for fit checks inside a fixed box.
const EM_PER_COLUMN: f64 = 0.55;
/// Tighter column width for continuous feed advance estimates (proportional
/// Latin is narrower than the fit-safety column used above).
const ADVANCE_EM_PER_COLUMN: f64 = 0.42;
/// Whitespace advance for continuous feed estimates (tighter than fit-check `0.28`).
const ADVANCE_EM_WHITESPACE: f64 = 0.22;

/// Scale a non-text fit allocation (QR / barcode / image). Never exceeds the
/// pre-scale size — oversize would overflow the printable box. User `1.0`
/// maps to [`SAFE_FULL_INTERNAL_SCALE`] / [`TIGHT_INTERNAL_SCALE`] of the max
/// (same remapping as text); full mark size needs ≈133%.
pub(crate) fn scaled_fit_px(px: f64, opts: &TranspileOptions) -> f64 {
    let user = opts.font_fit_scale.clamp(0.01, 5.0);
    let fraction = (user * (SAFE_FULL_INTERNAL_SCALE / TIGHT_INTERNAL_SCALE)).min(1.0);
    (px * fraction).max(1.0)
}

/// Font size + CSS line-height from auto-fit (line-height may tighten toward
/// safe full; above 100% grows toward and past ink-tight and may clip).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FitTextPx {
    pub font_px: f64,
    pub line_height: f64,
}

fn tight_fit_progress(internal_scale: f64) -> f64 {
    ((internal_scale - 1.0) / (TIGHT_INTERNAL_SCALE - 1.0)).clamp(0.0, 1.0)
}

/// Auto-fit plain text with [`TranspileOptions::font_fit_scale`].
///
/// `base_lh` is the mid-scale CSS line-height ([`LINE_HEIGHT`] for lone text,
/// row text uses [`crate::assets::ROW_TEXT_LINE_HEIGHT`]).
///
/// User scale maps linearly onto internal scale via [`SAFE_FULL_INTERNAL_SCALE`]
/// (`1.0` = largest clipping-safe fill). Aggressive ink-tight is at
/// `TIGHT_INTERNAL_SCALE / SAFE_FULL_INTERNAL_SCALE` (≈133%); above that the
/// font multiplies past the printable max (may clip on the head axis;
/// continuous feed length grows with the rendered font).
pub(crate) fn fit_text_font_px(
    width_px: f64,
    height_px: f64,
    text: &str,
    opts: &TranspileOptions,
    base_lh: f64,
) -> FitTextPx {
    let internal = opts.font_fit_scale.clamp(0.01, 5.0) * SAFE_FULL_INTERNAL_SCALE;
    fit_text_font_px_for_internal(width_px, height_px, text, internal, base_lh)
}

fn fit_text_font_px_for_internal(
    width_px: f64,
    height_px: f64,
    text: &str,
    internal: f64,
    base_lh: f64,
) -> FitTextPx {
    let base_lh = base_lh.max(0.01);
    if internal <= TIGHT_INTERNAL_SCALE {
        return fit_text_font_px_at_most_tight(width_px, height_px, text, internal, base_lh);
    }
    let tight =
        fit_text_font_px_at_most_tight(width_px, height_px, text, TIGHT_INTERNAL_SCALE, base_lh);
    FitTextPx {
        font_px: (tight.font_px * (internal / TIGHT_INTERNAL_SCALE)).max(1.0),
        line_height: tight.line_height,
    }
}

fn fit_text_font_px_at_most_tight(
    width_px: f64,
    height_px: f64,
    text: &str,
    internal_scale: f64,
    base_lh: f64,
) -> FitTextPx {
    let scale = internal_scale.clamp(0.01, TIGHT_INTERNAL_SCALE);
    let comfortable =
        max_fit_font_px_with_margin_lh(width_px, height_px, text, VISUAL_WIDTH_MARGIN, base_lh);
    if scale <= 1.0 {
        return FitTextPx {
            font_px: (comfortable * scale).max(1.0),
            line_height: base_lh,
        };
    }
    let t = tight_fit_progress(scale);
    let scale_for_lh = 1.0 + (TIGHT_INTERNAL_SCALE - 1.0) * t;
    let target_lh = base_lh / scale_for_lh;
    let width_margin = VISUAL_WIDTH_MARGIN + (1.0 - VISUAL_WIDTH_MARGIN) * t;
    let font = max_fit_font_px_with_margin_lh(width_px, height_px, text, width_margin, target_lh);
    FitTextPx {
        font_px: font.max(1.0),
        line_height: target_lh,
    }
}

/// Auto-fit HTML text (vertical-aware) with [`TranspileOptions::font_fit_scale`].
/// Same scale rules as [`fit_text_font_px`].
pub(crate) fn fit_text_font_px_html(
    width_px: f64,
    height_px: f64,
    inner: &str,
    vertical_lh: f64,
    opts: &TranspileOptions,
    base_lh: f64,
) -> FitTextPx {
    let internal = opts.font_fit_scale.clamp(0.01, 5.0) * SAFE_FULL_INTERNAL_SCALE;
    fit_text_font_px_html_for_internal(width_px, height_px, inner, vertical_lh, internal, base_lh)
}

fn fit_text_font_px_html_for_internal(
    width_px: f64,
    height_px: f64,
    inner: &str,
    vertical_lh: f64,
    internal: f64,
    base_lh: f64,
) -> FitTextPx {
    let base_lh = base_lh.max(0.01);
    if internal <= TIGHT_INTERNAL_SCALE {
        return fit_text_font_px_html_at_most_tight(
            width_px,
            height_px,
            inner,
            vertical_lh,
            internal,
            base_lh,
        );
    }
    let tight = fit_text_font_px_html_at_most_tight(
        width_px,
        height_px,
        inner,
        vertical_lh,
        TIGHT_INTERNAL_SCALE,
        base_lh,
    );
    FitTextPx {
        font_px: (tight.font_px * (internal / TIGHT_INTERNAL_SCALE)).max(1.0),
        line_height: tight.line_height,
    }
}

fn fit_text_font_px_html_at_most_tight(
    width_px: f64,
    height_px: f64,
    inner: &str,
    vertical_lh: f64,
    internal_scale: f64,
    base_lh: f64,
) -> FitTextPx {
    let scale = internal_scale.clamp(0.01, TIGHT_INTERNAL_SCALE);
    let comfortable = max_fit_font_px_html_with_margin_lh(
        width_px,
        height_px,
        inner,
        vertical_lh,
        VISUAL_WIDTH_MARGIN,
        base_lh,
    );
    if scale <= 1.0 {
        return FitTextPx {
            font_px: (comfortable * scale).max(1.0),
            line_height: base_lh,
        };
    }
    let t = tight_fit_progress(scale);
    let scale_for_lh = 1.0 + (TIGHT_INTERNAL_SCALE - 1.0) * t;
    let target_lh = base_lh / scale_for_lh;
    let width_margin = VISUAL_WIDTH_MARGIN + (1.0 - VISUAL_WIDTH_MARGIN) * t;
    let font = max_fit_font_px_html_with_margin_lh(
        width_px,
        height_px,
        inner,
        vertical_lh,
        width_margin,
        target_lh,
    );
    FitTextPx {
        font_px: font.max(1.0),
        line_height: target_lh,
    }
}

/// Parse the auto-fit font size actually injected into transpiled HTML.
pub fn injected_fit_font_px(html: &str) -> Option<f64> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"\.lbl-(?:label>\.lbl-text:only-child|row>\.lbl-text)\{[^}]*font-size:([\d.]+)px",
        )
        .expect("injected fit font regex")
    });
    RE.captures(html)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Parse continuous-feed length estimate pinned by content head-fit.
///
/// `--lbl-feed-px` may share the `.lbl-label{…}` rule with width/overflow pins
/// (oversize font clip). Match it anywhere inside that rule — not only when it
/// is the first property (a first-only regex returned `None` and print capture
/// fell back to the 2048px iframe placeholder → meter-long blank D1 tape).
pub fn injected_label_min_width_px(html: &str) -> Option<f64> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\.lbl-label\{[^}]*--lbl-feed-px:([\d.]+)").expect("label feed width regex")
    });
    RE.captures(html)
        .and_then(|caps| caps.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

/// Largest auto-fit text size in pixels for this body, if computable.
pub fn fitted_font_px(body: &str, opts: &TranspileOptions) -> Option<f64> {
    crate::layout_fit::apply_layout_fit(body, opts).font_px
}

/// Inner HTML of a lone `.lbl-text` direct child of `.lbl-label`.
#[cfg(test)]
fn lone_text_inner_html(body: &str) -> Option<String> {
    let body = body.trim();
    const LABEL_OPEN: &str = r#"<div class="lbl-label">"#;
    const LABEL_CLOSE: &str = "</div>";
    if !body.starts_with(LABEL_OPEN) || !body.ends_with(LABEL_CLOSE) {
        return None;
    }
    let child = body[LABEL_OPEN.len()..body.len() - LABEL_CLOSE.len()].trim();
    if child.contains("lbl-row") {
        return None;
    }
    extract_lbl_text_inner(child)
}

#[cfg(test)]
fn extract_lbl_text_inner(html: &str) -> Option<String> {
    for tag in ["span", "div"] {
        let exact = format!(r#"<{tag} class="lbl-text">"#);
        if let Some(rest) = html.strip_prefix(&exact) {
            return balanced_element_inner(rest, tag);
        }
        let prefix = format!(r#"<{tag} class="lbl-text" "#);
        if let Some(rest) = html.strip_prefix(&prefix) {
            let gt = rest.find('>')?;
            return balanced_element_inner(&rest[gt + 1..], tag);
        }
    }
    None
}

#[cfg(test)]
fn balanced_element_inner(html: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 1i32;
    let mut i = 0;
    while i < html.len() {
        if html[i..].starts_with(&close) {
            depth -= 1;
            if depth == 0 {
                return Some(html[..i].to_string());
            }
            i += close.len();
            continue;
        }
        if html[i..].starts_with(&open) {
            depth += 1;
            i += open.len();
            continue;
        }
        i += html[i..].chars().next().map_or(1, |c| c.len_utf8());
    }
    None
}

/// CSS rule setting a transpile-time font size, when viewport geometry is known.
#[cfg(test)]
pub fn lone_text_fit_css(body: &str, opts: &TranspileOptions) -> Option<String> {
    let fit = crate::layout_fit::apply_layout_fit(body, opts);
    if fit
        .css
        .contains(".lbl-label>.lbl-text:only-child{font-size:")
    {
        Some(fit.css)
    } else {
        None
    }
}

/// CSS rule setting a transpile-time font size for text in a Fill row.
#[cfg(test)]
pub fn row_text_qr_fit_css(body: &str, opts: &TranspileOptions) -> Option<String> {
    let fit = crate::layout_fit::apply_layout_fit(body, opts);
    if fit.css.contains(".lbl-row>.lbl-text{font-size:") {
        fit.css
            .lines()
            .find(|l| l.contains(".lbl-row>.lbl-text{font-size:"))
            .map(|l| format!("{l}\n"))
    } else {
        None
    }
}

pub(crate) fn text_line_width_px(text: &str, font_px: f64) -> f64 {
    line_em_width(text) * font_px * VISUAL_WIDTH_MARGIN
}

/// Extra em padding on each inline side so continuous stock / end-margin guides
/// sit past glyph ink. Advance width alone underestimates serifs and curves
/// (e.g. trailing “c”), which then paint into feed-end blank tape.
pub(crate) const INK_SIDE_BEARING_EM: f64 = 0.08;

/// Nominal advance width (no fit-safety margin) for continuous stock sizing.
pub(crate) fn text_advance_width_px(text: &str, font_px: f64) -> f64 {
    text.lines()
        .map(|line| {
            line.chars()
                .map(|c| {
                    if c.is_whitespace() {
                        ADVANCE_EM_WHITESPACE
                    } else {
                        let cols = c.width().unwrap_or(1).max(1);
                        cols as f64 * ADVANCE_EM_PER_COLUMN
                    }
                })
                .sum::<f64>()
        })
        .fold(0.0_f64, f64::max)
        * font_px
}

/// Continuous feed content width: must cover real CSS ink, not the tight
/// advance heuristic alone. [`text_advance_width_px`] uses a narrow em/column
/// for fit binary-search; feed length uses [`text_line_width_px`] (wider em ×
/// [`VISUAL_WIDTH_MARGIN`]) so continuous tape / print capture do not clip the
/// trailing glyph (preview “C” half-cut, print missing “C”).
pub(crate) fn text_feed_content_width_px(text: &str, font_px: f64) -> f64 {
    text_line_width_px(text, font_px) + 2.0 * INK_SIDE_BEARING_EM * font_px.max(0.0)
}

pub(crate) fn fit_box_px(opts: &TranspileOptions) -> Option<(f64, f64)> {
    let ViewportPx { width, height } = opts.viewport.as_ref()?;
    let width = width.filter(|w| *w > f64::EPSILON);
    let height = height.filter(|h| *h > f64::EPSILON);
    if width.is_none() && height.is_none() {
        return None;
    }
    const UNBOUNDED_AXIS_PX: f64 = 1.0e6;
    let scale = opts.label_fit_scale.clamp(0.01, 1.0);
    let pad_x = opts.style.padding_x_px();
    let pad_y = opts.style.padding_y_px();
    let border = opts.style.border_width_px.max(0.0);
    let inset = opts.media_inset;
    let box_w = if let Some(w) = width {
        let inner_w = (w - inset.cross_start - inset.cross_end).max(0.0);
        inner_w * scale - pad_x - 2.0 * border
    } else {
        UNBOUNDED_AXIS_PX
    };
    let box_h = if let Some(h) = height {
        let inner_h = (h - inset.start - inset.end).max(0.0);
        inner_h * scale - pad_y - 2.0 * border
    } else {
        UNBOUNDED_AXIS_PX
    };
    if box_w <= f64::EPSILON || box_h <= f64::EPSILON {
        return None;
    }
    Some((box_w, box_h))
}

/// Whether lone-text auto-fit can measure this inner HTML (plain text plus
/// inline color/size/font spans, but not nested `.lbl-text` or widgets).
pub(crate) fn is_fit_measurable_html(inner: &str) -> bool {
    static NESTED_LBL_TEXT: Lazy<Regex> = Lazy::new(|| {
        // Match `.lbl-text` but not `.lbl-text-inlines` (no lookahead in `regex` crate).
        Regex::new(r#"class="lbl-text"([ >]|$)"#).expect("nested lbl-text regex")
    });
    !NESTED_LBL_TEXT.is_match(inner)
        && !inner.contains("<qr")
        && !inner.contains("<barcode")
        && !inner.contains("<stamp")
        && !inner.contains("lbl-barcode")
        && !inner.contains("lbl-qr")
        && !inner.contains("<img")
}

pub(crate) fn html_to_plain_text(inner: &str) -> String {
    let normalized = inner
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n");
    ANY_TAG_RE.replace_all(&normalized, "").to_string()
}

/// Em width/height of text HTML, honoring `.lbl-vertical` stacks as tall columns.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TextHtmlEmMetrics {
    pub width_em: f64,
    pub height_em: f64,
    pub advance_em: f64,
}

fn horizontal_advance_em(text: &str) -> f64 {
    text.chars()
        .map(|c| {
            if c.is_whitespace() {
                ADVANCE_EM_WHITESPACE
            } else {
                let cols = c.width().unwrap_or(1).max(1);
                cols as f64 * ADVANCE_EM_PER_COLUMN
            }
        })
        .sum()
}

fn vertical_stack_height_em(text: &str, vertical_lh: f64) -> f64 {
    let n = text.chars().filter(|c| !c.is_control()).count().max(1);
    n as f64 * vertical_lh
}

/// Measure authoring text HTML for auto-fit. Plain text keeps wrapping-aware
/// [`max_fit_font_px_with_margin_lh`]; `.lbl-vertical` runs are one column wide
/// and N glyphs tall.
pub(crate) fn text_html_em_metrics(inner: &str, _vertical_lh: f64) -> TextHtmlEmMetrics {
    static VERTICAL_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"(?is)<span\s+[^>]*\bclass="lbl-vertical"[^>]*>(.*?)</span>"#)
            .expect("lbl-vertical regex")
    });

    if !inner.contains("lbl-vertical") {
        let text = html_to_plain_text(inner);
        let mut width_em = 0.0_f64;
        let mut advance_em = 0.0_f64;
        let mut lines = 0usize;
        for line in text.split('\n') {
            lines += 1;
            width_em = width_em.max(line_em_width(line));
            advance_em = advance_em.max(horizontal_advance_em(line));
        }
        if lines == 0 {
            lines = 1;
        }
        return TextHtmlEmMetrics {
            width_em,
            height_em: lines as f64 * LINE_HEIGHT,
            advance_em,
        };
    }

    let mut width_em = 0.0_f64;
    let mut advance_em = 0.0_f64;
    let mut height_em = LINE_HEIGHT;
    let mut last = 0usize;

    for caps in VERTICAL_RE.captures_iter(inner) {
        let full = caps.get(0).expect("full match");
        let before = html_to_plain_text(&inner[last..full.start()]).replace('\n', " ");
        if !before.is_empty() {
            width_em += line_em_width(&before);
            advance_em += horizontal_advance_em(&before);
            height_em = height_em.max(LINE_HEIGHT);
        }
        let vertical_text = html_to_plain_text(caps.get(1).map(|m| m.as_str()).unwrap_or(""));
        width_em += VERTICAL_COL_EM;
        advance_em += VERTICAL_COL_EM;
        // Fit glyphs from the tight stack. Per-span `--lbl-vertical-spacing`
        // is CSS letter-spacing only — including it here would shrink/grow letters.
        height_em = height_em.max(vertical_stack_height_em(
            &vertical_text,
            VERTICAL_LINE_HEIGHT,
        ));
        last = full.end();
    }

    let after = html_to_plain_text(&inner[last..]).replace('\n', " ");
    if !after.is_empty() {
        width_em += line_em_width(&after);
        advance_em += horizontal_advance_em(&after);
        height_em = height_em.max(LINE_HEIGHT);
    }

    TextHtmlEmMetrics {
        width_em: width_em.max(0.0),
        height_em: height_em.max(VERTICAL_LINE_HEIGHT),
        advance_em: advance_em.max(0.0),
    }
}

/// Largest font that fits `inner` HTML into the box (vertical-aware), with
/// explicit width safety margin and CSS line-height for non-vertical runs.
pub(crate) fn max_fit_font_px_html_with_margin_lh(
    width_px: f64,
    height_px: f64,
    inner: &str,
    vertical_lh: f64,
    width_margin: f64,
    line_height: f64,
) -> f64 {
    if !inner.contains("lbl-vertical") {
        return max_fit_font_px_with_margin_lh(
            width_px,
            height_px,
            &html_to_plain_text(inner),
            width_margin,
            line_height,
        );
    }
    let margin = width_margin.max(1.0);
    let metrics = text_html_em_metrics(inner, vertical_lh);
    if metrics.width_em <= f64::EPSILON || metrics.height_em <= f64::EPSILON {
        return 1.0;
    }
    // Vertical stacks size by glyph columns; CSS line-height on the outer text
    // block does not change the upright stack em height used here.
    let mut lo = 1.0;
    let mut hi = (height_px / metrics.height_em).max(1.0);
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        let fits_w = metrics.width_em * mid * margin <= width_px + 0.5;
        let fits_h = metrics.height_em * mid <= height_px + 0.5;
        if fits_w && fits_h {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo.max(1.0)
}

pub(crate) fn text_html_advance_width_px(inner: &str, font_px: f64, vertical_lh: f64) -> f64 {
    if !inner.contains("lbl-vertical") {
        return text_advance_width_px(&html_to_plain_text(inner), font_px);
    }
    text_html_em_metrics(inner, vertical_lh).advance_em * font_px.max(0.0)
}

pub(crate) fn text_html_feed_content_width_px(inner: &str, font_px: f64, vertical_lh: f64) -> f64 {
    if !inner.contains("lbl-vertical") {
        return text_feed_content_width_px(&html_to_plain_text(inner), font_px);
    }
    text_html_em_metrics(inner, vertical_lh).advance_em * font_px.max(0.0) * VISUAL_WIDTH_MARGIN
        + 2.0 * INK_SIDE_BEARING_EM * font_px.max(0.0)
}

fn char_width_em(c: char) -> f64 {
    if c.is_whitespace() {
        return 0.28;
    }
    let cols = c.width().unwrap_or(1).max(1);
    cols as f64 * EM_PER_COLUMN
}

fn line_em_width(line: &str) -> f64 {
    line.chars().map(char_width_em).sum()
}

fn wrapped_line_count(line: &str, font_px: f64, max_width_px: f64) -> usize {
    if line.is_empty() {
        return 1;
    }
    if max_width_px <= f64::EPSILON {
        return 1;
    }

    let mut lines = 1usize;
    let mut current_em = 0.0;

    for word in line.split_whitespace() {
        let word_em = line_em_width(word);
        let space_em = if current_em > 0.0 {
            char_width_em(' ')
        } else {
            0.0
        };
        if (current_em + space_em + word_em) * font_px <= max_width_px {
            current_em += space_em + word_em;
            continue;
        }

        if word_em * font_px <= max_width_px {
            lines += 1;
            current_em = word_em;
            continue;
        }

        for ch in word.chars() {
            let ch_em = char_width_em(ch);
            if current_em > 0.0 && (current_em + ch_em) * font_px > max_width_px {
                lines += 1;
                current_em = ch_em;
            } else {
                current_em += ch_em;
            }
        }
    }

    lines
}

fn text_fits(
    font_px: f64,
    width_px: f64,
    height_px: f64,
    text: &str,
    width_margin: f64,
    line_height: f64,
) -> bool {
    if font_px <= f64::EPSILON {
        return false;
    }
    let fit_width = width_px / width_margin.max(1.0);
    let lh = line_height.max(0.01);
    for ch in text.chars() {
        if !ch.is_whitespace() && char_width_em(ch) * font_px > fit_width + 0.5 {
            return false;
        }
    }
    let mut total_lines = 0usize;
    for line in text.split('\n') {
        total_lines += wrapped_line_count(line, font_px, fit_width);
    }
    if total_lines == 0 {
        total_lines = 1;
    }
    total_lines as f64 * font_px * lh <= height_px + 0.5
}

/// Largest font that fits with explicit width margin and CSS line-height.
pub(crate) fn max_fit_font_px_with_margin_lh(
    width_px: f64,
    height_px: f64,
    text: &str,
    width_margin: f64,
    line_height: f64,
) -> f64 {
    if text.is_empty() {
        return 1.0;
    }
    let lh = line_height.max(0.01);
    let mut lo = 1.0;
    let mut hi = height_px / lh;
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        if text_fits(mid, width_px, height_px, text, width_margin, lh) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo.max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::ROW_TEXT_LINE_HEIGHT;
    use crate::transpile::{LabelFit, LabelStyle, MediaInsetPx};

    #[test]
    fn feed_content_width_covers_visual_margin_and_bearings() {
        let font = 100.0;
        let advance = text_advance_width_px("abc", font);
        let feed = text_feed_content_width_px("abc", font);
        let expected = text_line_width_px("abc", font) + 2.0 * INK_SIDE_BEARING_EM * font;
        assert!(
            (feed - expected).abs() < 1e-9,
            "feed={feed} expected={expected}"
        );
        assert!(
            feed > advance * 1.2,
            "feed must substantially exceed bare advance: feed={feed} advance={advance}"
        );
    }

    #[test]
    fn injected_label_min_width_reads_feed_when_first_property() {
        let html = r#"<style>.lbl-label{--lbl-feed-px:123.45}</style>"#;
        assert_eq!(injected_label_min_width_px(html), Some(123.45));
    }

    #[test]
    fn injected_label_min_width_reads_feed_amid_width_pin() {
        // Oversize clip pins width/max-width in the same rule as --lbl-feed-px.
        let html = r#"<style>.lbl-label{width:200.00px;max-width:200.00px;--lbl-feed-px:200.00;overflow:hidden}</style>"#;
        assert_eq!(injected_label_min_width_px(html), Some(200.0));
    }

    #[test]
    fn short_text_fills_cross_head() {
        let font =
            max_fit_font_px_with_margin_lh(354.0, 142.0, "#1", VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        assert!(font > 120.0, "font={font}");
        assert!(font <= 142.0 / LINE_HEIGHT + 0.01, "font={font}");
    }

    #[test]
    fn vertical_span_metrics_are_tall_and_narrow() {
        let inner =
            r#"<span class="lbl-text-inlines"><span class="lbl-vertical">ABC</span>abcde</span>"#;
        let metrics = text_html_em_metrics(inner, VERTICAL_LINE_HEIGHT);
        assert!(
            (metrics.height_em - 3.0 * VERTICAL_LINE_HEIGHT).abs() < 0.01,
            "height_em={}",
            metrics.height_em
        );
        assert!(
            metrics.width_em < line_em_width("ABCabcde"),
            "width_em={} should be less than flattened ABCabcde",
            metrics.width_em
        );
        assert!(
            (metrics.width_em - (VERTICAL_COL_EM + line_em_width("abcde"))).abs() < 0.01,
            "width_em={}",
            metrics.width_em
        );
    }

    #[test]
    fn vertical_mixed_fit_is_height_limited_not_flattened_width() {
        let inner = r#"<span class="lbl-vertical">ABC</span>abcde"#;
        let flat = max_fit_font_px_with_margin_lh(
            354.0,
            142.0,
            "ABCabcde",
            VISUAL_WIDTH_MARGIN,
            LINE_HEIGHT,
        );
        let vertical = max_fit_font_px_html_with_margin_lh(
            354.0,
            142.0,
            inner,
            VERTICAL_LINE_HEIGHT,
            VISUAL_WIDTH_MARGIN,
            LINE_HEIGHT,
        );
        // Three upright glyphs need ~3em of height; font should approach box_h/3.
        assert!(
            (vertical - 142.0 / 3.0).abs() < 1.0,
            "vertical={vertical} expected ~{}",
            142.0 / 3.0
        );
        // Flattened "ABCabcde" is width-limited on this landscape box and differs.
        assert!(
            (flat - vertical).abs() > 1.0,
            "flat={flat} vertical={vertical}"
        );
    }

    #[test]
    fn vertical_line_height_style_does_not_change_fit_font() {
        let tight = r#"<span class="lbl-vertical">ABC</span>"#;
        let loose = r#"<span class="lbl-vertical" style="--lbl-vertical-spacing:1.5">ABC</span>"#;
        let tight_font = max_fit_font_px_html_with_margin_lh(
            354.0,
            142.0,
            tight,
            VERTICAL_LINE_HEIGHT,
            VISUAL_WIDTH_MARGIN,
            LINE_HEIGHT,
        );
        let loose_font = max_fit_font_px_html_with_margin_lh(
            354.0,
            142.0,
            loose,
            VERTICAL_LINE_HEIGHT,
            VISUAL_WIDTH_MARGIN,
            LINE_HEIGHT,
        );
        assert!(
            (loose_font - tight_font).abs() < 0.01,
            "loose={loose_font} tight={tight_font}"
        );
    }

    #[test]
    fn lone_char_on_tall_narrow_media_is_width_limited() {
        let width = 1135.0;
        let height = 4015.0;
        let font =
            max_fit_font_px_with_margin_lh(width, height, "A", VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        assert!(
            font * char_width_em('A') * VISUAL_WIDTH_MARGIN <= width + 1.0,
            "font={font} exceeds width"
        );
        assert!(
            font < height / LINE_HEIGHT - 1.0,
            "font={font} should not be height-limited"
        );
    }

    #[test]
    fn wide_unicode_char_is_width_limited() {
        let width = 1135.0;
        let height = 4015.0;
        let font =
            max_fit_font_px_with_margin_lh(width, height, "字", VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        assert!(
            font * char_width_em('字') * VISUAL_WIDTH_MARGIN <= width + 1.0,
            "font={font} exceeds width"
        );
        assert!(
            font < height / LINE_HEIGHT - 1.0,
            "font={font} should not be height-limited"
        );
    }

    #[test]
    fn long_line_shrinks_to_fit_width() {
        let font_short =
            max_fit_font_px_with_margin_lh(354.0, 142.0, "#1", VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        let font_long = max_fit_font_px_with_margin_lh(
            354.0,
            142.0,
            "User number forty-two please",
            VISUAL_WIDTH_MARGIN,
            LINE_HEIGHT,
        );
        assert!(font_long < font_short, "{font_long} vs {font_short}");
        assert!(text_fits(
            font_long,
            354.0,
            142.0,
            "User number forty-two please",
            VISUAL_WIDTH_MARGIN,
            LINE_HEIGHT,
        ));
    }

    #[test]
    fn lone_text_inner_extracts_nested_color_spans() {
        let body = r#"<div class="lbl-label"><span class="lbl-text"><span class="lbl-text-inlines">Hi <span style="color:#ff0000">there</span></span></span></div>"#;
        let inner = super::lone_text_inner_html(body).expect("inner");
        assert!(inner.contains("Hi"));
        assert!(inner.contains("there"));
        assert!(super::is_fit_measurable_html(&inner));
    }

    #[test]
    fn inline_color_spans_remain_measurable() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_top_px: 0.0,
                padding_right_px: 0.0,
                padding_bottom_px: 0.0,
                padding_left_px: 0.0,
                ..Default::default()
            },
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><span class="lbl-text"><span class="lbl-text-inlines">Hi <span style="color:#ff0000">there</span></span></span></div>"#;
        let css = lone_text_fit_css(body, &opts).expect("css");
        assert!(css.contains("font-size:"), "{css}");
    }

    #[test]
    fn skips_rich_inner_markup() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(100.0),
                height: Some(50.0),
            }),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><div class="lbl-text"><span class="lbl-text" style="font-size:2em">A</span></div></div>"#;
        assert!(lone_text_fit_css(body, &opts).is_none());
    }

    #[test]
    fn injects_px_rule_for_plain_lone_text() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_top_px: 0.0,
                padding_right_px: 0.0,
                padding_bottom_px: 0.0,
                padding_left_px: 0.0,
                ..Default::default()
            },
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let css = lone_text_fit_css(
            r#"<div class="lbl-label"><div class="lbl-text">#1</div></div>"#,
            &opts,
        )
        .expect("css");
        assert!(css.contains("font-size:"), "{css}");
    }

    #[test]
    fn injects_px_rule_for_span_lone_text() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_top_px: 0.0,
                padding_right_px: 0.0,
                padding_bottom_px: 0.0,
                padding_left_px: 0.0,
                ..Default::default()
            },
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let css = lone_text_fit_css(
            r#"<div class="lbl-label"><span class="lbl-text"><span class="lbl-text-inlines">30×20</span></span></div>"#,
            &opts,
        )
        .expect("css");
        assert!(css.contains("font-size:"), "{css}");
        let font: f64 = css
            .split("font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .unwrap()
            .parse()
            .unwrap();
        let tight_lh = LINE_HEIGHT / TIGHT_INTERNAL_SCALE;
        assert!(
            font > 20.0 && font <= 142.0 / tight_lh + 0.01,
            "font={font}"
        );
    }

    #[test]
    fn font_fit_scale_full_is_clipping_safe_not_ink_tight() {
        // Tall box + medium text: user 1.0 is halfway to aggressive ink-tight
        // (clipping-safe full); user ≈133% spends width margin fully.
        let width = 200.0;
        let height = 500.0;
        let text = "Hello World";
        let comfortable =
            max_fit_font_px_with_margin_lh(width, height, text, VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        let at_max_lh = LINE_HEIGHT / TIGHT_INTERNAL_SCALE;
        let tight = max_fit_font_px_with_margin_lh(width, height, text, 1.0, at_max_lh);
        assert!(
            tight > comfortable + 0.5,
            "expected headroom: comfortable={comfortable} tight={tight}"
        );

        let mid_internal = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                // internal 1.0 = comfortable (user = 1/SAFE_FULL)
                font_fit_scale: 1.0 / SAFE_FULL_INTERNAL_SCALE,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        let full = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                font_fit_scale: 1.0,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        let ink_tight = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                font_fit_scale: TIGHT_INTERNAL_SCALE / SAFE_FULL_INTERNAL_SCALE,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        assert!(
            (mid_internal.font_px - comfortable).abs() < 0.05,
            "mid_internal={:?} comfortable={comfortable}",
            mid_internal
        );
        // User 1.0 → internal 1.5 → t = 0.5 between comfortable and tight.
        let half_lh = LINE_HEIGHT / (1.0 + (TIGHT_INTERNAL_SCALE - 1.0) * 0.5);
        let half_margin = VISUAL_WIDTH_MARGIN + (1.0 - VISUAL_WIDTH_MARGIN) * 0.5;
        let expected_full =
            max_fit_font_px_with_margin_lh(width, height, text, half_margin, half_lh);
        assert!(
            (full.font_px - expected_full).abs() < 0.05,
            "full={:?} expected={expected_full}",
            full
        );
        assert!(
            (full.line_height - half_lh).abs() < 1e-9,
            "lh={}",
            full.line_height
        );
        assert!(
            (ink_tight.font_px - tight).abs() < 0.05,
            "ink_tight={:?} tight={tight}",
            ink_tight
        );
        assert!(
            (ink_tight.line_height - at_max_lh).abs() < 1e-9,
            "lh={}",
            ink_tight.line_height
        );
        assert!(
            full.font_px * full.line_height <= height + 0.5,
            "line box overflows: {:?}",
            full
        );
    }

    #[test]
    fn font_fit_scale_full_grows_height_bound_ink() {
        // Height-bound text: at padded mid (internal 1.0) the line box fills
        // the head but glyph ink sits short; 100% (internal 1.5) grows ~1.5×.
        let width = 2000.0;
        let height = 100.0;
        let text = "X";
        let comfortable =
            max_fit_font_px_with_margin_lh(width, height, text, VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        let full = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                font_fit_scale: 1.0,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        assert!(
            (full.font_px - comfortable * SAFE_FULL_INTERNAL_SCALE).abs() < 0.5,
            "full={:?} comfortable={comfortable}",
            full
        );
        assert!(
            full.font_px * full.line_height <= height + 0.5,
            "line box overflows: {:?}",
            full
        );
        assert!(full.font_px > comfortable + 1.0);
    }

    #[test]
    fn font_fit_scale_above_tight_multiplies_past_ink_tight() {
        let width = 2000.0;
        let height = 100.0;
        let text = "X";
        let ink_tight_user = TIGHT_INTERNAL_SCALE / SAFE_FULL_INTERNAL_SCALE;
        let tight = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                font_fit_scale: ink_tight_user,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        let double_past_tight = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                font_fit_scale: ink_tight_user * 2.0,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        assert!(
            (double_past_tight.font_px - tight.font_px * 2.0).abs() < 0.05,
            "tight={:?} double={:?}",
            tight,
            double_past_tight
        );
        assert!((double_past_tight.line_height - tight.line_height).abs() < 1e-9);
    }

    #[test]
    fn font_fit_scale_shrinks_below_mid_scale() {
        // User 0.25 → internal 0.375 → 0.375× the padded (internal-1.0) max.
        let width = 354.0;
        let height = 142.0;
        let text = "#1";
        let comfortable =
            max_fit_font_px_with_margin_lh(width, height, text, VISUAL_WIDTH_MARGIN, LINE_HEIGHT);
        let quarter = fit_text_font_px(
            width,
            height,
            text,
            &TranspileOptions {
                font_fit_scale: 0.25,
                ..Default::default()
            },
            LINE_HEIGHT,
        );
        assert!(
            (quarter.font_px - comfortable * (0.25 * SAFE_FULL_INTERNAL_SCALE)).abs() < 0.05,
            "quarter={:?} comfortable={comfortable}",
            quarter
        );
        assert!((quarter.line_height - LINE_HEIGHT).abs() < 1e-9);
    }

    fn parse_row_text_font_px(css: &str) -> f64 {
        css.split("font-size:")
            .nth(1)
            .unwrap()
            .split("px")
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn font_fit_scale_row_text_stays_within_box_at_full() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_top_px: 0.0,
                padding_right_px: 0.0,
                padding_bottom_px: 0.0,
                padding_left_px: 0.0,
                qr_size_px: 160.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            font_fit_scale: 1.0,
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x"></div></div></div>"#;
        let mid_css = row_text_qr_fit_css(
            body,
            &TranspileOptions {
                font_fit_scale: 0.5,
                ..opts.clone()
            },
        )
        .expect("mid css");
        let full_css = row_text_qr_fit_css(body, &opts).expect("full css");
        let mid = parse_row_text_font_px(&mid_css);
        let full = parse_row_text_font_px(&full_css);
        assert!(full >= mid - 0.05, "mid={mid} full={full}");
        // User 0.5 → internal 0.75; user 1.0 → internal 1.5 → up to ~2× the 50% font.
        assert!(
            full <= mid * 2.0 + 0.05,
            "mid={mid} full={full} should not exceed ~2× at remapped 50%→100%"
        );
        // Line box may use a tightened line-height; font alone can exceed height/1.1.
        let full_lh_scale = 1.0 + (TIGHT_INTERNAL_SCALE - 1.0) * 0.5;
        assert!(
            full <= 142.0 / (ROW_TEXT_LINE_HEIGHT / full_lh_scale) + 0.5,
            "full={full} exceeds remapped row line-box budget"
        );
    }

    #[test]
    fn font_fit_scale_shrinks_row_text_beside_qr() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_top_px: 0.0,
                padding_right_px: 0.0,
                padding_bottom_px: 0.0,
                padding_left_px: 0.0,
                qr_size_px: 160.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            font_fit_scale: 0.25,
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x"></div></div></div>"#;
        let mid_css = row_text_qr_fit_css(
            body,
            &TranspileOptions {
                font_fit_scale: 0.5,
                ..opts.clone()
            },
        )
        .expect("mid css");
        let quarter_css = row_text_qr_fit_css(body, &opts).expect("quarter css");
        let mid = parse_row_text_font_px(&mid_css);
        let quarter = parse_row_text_font_px(&quarter_css);
        assert!(
            (quarter - mid * 0.5).abs() < 0.05,
            "mid={mid} quarter={quarter}"
        );
    }

    #[test]
    fn smaller_explicit_qr_yields_larger_row_text() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_top_px: 0.0,
                padding_right_px: 0.0,
                padding_bottom_px: 0.0,
                padding_left_px: 0.0,
                qr_size_px: 160.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        // Explicit data-width keeps QR fixed; grown QR takes a weighted share.
        let small_qr_body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x" data-width="50" style="width:50px;height:50px;flex:0 0 auto"></div></div></div>"#;
        let large_qr_body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x"></div></div></div>"#;
        let small_css = row_text_qr_fit_css(small_qr_body, &opts).expect("small qr css");
        let large_css = row_text_qr_fit_css(large_qr_body, &opts).expect("large qr css");
        let font_small_qr = parse_row_text_font_px(&small_css);
        let font_large_qr = parse_row_text_font_px(&large_css);
        assert!(
            font_small_qr > font_large_qr,
            "small_qr={font_small_qr} large_qr={font_large_qr}"
        );
    }
}
