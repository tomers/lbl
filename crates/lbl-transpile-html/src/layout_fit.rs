//! Fill-mode layout allocation: share the printable box among text, barcodes, QR, and images.
//!
//! Uses the existing flex row/column structure from authoring HTML (`.lbl-row` with
//! sibling `.lbl-text` / `.lbl-barcode` / `.lbl-qr` / `img` children). Transpile-time sizing
//! sets font sizes and `data-fit-*` dimensions so headless rendering is deterministic.

use once_cell::sync::Lazy;
use regex::Regex;

use lbl_text::BarcodeHeightMode;

use crate::assets::ROW_TEXT_LINE_HEIGHT;
use crate::text_fit::{
    fit_box_px, html_to_plain_text, is_fit_measurable_html, max_fit_font_px, max_fit_font_px_html,
    scaled_fit_px, text_html_advance_width_px, text_html_feed_content_width_px, text_line_width_px,
    INK_SIDE_BEARING_EM, LINE_HEIGHT, VERTICAL_LINE_HEIGHT,
};
use crate::transpile::TranspileOptions;

const WEIGHT_TEXT: f64 = 1.0;
const WEIGHT_BARCODE: f64 = 1.5;
const WEIGHT_QR: f64 = 1.0;
/// Safety margin so ink and JsBarcode caption descenders stay inside the label box.
const ROW_FIT_SAFETY: f64 = 0.90;
/// Extra ascender/descender allowance for row text beside codes (serif glyphs).
const ROW_TEXT_INK_FACTOR: f64 = 1.08;
/// Continuous content-fit: 1D barcode feed width as a multiple of head height when
/// the payload length is not available on the layout child (scannable, not stamp-sized).
const CONTINUOUS_BARCODE_FEED_ASPECT: f64 = 2.75;

static DATA_WIDTH_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\bdata-width\s*=\s*"(\d+)""#).expect("data-width regex"));

/// CSS injected for Fill-mode rows so flex children share the label box.
/// Stretch rules live in [`crate::assets::LABEL_FIT_ROW_CSS`] (always on in Fill).
pub const LABEL_FIT_ROW_CHILD_CSS: &str = r#"
.lbl-row>.lbl-text{
  flex:0 0 auto;
  min-width:0;
  align-self:center;
  overflow:visible;
  text-align:center;
}
.lbl-row>.lbl-barcode,.lbl-row>.lbl-qr,.lbl-row>img{
  flex:0 0 auto;
  align-self:center;
  overflow:visible;
}
"#;

/// CSS for Fill-mode labels with multiple stacked rows (column layout).
pub const LABEL_FIT_COLUMN_CSS: &str = r#"
.lbl-label>:not(:only-child){
  flex:1 1 0;
  min-height:0;
}
.lbl-label>.lbl-text:not(:only-child){
  display:flex;
  flex-direction:column;
  justify-content:center;
  align-items:center;
  overflow:hidden;
  line-height:1.1;
  text-align:center;
}
"#;

/// Result of Fill layout: patched body plus extra CSS rules.
#[derive(Debug, Clone, Default)]
pub struct LayoutFit {
    pub body: String,
    pub css: String,
    pub font_px: Option<f64>,
}

/// Apply Fill-mode shared sizing to a rewritten label body (after QR/barcode rewrite).
pub fn apply_layout_fit(body: &str, opts: &TranspileOptions) -> LayoutFit {
    let (box_w, box_h) = match fit_box_px(opts) {
        Some(b) => b,
        None => {
            return LayoutFit {
                body: body.to_string(),
                ..Default::default()
            };
        }
    };

    let Some(inner) = label_inner(body) else {
        return LayoutFit {
            body: body.to_string(),
            ..Default::default()
        };
    };

    let children = parse_top_children(inner);
    if children.is_empty() {
        return LayoutFit {
            body: body.to_string(),
            ..Default::default()
        };
    }

    let gap = opts.style.element_gap_px.max(0.0);
    let mut css = String::from(LABEL_FIT_ROW_CHILD_CSS);

    let result = if children.len() == 1 {
        fit_lone(&children[0], body, box_w, box_h, opts, false)
    } else if let [Child::Row {
        children: row_kids, ..
    }] = children.as_slice()
    {
        fit_row(row_kids, body, box_w, box_h, gap, opts, None)
    } else {
        fit_column(&children, body, box_w, box_h, gap, opts)
    };

    css.push_str(&result.css);
    LayoutFit {
        body: result.body,
        css,
        font_px: result.font_px,
    }
}

/// Size lone text to the fixed head axis when continuous media leaves the feed
/// axis unbounded (content fit).
pub fn apply_content_head_text_fit(body: &str, opts: &TranspileOptions) -> LayoutFit {
    let viewport = match opts.viewport.as_ref() {
        Some(v) => v,
        None => {
            return LayoutFit {
                body: body.to_string(),
                ..Default::default()
            };
        }
    };
    let width_known = viewport.width.filter(|w| *w > f64::EPSILON).is_some();
    let height_known = viewport.height.filter(|h| *h > f64::EPSILON).is_some();
    if width_known == height_known {
        return LayoutFit {
            body: body.to_string(),
            ..Default::default()
        };
    }

    let (box_w, box_h) = match fit_box_px(opts) {
        Some(b) => b,
        None => {
            return LayoutFit {
                body: body.to_string(),
                ..Default::default()
            };
        }
    };

    let Some(inner) = label_inner(body) else {
        return LayoutFit {
            body: body.to_string(),
            ..Default::default()
        };
    };

    let children = parse_top_children(inner);
    if children.len() != 1 {
        return LayoutFit {
            body: body.to_string(),
            ..Default::default()
        };
    }

    fit_lone(&children[0], body, box_w, box_h, opts, true)
}

