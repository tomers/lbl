//! Compute the largest font size for a lone `.lbl-text` block on fixed media.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::transpile::{TranspileOptions, ViewportPx};

/// Matches `.lbl-label` containing a single `.lbl-text` child.
static LONE_TEXT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?is)^\s*<div\s+class="lbl-label"[^>]*>\s*<div\s+class="lbl-text"[^>]*>([\s\S]*?)</div>\s*</div>\s*$"#,
    )
    .expect("lone text regex")
});

static ANY_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").expect("any tag regex"));

static BR_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<br\s*/?>").expect("br tag regex"));

/// Line height for lone-text auto-fit (must match [`crate::assets::LABEL_FIT_TEXT_CSS`]).
pub const LINE_HEIGHT: f64 = 1.1;

/// CSS rule setting a transpile-time font size, when viewport geometry is known.
pub fn lone_text_fit_css(body: &str, opts: &TranspileOptions) -> Option<String> {
    let (width, height) = fit_box_px(opts)?;
    let caps = LONE_TEXT_RE.captures(body)?;
    let inner = caps.get(1)?.as_str();
    if !is_plain_text_html(inner) {
        return None;
    }
    let text = html_to_plain_text(inner);
    let font_px = max_fit_font_px(width, height, &text);
    Some(format!(
        ".lbl-label>.lbl-text:only-child{{font-size:{font_px:.2}px}}\n"
    ))
}

fn fit_box_px(opts: &TranspileOptions) -> Option<(f64, f64)> {
    let ViewportPx { width, height } = opts.viewport.as_ref()?;
    let width = width.filter(|w| *w > f64::EPSILON)?;
    let height = height.filter(|h| *h > f64::EPSILON)?;
    let scale = opts.label_fit_scale.clamp(0.01, 1.0);
    let pad = opts.style.padding_px.max(0.0);
    let inset = opts.media_inset;
    let inner_w = (width - inset.cross_start - inset.cross_end).max(0.0);
    let inner_h = (height - inset.start - inset.end).max(0.0);
    let box_w = inner_w * scale - 2.0 * pad;
    let box_h = inner_h * scale - 2.0 * pad;
    if box_w <= f64::EPSILON || box_h <= f64::EPSILON {
        return None;
    }
    Some((box_w, box_h))
}

fn is_plain_text_html(inner: &str) -> bool {
    !BR_TAG_RE.replace_all(inner, "").contains('<')
}

fn html_to_plain_text(inner: &str) -> String {
    let normalized = inner
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n");
    ANY_TAG_RE.replace_all(&normalized, "").to_string()
}

fn char_width_em(c: char) -> f64 {
    match c {
        'i' | 'l' | '1' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' => 0.35,
        'M' | 'W' | '@' | '%' | '#' => 0.65,
        ' ' => 0.28,
        _ => 0.55,
    }
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
    let mut total_lines = 0usize;
    for line in text.split('\n') {
        total_lines += wrapped_line_count(line, font_px, width_px);
    }
    if total_lines == 0 {
        total_lines = 1;
    }
    total_lines as f64 * font_px * LINE_HEIGHT <= height_px + 0.5
}

fn max_fit_font_px(width_px: f64, height_px: f64, text: &str) -> f64 {
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
}
