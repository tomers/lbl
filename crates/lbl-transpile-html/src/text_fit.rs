//! Compute the largest font size for a lone `.lbl-text` block on fixed media.

use once_cell::sync::Lazy;
use regex::Regex;
use unicode_width::UnicodeWidthChar;

use crate::transpile::{TranspileOptions, ViewportPx};

static ANY_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").expect("any tag regex"));

/// Line height for lone-text auto-fit (must match [`crate::assets::LABEL_FIT_TEXT_CSS`]).
pub const LINE_HEIGHT: f64 = 1.1;

/// Ink often extends past estimated advance width (diagonal strokes, serifs).
const VISUAL_WIDTH_MARGIN: f64 = 1.35;

/// Estimated em width of one terminal column at the transpiled font size.
const EM_PER_COLUMN: f64 = 0.55;

pub(crate) fn scaled_fit_px(px: f64, opts: &TranspileOptions) -> f64 {
    (px * opts.font_fit_scale.clamp(0.01, 5.0)).max(1.0)
}

/// Parse the auto-fit font size actually injected into transpiled HTML.
pub fn injected_fit_font_px(html: &str) -> Option<f64> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\.lbl-(?:label>\.lbl-text:only-child|row>\.lbl-text)\{font-size:([\d.]+)px\}")
            .expect("injected fit font regex")
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

pub(crate) fn fit_box_px(opts: &TranspileOptions) -> Option<(f64, f64)> {
    let ViewportPx { width, height } = opts.viewport.as_ref()?;
    let width = width.filter(|w| *w > f64::EPSILON);
    let height = height.filter(|h| *h > f64::EPSILON);
    if width.is_none() && height.is_none() {
        return None;
    }
    const UNBOUNDED_AXIS_PX: f64 = 1.0e6;
    let scale = opts.label_fit_scale.clamp(0.01, 1.0);
    let pad = opts.style.padding_px.max(0.0);
    let border = opts.style.border_width_px.max(0.0);
    let inset = opts.media_inset;
    let box_w = if let Some(w) = width {
        let inner_w = (w - inset.cross_start - inset.cross_end).max(0.0);
        inner_w * scale - 2.0 * pad - 2.0 * border
    } else {
        UNBOUNDED_AXIS_PX
    };
    let box_h = if let Some(h) = height {
        let inner_h = (h - inset.start - inset.end).max(0.0);
        inner_h * scale - 2.0 * pad - 2.0 * border
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

fn text_fits(font_px: f64, width_px: f64, height_px: f64, text: &str) -> bool {
    if font_px <= f64::EPSILON {
        return false;
    }
    let fit_width = width_px / VISUAL_WIDTH_MARGIN;
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
    total_lines as f64 * font_px * LINE_HEIGHT <= height_px + 0.5
}

pub(crate) fn max_fit_font_px(width_px: f64, height_px: f64, text: &str) -> f64 {
    if text.is_empty() {
        return 1.0;
    }
    let mut lo = 1.0;
    let mut hi = height_px / LINE_HEIGHT;
    for _ in 0..48 {
        let mid = (lo + hi) / 2.0;
        if text_fits(mid, width_px, height_px, text) {
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
    use crate::transpile::{LabelFit, LabelStyle, MediaInsetPx};

    #[test]
    fn short_text_fills_cross_head() {
        let font = max_fit_font_px(354.0, 142.0, "#1");
        assert!(font > 120.0, "font={font}");
        assert!(font <= 142.0 / LINE_HEIGHT + 0.01, "font={font}");
    }

    #[test]
    fn lone_char_on_tall_narrow_media_is_width_limited() {
        let width = 1135.0;
        let height = 4015.0;
        let font = max_fit_font_px(width, height, "A");
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
        let font = max_fit_font_px(width, height, "字");
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
        let font_short = max_fit_font_px(354.0, 142.0, "#1");
        let font_long = max_fit_font_px(354.0, 142.0, "User number forty-two please");
        assert!(font_long < font_short, "{font_long} vs {font_short}");
        assert!(text_fits(
            font_long,
            354.0,
            142.0,
            "User number forty-two please"
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
                padding_px: 0.0,
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
                padding_px: 0.0,
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
                padding_px: 0.0,
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
            .unwrap()
            .trim_end_matches("px}\n")
            .parse()
            .unwrap();
        assert!(
            font > 20.0 && font <= 142.0 / LINE_HEIGHT + 0.01,
            "font={font}"
        );
    }

    #[test]
    fn font_fit_scale_can_exceed_maximum_fit() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Fill,
            viewport: Some(ViewportPx {
                width: Some(354.0),
                height: Some(142.0),
            }),
            style: LabelStyle {
                padding_px: 0.0,
                qr_size_px: 160.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            font_fit_scale: 1.5,
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x"></div></div></div>"#;
        let full_css = row_text_qr_fit_css(
            body,
            &TranspileOptions {
                font_fit_scale: 1.0,
                ..opts.clone()
            },
        )
        .expect("full css");
        let enlarged_css = row_text_qr_fit_css(body, &opts).expect("enlarged css");
        let parse_font = |css: &str| -> f64 {
            css.split("font-size:")
                .nth(1)
                .unwrap()
                .trim_end_matches("px}\n")
                .parse()
                .unwrap()
        };
        let full = parse_font(&full_css);
        let enlarged = parse_font(&enlarged_css);
        assert!(
            (enlarged - full * 1.5).abs() < 0.05,
            "full={full} enlarged={enlarged}"
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
                padding_px: 0.0,
                qr_size_px: 160.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            font_fit_scale: 0.5,
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x"></div></div></div>"#;
        let full_css = row_text_qr_fit_css(
            body,
            &TranspileOptions {
                font_fit_scale: 1.0,
                ..opts.clone()
            },
        )
        .expect("full css");
        let half_css = row_text_qr_fit_css(body, &opts).expect("half css");
        let parse_font = |css: &str| -> f64 {
            css.split("font-size:")
                .nth(1)
                .unwrap()
                .trim_end_matches("px}\n")
                .parse()
                .unwrap()
        };
        let full = parse_font(&full_css);
        let half = parse_font(&half_css);
        assert!((half - full * 0.5).abs() < 0.05, "full={full} half={half}");
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
                padding_px: 0.0,
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
        let parse_font = |css: &str| -> f64 {
            css.split("font-size:")
                .nth(1)
                .unwrap()
                .trim_end_matches("px}\n")
                .parse()
                .unwrap()
        };
        let font_small_qr = parse_font(&small_css);
        let font_large_qr = parse_font(&large_css);
        assert!(
            font_small_qr > font_large_qr,
            "small_qr={font_small_qr} large_qr={font_large_qr}"
        );
    }
}