fn fit_lone(
    child: &Child,
    body: &str,
    box_w: f64,
    box_h: f64,
    opts: &TranspileOptions,
    // Continuous content-fit: claim inline space for glyph ink past advance so
    // feed-end stock padding stays blank.
    claim_ink_bearings: bool,
) -> LayoutFit {
    match child {
        Child::Text { inner } => {
            if !is_fit_measurable_html(inner) {
                return LayoutFit {
                    body: body.to_string(),
                    ..Default::default()
                };
            }
            let font_px = scaled_fit_px(
                max_fit_font_px_html(box_w, box_h, inner, VERTICAL_LINE_HEIGHT),
                opts,
            );
            let text_w = if claim_ink_bearings {
                text_html_feed_content_width_px(inner, font_px, VERTICAL_LINE_HEIGHT)
            } else {
                // Fill/fixed: VISUAL_WIDTH_MARGIN already keeps ink inside the box.
                text_html_advance_width_px(inner, font_px, VERTICAL_LINE_HEIGHT)
            };
            let content_w =
                text_w + opts.style.padding_x_px() + 2.0 * opts.style.border_width_px.max(0.0);
            // Match text_fit::LINE_HEIGHT (1.1). Base .lbl-label uses 1.3, which
            // would clip head-fitted glyphs. nowrap keeps continuous feed text on
            // one line so a narrow preview iframe cannot re-wrap and overflow.
            // --lbl-feed-px is for iframe sizing; do not set min-width or the tape
            // grows a hollow feed gap.
            let text_css = if claim_ink_bearings {
                format!(
                    ".lbl-label>.lbl-text:only-child{{font-size:{font_px:.2}px;line-height:{lh};\
                     white-space:nowrap;padding-inline:{bearing:.4}em}}\n\
                     .lbl-label{{--lbl-feed-px:{content_w:.2}}}\n",
                    font_px = font_px,
                    lh = LINE_HEIGHT,
                    bearing = INK_SIDE_BEARING_EM,
                    content_w = content_w,
                )
            } else {
                format!(
                    ".lbl-label>.lbl-text:only-child{{font-size:{font_px:.2}px;line-height:{lh};white-space:nowrap}}\n\
                     .lbl-label{{--lbl-feed-px:{content_w:.2}}}\n",
                    font_px = font_px,
                    lh = LINE_HEIGHT,
                    content_w = content_w,
                )
            };
            LayoutFit {
                body: body.to_string(),
                css: text_css,
                font_px: Some(font_px),
            }
        }
        Child::Barcode { height_mode, is_2d } => {
            let bar_w = if row_main_axis_unbounded(opts) {
                continuous_mark_feed_px(box_h, *is_2d, opts)
            } else {
                box_w
            };
            let (w, bar_h, _total_h, caption_font) =
                barcode_fit_dims(bar_w, box_h, None, *height_mode, *is_2d, opts);
            let feed = w + opts.style.padding_x_px() + 2.0 * opts.style.border_width_px.max(0.0);
            let mut css = format!(
                ".lbl-label>.lbl-barcode:only-child{{width:{w:.2}px;{}}}\n",
                barcode_container_css()
            );
            if claim_ink_bearings {
                css.push_str(&format!(".lbl-label{{--lbl-feed-px:{feed:.2}}}\n"));
            }
            LayoutFit {
                body: patch_nth_code(body, "lbl-barcode", 0, w, bar_h, caption_font),
                css,
                font_px: None,
            }
        }
        Child::Qr { explicit_size, .. } => {
            if *explicit_size {
                return LayoutFit {
                    body: body.to_string(),
                    ..Default::default()
                };
            }
            let s = scaled_fit_px(box_w.min(box_h), opts);
            let mut css =
                format!(".lbl-label>.lbl-qr:only-child{{width:{s:.2}px;height:{s:.2}px}}\n");
            if claim_ink_bearings {
                let feed =
                    s + opts.style.padding_x_px() + 2.0 * opts.style.border_width_px.max(0.0);
                css.push_str(&format!(".lbl-label{{--lbl-feed-px:{feed:.2}}}\n"));
            }
            LayoutFit {
                body: patch_nth_code(body, "lbl-qr", 0, s, s, None),
                css,
                font_px: None,
            }
        }
        Child::Img => {
            // Same box as lone QR: fill the shorter printable axis so icons and
            // logos are visible on continuous tape (intrinsic SVG is often 24px).
            let s = scaled_fit_px(box_w.min(box_h), opts);
            let feed = s + opts.style.padding_x_px() + 2.0 * opts.style.border_width_px.max(0.0);
            LayoutFit {
                body: body.to_string(),
                css: format!(
                    ".lbl-label>img:only-child{{width:{s:.2}px;height:{s:.2}px;object-fit:contain;display:block}}\n\
                     .lbl-label{{--lbl-feed-px:{feed:.2}}}\n"
                ),
                font_px: None,
            }
        }
        Child::Row { children, .. } => fit_row(
            children,
            body,
            box_w,
            box_h,
            opts.style.element_gap_px.max(0.0),
            opts,
            None,
        ),
        Child::Other => LayoutFit {
            body: body.to_string(),
            ..Default::default()
        },
    }
}

/// Landscape continuous: feed (width) unknown, head (height) known. Fill-style
/// row allocation must not treat [`crate::text_fit::fit_box_px`]'s unbounded
/// sentinel as a real box to share — text would claim ~1e6px and preview collapses.
fn row_main_axis_unbounded(opts: &TranspileOptions) -> bool {
    match opts.viewport.as_ref() {
        Some(v) => {
            v.width.filter(|w| *w > f64::EPSILON).is_none()
                && v.height.filter(|h| *h > f64::EPSILON).is_some()
        }
        None => false,
    }
}

fn continuous_mark_feed_px(box_h: f64, is_2d: bool, opts: &TranspileOptions) -> f64 {
    if is_2d {
        scaled_fit_px(box_h, opts)
    } else {
        scaled_fit_px(box_h * CONTINUOUS_BARCODE_FEED_ASPECT, opts)
    }
}

fn fit_row(
    row_kids: &[Child],
    body: &str,
    box_w: f64,
    box_h: f64,
    gap: f64,
    opts: &TranspileOptions,
    forced_font: Option<f64>,
) -> LayoutFit {
    if row_kids.is_empty() {
        return LayoutFit {
            body: body.to_string(),
            ..Default::default()
        };
    }

    if row_main_axis_unbounded(opts) {
        return fit_row_continuous_feed(row_kids, body, box_h, gap, opts, forced_font);
    }

    let n_gaps = (row_kids.len() - 1) as f64;
    let (widths, avail, grow_len) = row_width_layout(row_kids, box_w, gap, n_gaps, opts);

    let mut css = String::new();
    let mut font_px = forced_font.or_else(|| row_text_fit_font_px(row_kids, &widths, box_h, opts));

    font_px = font_px.map(|fp| {
        if forced_font.is_some() {
            fp
        } else if row_has_barcode(row_kids) {
            finalize_row_font_px(fp, box_h, row_kids, &widths, avail, grow_len, opts)
        } else {
            fp
        }
    });

    if let Some(px) = font_px {
        css.push_str(&format!(".lbl-row>.lbl-text{{font-size:{px:.2}px}}\n"));
    }

    let mut patched = body.to_string();
    let mut barcode_n = 0usize;
    let mut qr_n = 0usize;

    for (i, kid) in row_kids.iter().enumerate() {
        let w = widths[i].unwrap_or(avail / grow_len.max(1) as f64);
        match kid {
            Child::Barcode { height_mode, is_2d } => {
                let (fw, bar_h, _total_h, caption_font) =
                    barcode_fit_dims(w, box_h, font_px, *height_mode, *is_2d, opts);
                patched =
                    patch_nth_code(&patched, "lbl-barcode", barcode_n, fw, bar_h, caption_font);
                css.push_str(&format!(
                    ".lbl-row>.lbl-barcode:nth-child({}){{width:{fw:.2}px;{};flex:0 0 auto}}\n",
                    i + 1,
                    barcode_container_css()
                ));
                barcode_n += 1;
            }
            Child::Qr {
                explicit_size: false,
                ..
            } => {
                let s = scaled_fit_px(w.min(box_h), opts);
                patched = patch_nth_code(&patched, "lbl-qr", qr_n, s, s, None);
                css.push_str(&format!(
                    ".lbl-row>.lbl-qr:nth-child({}){{width:{s:.2}px;height:{s:.2}px;flex:0 0 auto}}\n",
                    i + 1
                ));
                qr_n += 1;
            }
            Child::Qr {
                explicit_size: true,
                ..
            } => {
                qr_n += 1;
            }
            Child::Text { .. } => {
                css.push_str(&format!(
                    ".lbl-row>.lbl-text:nth-child({}){{width:{w:.2}px;flex:0 0 auto;text-align:center}}\n",
                    i + 1
                ));
            }
            Child::Img => {
                let s = scaled_fit_px(w.min(box_h), opts);
                css.push_str(&format!(
                    ".lbl-row>img:nth-child({}){{width:{s:.2}px;height:{s:.2}px;object-fit:contain;flex:0 0 auto}}\n",
                    i + 1
                ));
            }
            Child::Other | Child::Row { .. } => {}
        }
    }

    LayoutFit {
        body: patched,
        css,
        font_px,
    }
}

/// Content-head row on landscape continuous media: size each cell from ink /
/// head budget, then pin `--lbl-feed-px` to the sum (plus gaps and chrome).
fn fit_row_continuous_feed(
    row_kids: &[Child],
    body: &str,
    box_h: f64,
    gap: f64,
    opts: &TranspileOptions,
    forced_font: Option<f64>,
) -> LayoutFit {
    let text_budget_h = box_h * LINE_HEIGHT / ROW_TEXT_LINE_HEIGHT;
    let mut font_px = forced_font;
    if font_px.is_none() {
        for kid in row_kids {
            let Child::Text { inner } = kid else {
                continue;
            };
            if !is_fit_measurable_html(inner) {
                continue;
            }
            let text = html_to_plain_text(inner);
            if text.trim().is_empty() {
                continue;
            }
            // Feed is free: font is limited by the known head axis (same as lone text).
            let px = scaled_fit_px(
                max_fit_font_px_html(1.0e6, text_budget_h, inner, VERTICAL_LINE_HEIGHT),
                opts,
            );
            font_px = Some(match font_px {
                Some(cur) => cur.min(px),
                None => px,
            });
        }
    }

    if let Some(fp) = font_px.filter(|_| forced_font.is_none() && row_has_barcode(row_kids)) {
        // Height-only shrink (feed widths are content-sized, not a shared box).
        let budget_h = box_h * ROW_FIT_SAFETY;
        let mut fp = fp;
        for _ in 0..16 {
            let mut content_h = fp * ROW_TEXT_LINE_HEIGHT * ROW_TEXT_INK_FACTOR;
            for kid in row_kids {
                match kid {
                    Child::Barcode { height_mode, is_2d } => {
                        let w = continuous_mark_feed_px(box_h, *is_2d, opts);
                        let (_, bar_h, total_h, caption) =
                            barcode_fit_dims(w, box_h, Some(fp), *height_mode, *is_2d, opts);
                        let total = if *is_2d {
                            total_h
                        } else {
                            caption
                                .map(|f| jsbarcode_svg_height(bar_h, f))
                                .unwrap_or(bar_h)
                        };
                        content_h = content_h.max(total);
                    }
                    Child::Qr {
                        explicit_size: false,
                        ..
                    }
                    | Child::Img => {
                        content_h = content_h.max(scaled_fit_px(box_h, opts));
                    }
                    _ => {}
                }
            }
            if content_h <= budget_h * 1.001 {
                break;
            }
            fp = (fp * budget_h / content_h).max(1.0);
        }
        font_px = Some(fp);
    }

    let mark_s = scaled_fit_px(box_h, opts);
    let mut widths: Vec<f64> = Vec::with_capacity(row_kids.len());
    for kid in row_kids {
        let w = match kid {
            Child::Text { inner } => {
                let fp = font_px.unwrap_or(mark_s);
                text_html_feed_content_width_px(inner, fp, VERTICAL_LINE_HEIGHT)
            }
            Child::Img => mark_s,
            Child::Qr {
                explicit_size: true,
                attrs,
                ..
            } => qr_explicit_width(attrs, opts.style.qr_size_px),
            Child::Qr { .. } => mark_s,
            Child::Barcode { is_2d, .. } => continuous_mark_feed_px(box_h, *is_2d, opts),
            Child::Other | Child::Row { .. } => mark_s,
        };
        widths.push(w.max(1.0));
    }

    let mut css = String::new();
    if let Some(px) = font_px {
        css.push_str(&format!(".lbl-row>.lbl-text{{font-size:{px:.2}px}}\n"));
    }

    let mut patched = body.to_string();
    let mut barcode_n = 0usize;
    let mut qr_n = 0usize;
    let mut feed_sum = 0.0;
    let n_gaps = row_kids.len().saturating_sub(1) as f64;

    for (i, kid) in row_kids.iter().enumerate() {
        let w = widths[i];
        feed_sum += w;
        match kid {
            Child::Barcode { height_mode, is_2d } => {
                let (fw, bar_h, _total_h, caption_font) =
                    barcode_fit_dims(w, box_h, font_px, *height_mode, *is_2d, opts);
                // Keep feed pin in sync if barcode_fit_dims clamps width.
                feed_sum += fw - w;
                patched =
                    patch_nth_code(&patched, "lbl-barcode", barcode_n, fw, bar_h, caption_font);
                css.push_str(&format!(
                    ".lbl-row>.lbl-barcode:nth-child({}){{width:{fw:.2}px;{};flex:0 0 auto}}\n",
                    i + 1,
                    barcode_container_css()
                ));
                barcode_n += 1;
            }
            Child::Qr {
                explicit_size: false,
                ..
            } => {
                let s = scaled_fit_px(w.min(box_h), opts);
                feed_sum += s - w;
                patched = patch_nth_code(&patched, "lbl-qr", qr_n, s, s, None);
                css.push_str(&format!(
                    ".lbl-row>.lbl-qr:nth-child({}){{width:{s:.2}px;height:{s:.2}px;flex:0 0 auto}}\n",
                    i + 1
                ));
                qr_n += 1;
            }
            Child::Qr {
                explicit_size: true,
                ..
            } => {
                qr_n += 1;
            }
            Child::Text { .. } => {
                css.push_str(&format!(
                    ".lbl-row>.lbl-text:nth-child({}){{width:{w:.2}px;flex:0 0 auto;text-align:center;white-space:nowrap}}\n",
                    i + 1
                ));
            }
            Child::Img => {
                let s = scaled_fit_px(w.min(box_h), opts);
                feed_sum += s - w;
                css.push_str(&format!(
                    ".lbl-row>img:nth-child({}){{width:{s:.2}px;height:{s:.2}px;object-fit:contain;flex:0 0 auto}}\n",
                    i + 1
                ));
            }
            Child::Other | Child::Row { .. } => {}
        }
    }

    feed_sum +=
        gap * n_gaps + opts.style.padding_x_px() + 2.0 * opts.style.border_width_px.max(0.0);
    css.push_str(&format!(".lbl-label{{--lbl-feed-px:{feed_sum:.2}}}\n"));

    LayoutFit {
        body: patched,
        css,
        font_px,
    }
}

fn column_visual_row_count(children: &[Child]) -> usize {
    children
        .iter()
        .map(|child| match child {
            Child::Text { inner } => {
                let plain = html_to_plain_text(inner);
                plain
                    .split('\n')
                    .filter(|line| !line.trim().is_empty())
                    .count()
                    .max(1)
            }
            _ => 1,
        })
        .sum()
}

fn row_width_layout(
    row_kids: &[Child],
    box_w: f64,
    gap: f64,
    n_gaps: f64,
    opts: &TranspileOptions,
) -> (Vec<Option<f64>>, f64, usize) {
    let mut fixed_w = 0.0;
    let mut grow: Vec<(usize, f64)> = Vec::new();

    for (i, kid) in row_kids.iter().enumerate() {
        match kid {
            Child::Img | Child::Other => {
                fixed_w += opts.style.qr_size_px.min(box_w * 0.2);
            }
            Child::Qr {
                explicit_size: true,
                attrs,
                ..
            } => {
                fixed_w += qr_explicit_width(attrs, opts.style.qr_size_px);
            }
            Child::Text { .. } => grow.push((i, WEIGHT_TEXT)),
            Child::Barcode { .. } => grow.push((i, WEIGHT_BARCODE)),
            Child::Qr { .. } => grow.push((i, WEIGHT_QR)),
            Child::Row { .. } => grow.push((i, WEIGHT_TEXT)),
        }
    }

    let weight_sum: f64 = grow.iter().map(|(_, w)| *w).sum();
    let avail = (box_w - gap * n_gaps - fixed_w).max(1.0);
    let grow_len = grow.len();

    let widths: Vec<Option<f64>> = row_kids
        .iter()
        .enumerate()
        .map(|(i, kid)| match kid {
            Child::Qr {
                explicit_size: true,
                attrs,
                ..
            } => Some(qr_explicit_width(attrs, opts.style.qr_size_px)),
            Child::Img | Child::Other => Some(opts.style.qr_size_px.min(box_w * 0.2)),
            _ => grow
                .iter()
                .find(|(gi, _)| *gi == i)
                .map(|(_, w)| avail * (*w) / weight_sum.max(f64::EPSILON)),
        })
        .collect();

    (widths, avail, grow_len)
}

fn row_max_font_px(
    row_kids: &[Child],
    box_w: f64,
    box_h: f64,
    gap: f64,
    opts: &TranspileOptions,
) -> f64 {
    let n_gaps = row_kids.len().saturating_sub(1) as f64;
    let (widths, avail, grow_len) = row_width_layout(row_kids, box_w, gap, n_gaps, opts);
    if row_has_barcode(row_kids) {
        let hi = (box_h / ROW_TEXT_LINE_HEIGHT).max(1.0);
        finalize_row_font_px(hi, box_h, row_kids, &widths, avail, grow_len, opts)
    } else {
        row_text_fit_font_px(row_kids, &widths, box_h, opts)
            .unwrap_or((box_h / ROW_TEXT_LINE_HEIGHT).max(1.0))
    }
}

fn row_text_fit_font_px(
    row_kids: &[Child],
    widths: &[Option<f64>],
    box_h: f64,
    opts: &TranspileOptions,
) -> Option<f64> {
    let text_budget_h = box_h * LINE_HEIGHT / ROW_TEXT_LINE_HEIGHT;
    let mut font_px: Option<f64> = None;
    for (i, kid) in row_kids.iter().enumerate() {
        let Child::Text { inner } = kid else {
            continue;
        };
        if !is_fit_measurable_html(inner) {
            continue;
        }
        let text = html_to_plain_text(inner);
        let Some(w) = widths[i] else {
            continue;
        };
        if w <= f64::EPSILON || text.trim().is_empty() {
            continue;
        }
        let px = scaled_fit_px(
            max_fit_font_px_html(w, text_budget_h, inner, VERTICAL_LINE_HEIGHT),
            opts,
        );
        font_px = Some(match font_px {
            Some(cur) => cur.min(px),
            None => px,
        });
    }
    font_px
}

fn text_sample_line(inner: &str) -> String {
    let text = html_to_plain_text(inner);
    text.split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .max_by_key(|line| line.chars().count())
        .map(str::to_string)
        .unwrap_or(text)
}

fn column_unified_font(
    children: &[Child],
    box_w: f64,
    slot_h: f64,
    gap: f64,
    opts: &TranspileOptions,
) -> Option<f64> {
    let mut font = f64::INFINITY;
    for child in children {
        match child {
            Child::Text { inner } if is_fit_measurable_html(inner) => {
                let sample = text_sample_line(inner);
                font = font.min(max_fit_font_px(box_w, slot_h, &sample));
            }
            Child::Row {
                children: row_kids, ..
            } => {
                font = font.min(row_max_font_px(row_kids, box_w, slot_h, gap, opts));
            }
            _ => {}
        }
    }
    if font.is_finite() {
        Some(scaled_fit_px(font.max(1.0), opts))
    } else {
        None
    }
}

fn fit_column(
    children: &[Child],
    body: &str,
    box_w: f64,
    box_h: f64,
    gap: f64,
    opts: &TranspileOptions,
) -> LayoutFit {
    let visual_rows = column_visual_row_count(children).max(1);
    let n_gaps = (visual_rows - 1) as f64;
    let slot_h = ((box_h - gap * n_gaps) / visual_rows as f64).max(1.0);
    let unified_font = column_unified_font(children, box_w, slot_h, gap, opts);

    let mut css = String::from(LABEL_FIT_COLUMN_CSS);
    let mut patched = body.to_string();
    let mut barcode_n = 0usize;
    let mut qr_n = 0usize;
    let mut font_px = unified_font;

    for (i, child) in children.iter().enumerate() {
        match child {
            Child::Text { inner } => {
                if is_fit_measurable_html(inner) {
                    let px = unified_font.unwrap_or_else(|| {
                        scaled_fit_px(
                            max_fit_font_px(box_w, slot_h, &text_sample_line(inner)),
                            opts,
                        )
                    });
                    font_px = Some(px);
                    css.push_str(&format!(
                        ".lbl-label>.lbl-text:nth-child({}){{font-size:{px:.2}px}}\n",
                        i + 1
                    ));
                }
            }
            Child::Barcode { height_mode, is_2d } => {
                let (fw, bar_h, _total_h, caption_font) =
                    barcode_fit_dims(box_w, slot_h, font_px, *height_mode, *is_2d, opts);
                patched =
                    patch_nth_code(&patched, "lbl-barcode", barcode_n, fw, bar_h, caption_font);
                css.push_str(&format!(
                    ".lbl-label>.lbl-barcode:nth-child({}){{width:{fw:.2}px;{}}}\n",
                    i + 1,
                    barcode_container_css()
                ));
                barcode_n += 1;
            }
            Child::Qr {
                explicit_size: false,
                ..
            } => {
                let s = scaled_fit_px(box_w.min(slot_h), opts);
                patched = patch_nth_code(&patched, "lbl-qr", qr_n, s, s, None);
                css.push_str(&format!(
                    ".lbl-label>.lbl-qr:nth-child({}){{width:{s:.2}px;height:{s:.2}px}}\n",
                    i + 1
                ));
                qr_n += 1;
            }
            Child::Qr {
                explicit_size: true,
                ..
            } => {
                qr_n += 1;
            }
            Child::Row {
                children: row_kids, ..
            } => {
                let nested = fit_row(row_kids, &patched, box_w, slot_h, gap, opts, unified_font);
                patched = nested.body;
                css.push_str(&nested.css);
                if nested.font_px.is_some() {
                    font_px = nested.font_px;
                }
                barcode_n += count_codes(row_kids, "barcode");
                qr_n += count_codes(row_kids, "qr");
            }
            Child::Img => {
                let s = scaled_fit_px(box_w.min(slot_h), opts);
                css.push_str(&format!(
                    ".lbl-label>img:nth-child({}){{width:{s:.2}px;height:{s:.2}px;object-fit:contain;display:block}}\n",
                    i + 1
                ));
            }
            Child::Other => {}
        }
    }

    LayoutFit {
        body: patched,
        css,
        font_px,
    }
}

fn count_codes(kids: &[Child], kind: &str) -> usize {
    kids.iter()
        .filter(|k| {
            matches!(
                (kind, k),
                ("barcode", Child::Barcode { .. }) | ("qr", Child::Qr { .. })
            )
        })
        .count()
}

fn barcode_fit_dims(
    width: f64,
    box_h: f64,
    font_px: Option<f64>,
    mode: BarcodeHeightMode,
    is_2d: bool,
    opts: &TranspileOptions,
) -> (f64, f64, f64, Option<f64>) {
    if is_2d {
        // Matrix codes: square like QR; no human-readable caption strip.
        let side = scaled_fit_px(width.min(box_h), opts).max(1.0);
        return (side, side, side, None);
    }

    let w = scaled_fit_px(width, opts).max(1.0);
    let base_h = opts.style.barcode_height_px.max(1.0);
    let base_font = opts.style.font_size_px.max(8.0);

    let (bar_h, caption_font_px) = match mode {
        BarcodeHeightMode::Stretch => {
            let budget = box_h * ROW_FIT_SAFETY;
            let bars = (budget * 0.75).max(8.0);
            let caption_font = ((budget - bars - 12.0) / 1.75).max(8.0);
            (bars, Some(caption_font))
        }
        BarcodeHeightMode::Normal => match font_px {
            // Beside auto-fit text: scale bars to the sibling em box, not the
            // configured physical bar height (which stays small on wide labels).
            Some(fp) => {
                let bars = (fp * 0.75).max(scaled_fit_px(base_h, opts).min(fp));
                let caption = (fp * 0.38).max(8.0);
                (bars.min(box_h * 0.85).max(8.0), Some(caption))
            }
            None => (scaled_fit_px(base_h, opts).min(box_h * 0.85).max(8.0), None),
        },
    };

    let caption_h = caption_font_px
        .map(|f| jsbarcode_svg_height(bar_h, f) - bar_h)
        .unwrap_or_else(|| barcode_caption_px(bar_h, base_h, base_font, None));
    let total_h = bar_h + caption_h;
    (w, bar_h, total_h, caption_font_px)
}

/// Approximate rendered JsBarcode SVG height (bars + caption + descenders).
fn jsbarcode_svg_height(bar_h: f64, caption_font: f64) -> f64 {
    bar_h + caption_font * 1.75 + 12.0
}

fn barcode_container_css() -> &'static str {
    "height:auto;overflow:visible"
}

fn barcode_caption_px(bar_h: f64, base_h: f64, base_font: f64, text_font_px: Option<f64>) -> f64 {
    if let Some(fp) = text_font_px {
        return jsbarcode_svg_height(bar_h, (fp * 0.38).max(8.0)) - bar_h;
    }
    jsbarcode_svg_height(bar_h, (base_font * (bar_h / base_h.max(1.0))).max(8.0)) - bar_h
}

fn row_has_barcode(row_kids: &[Child]) -> bool {
    row_kids.iter().any(|k| matches!(k, Child::Barcode { .. }))
}

fn row_item_width(widths: &[Option<f64>], i: usize, avail: f64, grow_len: usize) -> f64 {
    widths[i].unwrap_or(avail / grow_len.max(1) as f64)
}

fn row_content_height(
    box_h: f64,
    font_px: f64,
    row_kids: &[Child],
    widths: &[Option<f64>],
    avail: f64,
    grow_len: usize,
    opts: &TranspileOptions,
) -> f64 {
    let mut h = font_px * ROW_TEXT_LINE_HEIGHT * ROW_TEXT_INK_FACTOR;
    for (i, kid) in row_kids.iter().enumerate() {
        let w = row_item_width(widths, i, avail, grow_len);
        match kid {
            Child::Barcode { height_mode, is_2d } => {
                let (_, bar_h, total_h, caption) =
                    barcode_fit_dims(w, box_h, Some(font_px), *height_mode, *is_2d, opts);
                let total = if *is_2d {
                    total_h
                } else {
                    caption
                        .map(|f| jsbarcode_svg_height(bar_h, f))
                        .unwrap_or(bar_h)
                };
                h = h.max(total);
            }
            Child::Qr {
                explicit_size: false,
                ..
            } => {
                h = h.max(scaled_fit_px(w.min(box_h), opts));
            }
            _ => {}
        }
    }
    h
}

/// Shrink row font (and thus barcode) until text and barcode fit width and height.
fn finalize_row_font_px(
    font_px: f64,
    box_h: f64,
    row_kids: &[Child],
    widths: &[Option<f64>],
    avail: f64,
    grow_len: usize,
    opts: &TranspileOptions,
) -> f64 {
    let budget_h = box_h * ROW_FIT_SAFETY;
    let mut fp = font_px;
    for _ in 0..16 {
        let content_h = row_content_height(box_h, fp, row_kids, widths, avail, grow_len, opts);
        let mut scale = 1.0_f64;
        if content_h > budget_h {
            scale = scale.min(budget_h / content_h);
        }
        for (i, kid) in row_kids.iter().enumerate() {
            let Child::Text { inner } = kid else { continue };
            if !is_fit_measurable_html(inner) {
                continue;
            }
            let text = html_to_plain_text(inner);
            let Some(cell_w) = widths[i] else { continue };
            if cell_w <= f64::EPSILON || text.trim().is_empty() {
                continue;
            }
            let need = text_line_width_px(&text, fp);
            if need > cell_w * 0.94 {
                scale = scale.min((cell_w * 0.94) / need);
            }
        }
        if scale >= 0.999 {
            break;
        }
        fp = (fp * scale).max(1.0);
    }
    fp
}

fn qr_explicit_width(attrs: &str, default: f64) -> f64 {
    DATA_WIDTH_RE
        .captures(attrs)
        .and_then(|c| c[1].parse().ok())
        .unwrap_or(default)
}

fn label_inner(body: &str) -> Option<&str> {
    let body = body.trim();
    let open_end = label_open_tag_end(body)?;
    const CLOSE: &str = "</div>";
    if !body.ends_with(CLOSE) {
        return None;
    }
    Some(body[open_end..body.len() - CLOSE.len()].trim())
}

/// End offset of a root `<div … class="…lbl-label…">` opening tag.
fn label_open_tag_end(body: &str) -> Option<usize> {
    if body.as_bytes().first().is_none_or(|b| *b != b'<') {
        return None;
    }
    if !body[1..].starts_with("div") && !body[1..].starts_with("DIV") {
        return None;
    }
    let gt = body.find('>')?;
    let open = &body[..=gt];
    let class = class_attr_value(open)?;
    if class.split_whitespace().any(|c| c == "lbl-label") {
        Some(gt + 1)
    } else {
        None
    }
}

fn class_attr_value(open_tag: &str) -> Option<&str> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"\bclass\s*=\s*(?:"([^"]*)"|'([^']*)')"#).expect("class attr regex")
    });
    let caps = RE.captures(open_tag)?;
    caps.get(1).or_else(|| caps.get(2)).map(|m| m.as_str())
}

#[derive(Debug, Clone)]
enum Child {
    Text {
        inner: String,
    },
    Barcode {
        height_mode: BarcodeHeightMode,
        is_2d: bool,
    },
    Qr {
        attrs: String,
        explicit_size: bool,
    },
    Img,
    Row {
        children: Vec<Child>,
    },
    Other,
}

fn parse_top_children(inner: &str) -> Vec<Child> {
    let mut children = Vec::new();
    let mut i = 0;
    let bytes = inner.as_bytes();
    while i < inner.len() {
        while i < inner.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= inner.len() {
            break;
        }
        if !inner[i..].starts_with('<') {
            if let Some(rel) = inner[i..].find('<') {
                i += rel;
            } else {
                break;
            }
            continue;
        }
        let Some((consumed, child)) = parse_one_element(&inner[i..]) else {
            break;
        };
        children.push(child);
        i += consumed;
    }
    children
}

fn parse_one_element(html: &str) -> Option<(usize, Child)> {
    if html.starts_with("<div class=\"lbl-row") || html.starts_with("<div class='lbl-row") {
        let (_end, inner_start) = find_open_tag_end(html)?;
        let inner = balanced_element_inner_ref(&html[inner_start..], "div")?;
        let total = inner_start + inner.len() + "</div>".len();
        return Some((
            total,
            Child::Row {
                children: parse_top_children(inner),
            },
        ));
    }

    for tag in ["span", "div"] {
        let exact = format!(r#"<{tag} class="lbl-text">"#);
        let prefix = format!(r#"<{tag} class="lbl-text" "#);
        if html.starts_with(&exact) {
            let rest = &html[exact.len()..];
            let inner = balanced_element_inner(rest, tag)?;
            return Some((
                exact.len() + inner.len() + format!("</{tag}>").len(),
                Child::Text { inner },
            ));
        }
        if html.starts_with(&prefix) {
            let gt = html[prefix.len()..].find('>')? + prefix.len();
            let rest = &html[gt + 1..];
            let inner = balanced_element_inner(rest, tag)?;
            return Some((
                gt + 1 + inner.len() + format!("</{tag}>").len(),
                Child::Text { inner },
            ));
        }
    }

    for tag in ["div", "span"] {
        let prefix = format!("<{tag} class=\"lbl-barcode\"");
        if !html.starts_with(&prefix) {
            continue;
        }
        let (open_end, _) = find_open_tag_end(html)?;
        let open_tag = &html[..open_end];
        let height_mode = barcode_height_mode_from_attrs(open_tag);
        let is_2d = barcode_is_2d_from_attrs(open_tag);
        let close = format!("</{tag}>");
        if rest_is_empty_element(html, open_end, &close) {
            return Some((
                open_end + close.len(),
                Child::Barcode { height_mode, is_2d },
            ));
        }
    }

    for tag in ["div", "span"] {
        let prefix = format!("<{tag} class=\"lbl-qr\"");
        if !html.starts_with(&prefix) {
            continue;
        }
        let (open_end, _) = find_open_tag_end(html)?;
        let open_tag = &html[..open_end];
        let attrs = open_tag.to_string();
        let explicit_size = DATA_WIDTH_RE.is_match(open_tag);
        let close = format!("</{tag}>");
        if rest_is_empty_element(html, open_end, &close) {
            return Some((
                open_end + close.len(),
                Child::Qr {
                    attrs,
                    explicit_size,
                },
            ));
        }
    }

    if html.starts_with("<img") {
        let end = html.find('>')? + 1;
        return Some((end, Child::Img));
    }

    html.find('>').map(|end| (end + 1, Child::Other))
}

fn rest_is_empty_element(html: &str, open_end: usize, close: &str) -> bool {
    html[open_end..].starts_with(close)
}

fn find_open_tag_end(html: &str) -> Option<(usize, usize)> {
    let gt = html.find('>')?;
    Some((gt + 1, gt + 1))
}

fn balanced_element_inner(html: &str, tag: &str) -> Option<String> {
    balanced_element_inner_ref(html, tag).map(|s| s.to_string())
}

fn balanced_element_inner_ref<'a>(html: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 1i32;
    let mut i = 0;
    while i < html.len() {
        if html[i..].starts_with(&close) {
            depth -= 1;
            if depth == 0 {
                return Some(&html[..i]);
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

fn barcode_height_mode_from_attrs(attrs: &str) -> BarcodeHeightMode {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"data-barcode-height\s*=\s*"([^"]+)""#).expect("barcode height attr regex")
    });
    RE.captures(attrs)
        .map(|c| BarcodeHeightMode::parse(&c[1]))
        .unwrap_or_default()
}

fn barcode_is_2d_from_attrs(attrs: &str) -> bool {
    if attrs.contains(r#"data-barcode-2d="1""#) {
        return true;
    }
    static SYM_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"data-symbology\s*=\s*"([^"]+)""#).expect("symbology attr regex")
    });
    SYM_RE
        .captures(attrs)
        .map(|c| crate::symbology::resolve_symbology(&c[1]).is_2d)
        .unwrap_or(false)
}

fn patch_nth_code(
    body: &str,
    class_name: &str,
    nth: usize,
    w: f64,
    bar_h: f64,
    caption_font_px: Option<f64>,
) -> String {
    let needle = format!("class=\"{class_name}\"");
    let mut from = 0;
    let mut found = 0;
    let wr = w.round().max(1.0);
    let hr = bar_h.round().max(1.0);
    while let Some(rel) = body[from..].find(&needle) {
        let at = from + rel;
        if found == nth {
            let tag_start = body[..at]
                .rfind("<div")
                .or_else(|| body[..at].rfind("<span"))
                .unwrap_or(at);
            let gt = match body[at..].find('>') {
                Some(g) => at + g,
                None => return body.to_string(),
            };
            let open = &body[tag_start..=gt];
            if open.contains("data-fit-width") {
                return body.to_string();
            }
            let mut insert = format!(" data-fit-width=\"{wr:.0}\" data-fit-height=\"{hr:.0}\"");
            if let Some(cf) = caption_font_px {
                insert.push_str(&format!(
                    " data-fit-font-size=\"{:.0}\"",
                    cf.round().max(8.0)
                ));
            }
            let mut out = String::with_capacity(body.len() + insert.len());
            out.push_str(&body[..gt]);
            out.push_str(&insert);
            out.push_str(&body[gt..]);
            return out;
        }
        found += 1;
        from = at + needle.len();
    }
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transpile::{CascadingInsetMm, LabelFit, LabelStyle, MediaInsetPx, ViewportPx};

    fn fill_opts() -> TranspileOptions {
        TranspileOptions {
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
                qr_size_px: 40.0,
                element_gap_px: 8.0,
                ..Default::default()
            },
            media_inset: MediaInsetPx::default(),
            ..Default::default()
        }
    }

    #[test]
    fn content_head_fit_sizes_lone_text_on_landscape_continuous() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Content,
            viewport: Some(ViewportPx {
                width: None,
                height: Some(170.0),
            }),
            style: LabelStyle::from_mm(
                2.0,
                15.0,
                12.0,
                0.33,
                CascadingInsetMm::uniform(2.0),
                2.0,
                0.0,
                2.0,
                180.0,
                2,
            ),
            ..Default::default()
        };
        let body =
            r#"<div class="lbl-label"><div class="lbl-text">01234567890123456789</div></div>"#;
        let fit = apply_content_head_text_fit(body, &opts);
        assert!(
            fit.css.contains("font-size:"),
            "css={} font={:?}",
            fit.css,
            fit.font_px
        );
        assert!(
            fit.css.contains("padding-inline:"),
            "continuous text must claim ink side bearings: {}",
            fit.css
        );
        let font = fit.font_px.unwrap_or(0.0);
        assert!(font > 60.0, "font={font}");
    }

    #[test]
    fn content_head_fit_sizes_lone_image_on_landscape_continuous() {
        let opts = TranspileOptions {
            label_fit: LabelFit::Content,
            viewport: Some(ViewportPx {
                width: None,
                height: Some(170.0),
            }),
            style: LabelStyle::from_mm(
                2.0,
                15.0,
                12.0,
                0.33,
                CascadingInsetMm::uniform(2.0),
                2.0,
                0.0,
                2.0,
                180.0,
                2,
            ),
            ..Default::default()
        };
        let body = r#"<div class="lbl-label"><img src="data:image/png;base64,xx" /></div>"#;
        let fit = apply_content_head_text_fit(body, &opts);
        assert!(
            fit.css.contains("img:only-child") && fit.css.contains("--lbl-feed-px:"),
            "css={}",
            fit.css
        );
        assert!(fit.css.contains("object-fit:contain"), "css={}", fit.css);
    }

    fn continuous_content_opts() -> TranspileOptions {
        TranspileOptions {
            label_fit: LabelFit::Content,
            viewport: Some(ViewportPx {
                width: None,
                height: Some(170.0),
            }),
            style: LabelStyle::from_mm(
                2.0,
                15.0,
                12.0,
                0.33,
                CascadingInsetMm::uniform(2.0),
                2.0,
                0.0,
                2.0,
                180.0,
                2,
            ),
            ..Default::default()
        }
    }

    fn feed_px_from_css(css: &str) -> f64 {
        css.split("--lbl-feed-px:")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0)
    }

    fn text_cell_width_from_css(css: &str) -> f64 {
        css.split(".lbl-row>.lbl-text:nth-child(1){width:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0)
    }

    #[test]
    fn content_head_fit_sizes_text_image_row_on_landscape_continuous() {
        let opts = continuous_content_opts();
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">a</span></span><img src="https://api.iconify.design/lucide/a-arrow-up.svg?color=black" /></div></div>"#;
        let fit = apply_content_head_text_fit(body, &opts);
        let feed = feed_px_from_css(&fit.css);
        let text_w = text_cell_width_from_css(&fit.css);
        assert!(
            fit.css.contains(".lbl-row>img:nth-child(2){width:"),
            "css={}",
            fit.css
        );
        assert!(
            feed > 10.0 && feed < 2_000.0,
            "feed must be content-sized, not unbounded sentinel: feed={feed} css={}",
            fit.css
        );
        assert!(
            text_w > 1.0 && text_w < 2_000.0,
            "text cell must not claim ~1e6px feed: text_w={text_w} css={}",
            fit.css
        );
        let font = fit.font_px.unwrap_or(0.0);
        assert!(font > 20.0 && font < 400.0, "font={font} css={}", fit.css);
    }

    #[test]
    fn content_head_fit_sizes_text_qr_row_on_landscape_continuous() {
        let opts = continuous_content_opts();
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">a</span></span><div class="lbl-qr" data-qr="hi"></div></div></div>"#;
        let fit = apply_content_head_text_fit(body, &opts);
        let feed = feed_px_from_css(&fit.css);
        let text_w = text_cell_width_from_css(&fit.css);
        assert!(feed > 10.0 && feed < 2_000.0, "feed={feed} css={}", fit.css);
        assert!(
            text_w > 1.0 && text_w < 2_000.0,
            "text_w={text_w} css={}",
            fit.css
        );
        assert!(
            fit.css.contains(".lbl-row>.lbl-qr:nth-child(2){width:"),
            "css={}",
            fit.css
        );
    }

    #[test]
    fn content_head_fit_sizes_lone_barcode_on_landscape_continuous() {
        let opts = continuous_content_opts();
        let body = r#"<div class="lbl-label"><div class="lbl-barcode" data-symbology="CODE128" data-value="123"></div></div>"#;
        let fit = apply_content_head_text_fit(body, &opts);
        let feed = feed_px_from_css(&fit.css);
        assert!(
            feed > 10.0 && feed < 2_000.0,
            "lone barcode feed must not use unbounded axis: feed={feed} css={}",
            fit.css
        );
        assert!(
            !fit.body.contains("data-fit-width=\"1000000\""),
            "body={}",
            fit.body
        );
    }

    #[test]
    fn lone_barcode_gets_fit_attrs() {
        let body = r#"<div class="lbl-label"><div class="lbl-barcode" data-symbology="CODE128" data-value="123"></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        assert!(fit.body.contains("data-fit-width="), "{}", fit.body);
        assert!(fit.body.contains("data-fit-height="), "{}", fit.body);
    }

    #[test]
    fn text_barcode_text_row_uses_flex_sizing() {
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">aa </span></span><div class="lbl-barcode" data-symbology="CODE128" data-value="12346"></div><span class="lbl-text"><span class="lbl-text-inlines"> bc</span></span></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        assert!(
            fit.css.contains(".lbl-row>.lbl-text{font-size:"),
            "{}",
            fit.css
        );
        assert!(fit.body.contains("data-fit-width="), "{}", fit.body);
        assert!(
            fit.css
                .contains(".lbl-row>.lbl-barcode:nth-child(2){width:"),
            "{}",
            fit.css
        );
        let font: f64 = fit
            .css
            .split(".lbl-row>.lbl-text{font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let bar_h: f64 = fit
            .body
            .split("data-fit-height=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        assert!(font > 20.0, "expected row font, got {font}");
        assert!(
            bar_h > font * 0.65,
            "normal row barcode should track text size, font={font} bar_h={bar_h}"
        );
        assert!(fit.body.contains("data-fit-font-size="), "{}", fit.body);
        let row_h = font * ROW_TEXT_LINE_HEIGHT * ROW_TEXT_INK_FACTOR;
        let bar_h = fit
            .body
            .split("data-fit-height=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let caption: f64 = fit
            .body
            .split("data-fit-font-size=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let svg_h = jsbarcode_svg_height(bar_h, caption);
        let budget = 142.0 * ROW_FIT_SAFETY;
        assert!(
            row_h <= budget + 1.0 && svg_h <= budget + 1.0,
            "row should fit label, text_h={row_h} svg_h={svg_h}"
        );
    }

    #[test]
    fn row_oo_barcode_oo_fits_label_box() {
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">OO </span></span><div class="lbl-barcode" data-symbology="CODE128" data-value="12346"></div><span class="lbl-text"><span class="lbl-text-inlines"> OO</span></span></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        let font: f64 = fit
            .css
            .split(".lbl-row>.lbl-text{font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(font > 1.0, "{}", fit.css);
        let bar_h: f64 = fit
            .body
            .split("data-fit-height=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let caption: f64 = fit
            .body
            .split("data-fit-font-size=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let budget_h = 142.0 * ROW_FIT_SAFETY;
        let budget_w = 354.0 * ROW_FIT_SAFETY;
        let text_h = font * ROW_TEXT_LINE_HEIGHT * ROW_TEXT_INK_FACTOR;
        let svg_h = jsbarcode_svg_height(bar_h, caption);
        assert!(text_h <= budget_h + 1.0, "text too tall: {text_h}");
        assert!(svg_h <= budget_h + 1.0, "barcode too tall: {svg_h}");
        assert!(
            text_line_width_px("OO ", font) <= budget_w / 3.5 + 1.0,
            "OO too wide for cell at font {font}"
        );
    }

    #[test]
    fn row_bp_barcode_bp_fits_label_height() {
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">bp </span></span><div class="lbl-barcode" data-symbology="CODE128" data-value="12346"></div><span class="lbl-text"><span class="lbl-text-inlines"> bp</span></span></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        let font: f64 = fit
            .css
            .split(".lbl-row>.lbl-text{font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let bar_h: f64 = fit
            .body
            .split("data-fit-height=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let caption: f64 = fit
            .body
            .split("data-fit-font-size=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        let budget = 142.0 * ROW_FIT_SAFETY;
        assert!(
            font * ROW_TEXT_LINE_HEIGHT * ROW_TEXT_INK_FACTOR <= budget + 1.0,
            "text too tall"
        );
        assert!(
            jsbarcode_svg_height(bar_h, caption) <= budget + 1.0,
            "barcode too tall"
        );
    }

    #[test]
    fn row_text_slots_have_width_and_center() {
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">OO</span></span><div class="lbl-barcode" data-symbology="CODE128" data-value="O360"></div><span class="lbl-text"><span class="lbl-text-inlines">O</span></span></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        assert!(
            fit.css.contains(".lbl-row>.lbl-text:nth-child(1){width:"),
            "{}",
            fit.css
        );
        assert!(
            fit.css.contains(".lbl-row>.lbl-text:nth-child(3){width:"),
            "{}",
            fit.css
        );
        assert!(fit.css.contains("text-align:center"), "{}", fit.css);
        let t1: f64 = fit
            .css
            .split(".lbl-row>.lbl-text:nth-child(1){width:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let t3: f64 = fit
            .css
            .split(".lbl-row>.lbl-text:nth-child(3){width:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let bc: f64 = fit
            .css
            .split(".lbl-row>.lbl-barcode:nth-child(2){width:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!((t1 - t3).abs() < 1.0, "text slots equal: {t1} vs {t3}");
        assert!(bc > t1, "barcode slot wider than text: bc={bc} text={t1}");
    }

    #[test]
    fn column_row_and_text_lines_share_equal_font() {
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">OO</span></span><div class="lbl-barcode" data-symbology="CODE128" data-value="O360"></div></div><div class="lbl-text">OO</div><div class="lbl-text">OO</div><div class="lbl-text">OO</div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        assert!(
            fit.css.contains("lbl-label>:not(:only-child)"),
            "{}",
            fit.css
        );
        let row_font: f64 = fit
            .css
            .split(".lbl-row>.lbl-text{font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let line_font: f64 = fit
            .css
            .split(".lbl-label>.lbl-text:nth-child(2){font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(row_font > 0.0 && line_font > 0.0, "{}", fit.css);
        assert!(
            (row_font - line_font).abs() < row_font * 0.15,
            "row text and stacked lines should match: row={row_font} line={line_font}"
        );
    }

    #[test]
    fn stretch_barcode_uses_label_height() {
        let body = r#"<div class="lbl-label"><div class="lbl-barcode" data-symbology="CODE128" data-value="12346" data-barcode-height="stretch"></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        let bar_h: f64 = fit
            .body
            .split("data-fit-height=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        assert!(
            bar_h > 85.0,
            "stretch barcode bars should grow, got {bar_h} body={}",
            fit.body
        );
    }

    #[test]
    fn text_qr_row_grows_qr() {
        let body = r#"<div class="lbl-label"><div class="lbl-row lbl-center"><div class="lbl-text">Hello</div><div class="lbl-qr" data-qr="x"></div></div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        assert!(fit.body.contains("data-fit-width="), "{}", fit.body);
        assert!(fit.css.contains("font-size:"), "{}", fit.css);
    }

    #[test]
    fn column_text_row_text_grows_each_line() {
        let body = r#"<div class="lbl-label"><div class="lbl-text">a</div><div class="lbl-row lbl-center"><span class="lbl-text"><span class="lbl-text-inlines">b </span></span><div class="lbl-qr" data-qr="https://example.com"></div></div><div class="lbl-text">c</div></div>"#;
        let fit = apply_layout_fit(body, &fill_opts());
        let row_font: f64 = fit
            .css
            .split(".lbl-row>.lbl-text{font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let line1_font: f64 = fit
            .css
            .split(".lbl-label>.lbl-text:nth-child(1){font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        let line3_font: f64 = fit
            .css
            .split(".lbl-label>.lbl-text:nth-child(3){font-size:")
            .nth(1)
            .and_then(|s| s.split("px").next())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0.0);
        assert!(
            row_font > 20.0,
            "row font too small: {row_font}, css={}",
            fit.css
        );
        assert!(line1_font > 20.0, "line1 font too small: {line1_font}");
        assert!(line3_font > 20.0, "line3 font too small: {line3_font}");
        assert!(
            (row_font - line1_font).abs() < row_font * 0.05,
            "stacked lines and row text should share font: row={row_font} line1={line1_font}"
        );
    }
}
