//! Parsing of text + inline directives into an authoring document.

use crate::qr::{parse_qr_attrs, QrOptions};
use crate::DEFAULT_SYMBOLOGY;

/// How a barcode grows in fill mode: configured bar height vs stretch to the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BarcodeHeightMode {
    /// Use configured bar height (similar to surrounding text).
    #[default]
    Normal,
    /// Stretch bars to use available label height (caption still rendered below).
    Stretch,
}

impl BarcodeHeightMode {
    /// Parse `normal` / `stretch` (case-insensitive); unknown values map to [`Normal`].
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("stretch") {
            Self::Stretch
        } else {
            Self::Normal
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Stretch => "stretch",
        }
    }
}

fn is_barcode_height_mode(s: &str) -> bool {
    s.eq_ignore_ascii_case("stretch") || s.eq_ignore_ascii_case("normal")
}

/// A content block in a label.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Literal text (may contain newlines).
    Text(String),
    /// A run of text rendered at a font size relative to the base (`scale` is
    /// an `em` multiplier, e.g. `1.5` = 150%).
    Sized {
        /// Font-size multiplier relative to the base text size.
        scale: f64,
        /// Inline content rendered at this size (may include nested directives).
        content: Vec<Block>,
    },
    /// A run of text rendered in a named font family (see [`crate::fonts`]).
    Font {
        /// Font slug, e.g. `roboto`, `mono`.
        family: String,
        /// Inline content in this font (may include nested directives).
        content: Vec<Block>,
    },
    /// A run of text rendered in a foreground color (hex, e.g. `#ff0000`).
    Color {
        /// CSS color, normalized to `#rrggbb`.
        color: String,
        /// Inline content in this color (may include nested directives).
        content: Vec<Block>,
    },
    /// A QR code carrying the given payload and optional overrides.
    Qr {
        /// Encoded QR payload.
        payload: String,
        /// Per-code options (error correction, quiet zone, colors).
        options: QrOptions,
    },
    /// A barcode of the given symbology carrying the given data.
    Barcode {
        /// Symbology, e.g. `CODE128`, `EAN13`.
        symbology: String,
        /// The encoded data.
        data: String,
        /// Fill-mode bar height behaviour (`{{barcode:…:stretch}}` or `<barcode height="stretch">`).
        height_mode: BarcodeHeightMode,
    },
    /// An image referenced by a local path or remote URL.
    Image(String),
}

impl Block {
    /// Render this single block to its authoring HTML.
    ///
    /// Text blocks are wrapped in `<div class="lbl-text">` (with newlines
    /// turned into `<br>`); directive blocks render their custom element
    /// (`<qr>`, `<barcode>`, `<img>`). This is the per-block building block of
    /// [`Document::to_authoring_html`], and is reused by other front-ends (such
    /// as `lbl-markdown`) that need to emit the same directive elements.
    pub fn to_authoring_html(&self) -> String {
        match self {
            Block::Text(t) => {
                if t.contains('\n') {
                    t.split('\n')
                        .filter(|line| !line.is_empty())
                        .map(|line| {
                            format!(r#"<div class="lbl-text">{}</div>"#, text_to_html(line))
                        })
                        .collect()
                } else {
                    wrap_lbl_text_inlines(&text_to_html(t))
                }
            }
            Block::Sized { scale, content } => wrap_lbl_text_inlines(&format!(
                "<span style=\"font-size:{}em\">{}</span>",
                fmt_scale(*scale),
                inline_blocks_html(content)
            )),
            Block::Font { family, content } => wrap_lbl_text_inlines(&format!(
                "<span data-lbl-font=\"{}\">{}</span>",
                escape(family),
                inline_blocks_html(content)
            )),
            Block::Color { color, content } => wrap_lbl_text_inlines(&format!(
                "<span style=\"color:{}\">{}</span>",
                escape(color),
                inline_blocks_html(content)
            )),
            Block::Qr { payload, options } => {
                format!("<qr{}>{}</qr>", options.to_attrs(), escape(payload))
            }
            Block::Barcode {
                symbology,
                data,
                height_mode,
            } => {
                let height_attr = if *height_mode == BarcodeHeightMode::Stretch {
                    r#" height="stretch""#
                } else {
                    ""
                };
                format!(
                    "<barcode type=\"{}\"{height_attr}>{}</barcode>",
                    escape(symbology),
                    escape(data)
                )
            }
            Block::Image(uri) => format!("<img src=\"{}\" />", escape(uri)),
        }
    }

    /// Inline HTML for a text-run segment (no outer `.lbl-text` wrapper).
    fn to_inline_html(&self) -> String {
        match self {
            Block::Text(t) => text_to_html(t),
            Block::Sized { scale, content } => format!(
                "<span style=\"font-size:{}em\">{}</span>",
                fmt_scale(*scale),
                inline_blocks_html(content)
            ),
            Block::Font { family, content } => format!(
                "<span data-lbl-font=\"{}\">{}</span>",
                escape(family),
                inline_blocks_html(content)
            ),
            Block::Color { color, content } => format!(
                "<span style=\"color:{}\">{}</span>",
                escape(color),
                inline_blocks_html(content)
            ),
            _ => self.to_authoring_html(),
        }
    }
}

/// A parsed label document: an ordered list of [`Block`]s.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    /// The content blocks, in order.
    pub blocks: Vec<Block>,
}

impl Document {
    /// Parse `input` into a document. When `raw` is true, inline mini-syntax is
    /// not interpreted and the whole input becomes a single text block.
    pub fn parse(input: &str, raw: bool) -> Self {
        if raw {
            let mut doc = Document::default();
            if !input.is_empty() {
                doc.blocks.push(Block::Text(input.to_string()));
            }
            return doc;
        }
        Document {
            blocks: parse_inline(input),
        }
    }

    /// Append a QR directive (from a flag).
    pub fn push_qr(&mut self, payload: impl Into<String>) {
        self.blocks.push(Block::Qr {
            payload: payload.into(),
            options: QrOptions::default(),
        });
    }

    /// Append a barcode directive (from a flag). `spec` may be
    /// `SYMBOLOGY:data` or just `data` (defaulting the symbology).
    pub fn push_barcode(&mut self, spec: &str) {
        self.blocks.push(barcode_from_spec(spec));
    }

    /// Append an image directive (from a flag).
    pub fn push_image(&mut self, uri: impl Into<String>) {
        self.blocks.push(Block::Image(uri.into()));
    }

    /// Render the authoring HTML fragment (the `<div class="lbl-label">` root
    /// and its children) consumed by `lbl-transpile-html`.
    pub fn to_authoring_html(&self) -> String {
        let pieces = expand_layout_pieces(&self.blocks);
        let mut out = String::from("<div class=\"lbl-label\">");
        let mut i = 0;
        while i < pieces.len() {
            if let LayoutPiece::Line(line) = &pieces[i] {
                if line_followed_by_inline_widget(&pieces, i) {
                    out.push_str("<div class=\"lbl-row lbl-center\">");
                    out.push_str(&wrap_lbl_text_inlines(&text_to_html(line)));
                    i += 1;
                    while i < pieces.len() {
                        let LayoutPiece::Inline(block) = &pieces[i] else {
                            break;
                        };
                        if !is_inline_flow_block(block) {
                            break;
                        }
                        out.push_str(&block.to_authoring_html());
                        i += 1;
                    }
                    out.push_str("</div>");
                    continue;
                }
                out.push_str(&line_piece_html(line));
                i += 1;
                continue;
            }

            let LayoutPiece::Inline(block) = &pieces[i] else {
                i += 1;
                continue;
            };
            if is_inline_flow_block(block) {
                let start = i;
                while i < pieces.len() {
                    let LayoutPiece::Inline(b) = &pieces[i] else {
                        break;
                    };
                    if !is_inline_flow_block(b) {
                        break;
                    }
                    i += 1;
                }
                let group = &pieces[start..i];
                if group.len() == 1 {
                    let LayoutPiece::Inline(block) = &group[0] else {
                        continue;
                    };
                    out.push_str(&block.to_authoring_html());
                } else if is_text_run_piece_group(group) {
                    out.push_str(&text_run_piece_group_html(group));
                } else {
                    out.push_str("<div class=\"lbl-row lbl-center\">");
                    for piece in group {
                        let LayoutPiece::Inline(block) = piece else {
                            continue;
                        };
                        out.push_str(&block.to_authoring_html());
                    }
                    out.push_str("</div>");
                }
            } else {
                out.push_str(&block.to_authoring_html());
                i += 1;
            }
        }
        out.push_str("</div>");
        out
    }

    /// Render a full standalone authoring HTML document.
    pub fn to_authoring_document(&self) -> String {
        format!(
            "<!doctype html>\n<html>\n<head><meta charset=\"utf-8\"></head>\n<body>\n{}\n</body>\n</html>\n",
            self.to_authoring_html()
        )
    }
}

/// Parse a barcode `spec` (`SYMBOLOGY:data`, optional `:stretch` / `:normal` suffix)
/// into a [`Block::Barcode`], defaulting the symbology when none is given.
pub fn barcode_from_spec(spec: &str) -> Block {
    let mut parts: Vec<&str> = spec.split(':').collect();
    let mut height_mode = BarcodeHeightMode::Normal;
    if parts.len() > 1 {
        if let Some(last) = parts.last() {
            if is_barcode_height_mode(last) {
                height_mode = BarcodeHeightMode::parse(last);
                parts.pop();
            }
        }
    }
    let rest = parts.join(":");
    match rest.split_once(':') {
        Some((sym, data)) if !sym.is_empty() && !data.is_empty() => Block::Barcode {
            symbology: sym.to_string(),
            data: data.to_string(),
            height_mode,
        },
        _ if !rest.is_empty() => Block::Barcode {
            symbology: DEFAULT_SYMBOLOGY.to_string(),
            data: rest,
            height_mode,
        },
        _ => Block::Barcode {
            symbology: DEFAULT_SYMBOLOGY.to_string(),
            data: spec.to_string(),
            height_mode,
        },
    }
}

/// Try to parse a directive at `start` (the first `{` of `{{`).
///
/// Returns the parsed block and the index immediately after the consumed
/// directive. Used by `lbl-text` and `lbl-markdown` so both front-ends share
/// identical directive syntax.
pub fn scan_directive_at(input: &str, start: usize) -> Option<(Block, usize)> {
    if !input[start..].starts_with("{{") {
        return None;
    }

    if let Some(result) = try_parse_qr_block(input, start) {
        return Some(result);
    }

    let close = find_directive_close(input, start)?;
    let inner = &input[start + 2..close];
    let block = directive_from_inner(inner)?;
    Some((block, close + 2))
}

/// Index of the first `}` in the closing `}}` of the directive opened at
/// `open_start` (which must point at the first `{` of `{{`). Returns `None`
/// when braces are unbalanced.
fn find_directive_close(input: &str, open_start: usize) -> Option<usize> {
    if !input[open_start..].starts_with("{{") {
        return None;
    }
    let mut i = open_start + 2;
    let mut depth = 1;
    while i + 1 < input.len() {
        if input[i..].starts_with("{{") {
            depth += 1;
            i += 2;
            continue;
        }
        if input[i..].starts_with("}}") {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += 2;
            continue;
        }
        let ch = input[i..].chars().next().unwrap();
        i += ch.len_utf8();
    }
    None
}

/// Scan `input` for `{{type:...}}` directives, returning interleaved text and
/// directive blocks. Unrecognized `{{...}}` is kept as literal text.
fn parse_inline(input: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text_buf = String::new();
    let mut i = 0;

    while i < input.len() {
        if input[i..].starts_with("{{") {
            if let Some((block, end)) = scan_directive_at(input, i) {
                flush_text(&mut text_buf, &mut blocks);
                blocks.push(block);
                i = end;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        text_buf.push(ch);
        i += ch.len_utf8();
    }

    flush_text(&mut text_buf, &mut blocks);
    blocks
}

/// HTML-like block QR: `{{qr ec=low}}payload{{/qr}}`.
fn try_parse_qr_block(input: &str, start: usize) -> Option<(Block, usize)> {
    const OPEN: &str = "{{qr";
    if !input[start..].starts_with(OPEN) {
        return None;
    }
    let mut pos = start + OPEN.len();
    // `{{qr:payload}}` is the colon shorthand, not a block opener.
    if input.as_bytes().get(pos) == Some(&b':') {
        return None;
    }
    let close_rel = input[pos..].find("}}")?;
    let attrs = input[pos..pos + close_rel].trim();
    pos += close_rel + 2;

    const END: &str = "{{/qr}}";
    let payload_rel = input[pos..].find(END)?;
    let payload = &input[pos..pos + payload_rel];
    pos += payload_rel + END.len();

    if payload.is_empty() {
        return None;
    }

    Some((
        Block::Qr {
            payload: payload.to_string(),
            options: parse_qr_attrs(attrs),
        },
        pos,
    ))
}

/// Push accumulated text as a block. Whitespace between directives is preserved
/// so `aa {{color:#f00:bb}} cc` keeps its word spacing in the output.
fn flush_text(buf: &mut String, blocks: &mut Vec<Block>) {
    if !buf.is_empty() {
        blocks.push(Block::Text(buf.clone()));
    }
    buf.clear();
}

/// Layout segment after splitting multiline text at newlines.
enum LayoutPiece {
    /// One source line (block-level when not paired with a widget).
    Line(String),
    /// Parsed block without embedded newlines.
    Inline(Block),
}

fn expand_layout_pieces(blocks: &[Block]) -> Vec<LayoutPiece> {
    let mut out = Vec::new();
    for block in blocks {
        match block {
            Block::Text(t) if t.contains('\n') => {
                for line in t.split('\n').filter(|line| !line.is_empty()) {
                    out.push(LayoutPiece::Line(line.to_string()));
                }
            }
            other => out.push(LayoutPiece::Inline(other.clone())),
        }
    }
    out
}

fn line_followed_by_inline_widget(pieces: &[LayoutPiece], line_index: usize) -> bool {
    matches!(
        pieces.get(line_index + 1),
        Some(LayoutPiece::Inline(
            Block::Qr { .. } | Block::Barcode { .. } | Block::Image(_)
        ))
    )
}

fn line_piece_html(line: &str) -> String {
    format!(r#"<div class="lbl-text">{}</div>"#, text_to_html(line))
}

fn is_inline_flow_block(block: &Block) -> bool {
    match block {
        Block::Text(t) => !t.contains('\n'),
        Block::Sized { .. }
        | Block::Font { .. }
        | Block::Color { .. }
        | Block::Qr { .. }
        | Block::Barcode { .. }
        | Block::Image(_) => true,
    }
}

fn is_text_run_piece_group(pieces: &[LayoutPiece]) -> bool {
    pieces.iter().all(|piece| {
        matches!(
            piece,
            LayoutPiece::Inline(
                Block::Text(_) | Block::Sized { .. } | Block::Font { .. } | Block::Color { .. }
            )
        )
    })
}

fn text_run_piece_group_html(pieces: &[LayoutPiece]) -> String {
    let mut inner = String::new();
    for piece in pieces {
        if let LayoutPiece::Inline(block) = piece {
            inner.push_str(&block.to_inline_html());
        }
    }
    wrap_lbl_text_inlines(&inner)
}

fn wrap_lbl_text_inlines(inner: &str) -> String {
    format!("<span class=\"lbl-text\"><span class=\"lbl-text-inlines\">{inner}</span></span>")
}

fn inline_blocks_html(blocks: &[Block]) -> String {
    blocks.iter().map(|b| b.to_inline_html()).collect()
}

fn inline_content_from_spec(spec: &str) -> Option<Vec<Block>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }
    let blocks = parse_inline(spec);
    if blocks.is_empty() {
        return None;
    }
    Some(blocks)
}

/// Parse the inside of an inline `{{...}}` directive (e.g. `qr:https://x.y`,
/// `barcode:EAN13:123`, `image:./a.png`) into a [`Block`].
///
/// Returns `None` for unrecognized or empty directives, so callers can leave
/// the original text untouched. This is the same matcher used by the inline
/// scanner, exposed so other front-ends (such as `lbl-markdown`) apply
/// identical directive syntax.
pub fn parse_directive(inner: &str) -> Option<Block> {
    directive_from_inner(inner)
}

fn directive_from_inner(inner: &str) -> Option<Block> {
    let (kind, rest) = inner.split_once(':')?;
    let kind = kind.trim().to_ascii_lowercase();
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    match kind.as_str() {
        "barcode" => Some(barcode_from_spec(rest)),
        "color" | "fg" | "foreground" | "tc" | "text-color" => color_from_spec(rest),
        "ff" | "font" | "font-family" => font_from_spec(rest),
        "font-size" | "fs" | "size" => sized_from_spec(rest),
        "image" | "img" => Some(Block::Image(rest.to_string())),
        "qr" => {
            let payload = rest.trim();
            if payload.is_empty() {
                return None;
            }
            Some(Block::Qr {
                payload: payload.to_string(),
                options: QrOptions::default(),
            })
        }
        _ => None,
    }
}

/// Parse a sizing spec `SCALE:TEXT` (e.g. `1.5:World`) into a [`Block::Sized`].
/// `SCALE` accepts a bare multiplier (`1.5`), an explicit `x` suffix (`1.5x`),
/// or a percentage (`150%`). Returns `None` if the scale is invalid or the text
/// is empty, so the original `{{...}}` is left literal.
fn sized_from_spec(spec: &str) -> Option<Block> {
    let (scale_str, rest) = spec.split_once(':')?;
    let scale = parse_scale(scale_str.trim())?;
    let content = inline_content_from_spec(rest)?;
    Some(Block::Sized { scale, content })
}

/// Parse a color spec `HEX:TEXT` (e.g. `#ff0000:Hello`) into a [`Block::Color`].
fn color_from_spec(spec: &str) -> Option<Block> {
    let (color_str, rest) = spec.split_once(':')?;
    let color = parse_hex_color(color_str.trim())?;
    let content = inline_content_from_spec(rest)?;
    Some(Block::Color { color, content })
}

/// Normalize a CSS hex color to lowercase `#rrggbb`. Accepts `#rgb` and `#rrggbb`.
fn parse_hex_color(s: &str) -> Option<String> {
    let s = s.trim();
    let hex = s.strip_prefix('#')?;
    let expanded = match hex.len() {
        3 => {
            let mut out = String::with_capacity(6);
            for ch in hex.chars() {
                if !ch.is_ascii_hexdigit() {
                    return None;
                }
                out.push(ch);
                out.push(ch);
            }
            out
        }
        6 => {
            if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            hex.to_string()
        }
        _ => return None,
    };
    Some(format!("#{}", expanded.to_ascii_lowercase()))
}

/// Parse a font spec `SLUG:TEXT` (e.g. `roboto:Hello`) into a [`Block::Font`].
fn font_from_spec(spec: &str) -> Option<Block> {
    let (family, rest) = spec.split_once(':')?;
    let family = family.trim();
    if family.is_empty() || crate::fonts::resolve_slug(family).is_none() {
        return None;
    }
    let content = inline_content_from_spec(rest)?;
    Some(Block::Font {
        family: family.to_ascii_lowercase(),
        content,
    })
}

/// Parse a font-size multiplier: `1.5`, `1.5x`, or `150%`. Must be positive.
fn parse_scale(s: &str) -> Option<f64> {
    let s = s.trim();
    if let Some(pct) = s.strip_suffix('%') {
        return pct
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v / 100.0)
            .filter(|v| *v > 0.0);
    }
    let s = s.strip_suffix(['x', 'X']).unwrap_or(s);
    s.parse::<f64>().ok().filter(|v| *v > 0.0)
}

/// Format a scale for CSS. `f64`'s `Display` already trims trailing zeros
/// (e.g. `2.0` -> `2`, `1.5` -> `1.5`).
fn fmt_scale(scale: f64) -> String {
    format!("{scale}")
}

fn text_to_html(text: &str) -> String {
    let escaped = escape(text);
    escaped.replace('\n', "<br>")
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qr::{QrErrorCorrection, QrOptions};

    #[test]
    fn plain_text_becomes_single_block() {
        let doc = Document::parse("hello, world!", false);
        assert_eq!(doc.blocks, vec![Block::Text("hello, world!".to_string())]);
    }

    #[test]
    fn inline_qr_is_parsed() {
        let doc = Document::parse("ship to {{qr:https://x.y}}", false);
        assert_eq!(
            doc.blocks,
            vec![
                Block::Text("ship to ".to_string()),
                Block::Qr {
                    payload: "https://x.y".to_string(),
                    options: QrOptions::default(),
                },
            ]
        );
    }

    #[test]
    fn inline_qr_with_block_form() {
        let doc = Document::parse("{{qr ec=low margin=2}}hi{{/qr}}", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Qr {
                payload: "hi".to_string(),
                options: QrOptions {
                    error_correction: Some(QrErrorCorrection::L),
                    margin: Some(2),
                    ..QrOptions::default()
                },
            }]
        );
        let html = doc.to_authoring_html();
        assert!(html.contains(r#"<qr ec="L" margin="2">hi</qr>"#), "{html}");
    }

    #[test]
    fn colon_form_treats_entire_spec_as_payload() {
        let doc = Document::parse("{{qr:hi ec=low}}", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Qr {
                payload: "hi ec=low".to_string(),
                options: QrOptions::default(),
            }]
        );
    }

    #[test]
    fn block_form_allows_option_like_payload() {
        let doc = Document::parse("{{qr ec=low}}hi ec=low{{/qr}}", false);
        let html = doc.to_authoring_html();
        assert!(html.contains(r#"<qr ec="L">hi ec=low</qr>"#), "{html}");
    }

    #[test]
    fn inline_barcode_with_and_without_symbology() {
        let doc = Document::parse("{{barcode:EAN13:123}} {{barcode:456}}", false);
        assert_eq!(
            doc.blocks,
            vec![
                Block::Barcode {
                    symbology: "EAN13".into(),
                    data: "123".into(),
                    height_mode: BarcodeHeightMode::Normal,
                },
                Block::Text(" ".to_string()),
                Block::Barcode {
                    symbology: "CODE128".into(),
                    data: "456".into(),
                    height_mode: BarcodeHeightMode::Normal,
                },
            ]
        );
    }

    #[test]
    fn inline_barcode_height_mode_suffix() {
        let doc = Document::parse("{{barcode:12346:stretch}}", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Barcode {
                symbology: "CODE128".into(),
                data: "12346".into(),
                height_mode: BarcodeHeightMode::Stretch,
            }]
        );
        let html = doc.to_authoring_html();
        assert!(html.contains(r#"height="stretch""#), "{html}");
    }

    #[test]
    fn raw_mode_keeps_braces_literal() {
        let doc = Document::parse("price {{qr:x}} tag", true);
        assert_eq!(
            doc.blocks,
            vec![Block::Text("price {{qr:x}} tag".to_string())]
        );
        assert!(doc.to_authoring_html().contains("{{qr:x}}"));
    }

    #[test]
    fn unrecognized_directive_kept_literal() {
        let doc = Document::parse("a {{unknown:y}} b", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Text("a {{unknown:y}} b".to_string())]
        );
    }

    #[test]
    fn inline_color_directive_is_parsed() {
        let doc = Document::parse("Hello, {{color:#ff0000:World}}", false);
        assert_eq!(
            doc.blocks,
            vec![
                Block::Text("Hello, ".to_string()),
                Block::Color {
                    color: "#ff0000".to_string(),
                    content: vec![Block::Text("World".to_string())]
                },
            ]
        );
        let html = doc.to_authoring_html();
        assert!(html.contains(r#"style="color:#ff0000""#), "{html}");
        assert!(html.contains(">World</span>"), "{html}");
    }

    #[test]
    fn color_accepts_aliases_and_short_hex() {
        for spec in [
            "color:#f00:Red",
            "fg:#00ff00:Green",
            "foreground:#0000ff:Blue",
            "text-color:#abc:Short",
            "tc:#aabbcc:Full",
        ] {
            let doc = Document::parse(&format!("{{{{{spec}}}}}"), false);
            assert!(
                matches!(doc.blocks.as_slice(), [Block::Color { .. }]),
                "spec: {spec}"
            );
        }
        let doc = Document::parse("{{color:#f00:Red}}", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Color {
                color: "#ff0000".to_string(),
                content: vec![Block::Text("Red".to_string())]
            }]
        );
    }

    #[test]
    fn invalid_color_is_kept_literal() {
        for inner in ["color:#ff0000", "color:bad:x", "color::x"] {
            let doc = Document::parse(&format!("a {{{{{inner}}}}} b"), false);
            assert_eq!(
                doc.blocks,
                vec![Block::Text(format!("a {{{{{inner}}}}} b"))],
                "inner: {inner}"
            );
        }
    }

    #[test]
    fn inline_font_directive_is_parsed() {
        let doc = Document::parse("Hello, {{font:roboto:World}}", false);
        assert_eq!(
            doc.blocks,
            vec![
                Block::Text("Hello, ".to_string()),
                Block::Font {
                    family: "roboto".to_string(),
                    content: vec![Block::Text("World".to_string())]
                },
            ]
        );
        let html = doc.to_authoring_html();
        assert!(html.contains("data-lbl-font=\"roboto\""), "{html}");
        assert!(html.contains(">World</span>"), "{html}");
    }

    #[test]
    fn font_accepts_aliases() {
        for spec in ["font:mono:code", "font-family:serif:Text", "ff:oswald:BIG"] {
            let doc = Document::parse(&format!("{{{{{spec}}}}}"), false);
            assert!(
                matches!(doc.blocks.as_slice(), [Block::Font { .. }]),
                "spec: {spec}"
            );
        }
    }

    #[test]
    fn invalid_font_is_kept_literal() {
        for inner in ["font:roboto", "font:unknown:x", "font::x"] {
            let doc = Document::parse(&format!("a {{{{{inner}}}}} b"), false);
            assert_eq!(
                doc.blocks,
                vec![Block::Text(format!("a {{{{{inner}}}}} b"))],
                "inner: {inner}"
            );
        }
    }

    #[test]
    fn inline_size_directive_is_parsed() {
        let doc = Document::parse("Hello, {{size:1.5:World}}", false);
        assert_eq!(
            doc.blocks,
            vec![
                Block::Text("Hello, ".to_string()),
                Block::Sized {
                    scale: 1.5,
                    content: vec![Block::Text("World".to_string())]
                },
            ]
        );
        let html = doc.to_authoring_html();
        assert!(html.contains("font-size:1.5em"), "{html}");
        assert!(html.contains(">World</span>"), "{html}");
    }

    #[test]
    fn size_accepts_x_and_percent_and_aliases() {
        for spec in ["size:2x:Big", "font-size:200%:Big", "fs:2:Big"] {
            let doc = Document::parse(&format!("{{{{{spec}}}}}"), false);
            assert_eq!(
                doc.blocks,
                vec![Block::Sized {
                    scale: 2.0,
                    content: vec![Block::Text("Big".to_string())]
                }],
                "spec: {spec}"
            );
        }
    }

    #[test]
    fn invalid_size_is_kept_literal() {
        // Missing text, non-numeric scale, and non-positive scale all fall back.
        for inner in ["size:1.5", "size:big:x", "size:0:x", "size:-1:x"] {
            let doc = Document::parse(&format!("a {{{{{inner}}}}} b"), false);
            assert_eq!(
                doc.blocks,
                vec![Block::Text(format!("a {{{{{inner}}}}} b"))],
                "inner: {inner}"
            );
        }
    }

    #[test]
    fn mixed_color_text_runs_in_one_lbl_text() {
        let doc = Document::parse("מורן {{color:#ff0000:לימון}} יפה", false);
        let html = doc.to_authoring_html();
        assert!(
            html.contains(
                "<span class=\"lbl-text\"><span class=\"lbl-text-inlines\">מורן <span style=\"color:#ff0000\">לימון</span> יפה</span></span>"
            ),
            "{html}"
        );
        assert!(!html.contains("lbl-row"), "{html}");
    }

    #[test]
    fn color_directive_preserves_surrounding_spaces() {
        let doc = Document::parse("aa {{color:#e11515:bb}} cc", false);
        let html = doc.to_authoring_html();
        assert!(
            html.contains(
                "<span class=\"lbl-text\"><span class=\"lbl-text-inlines\">aa <span style=\"color:#e11515\">bb</span> cc</span></span>"
            ),
            "{html}"
        );
        assert!(!html.contains("lbl-row"), "{html}");
    }

    #[test]
    fn mixed_text_barcode_text_uses_flex_row_siblings() {
        let doc = Document::parse("aa {{barcode:12346}} bb", false);
        let html = doc.to_authoring_html();
        assert!(
            html.contains(
                "<div class=\"lbl-row lbl-center\"><span class=\"lbl-text\"><span class=\"lbl-text-inlines\">aa </span></span><barcode type=\"CODE128\">12346</barcode><span class=\"lbl-text\"><span class=\"lbl-text-inlines\"> bb</span></span></div>"
            ),
            "{html}"
        );
    }

    #[test]
    fn inline_directives_flow_in_a_row() {
        let doc = Document::parse(
            "Ship {{size:1.2:Alice}}{{barcode:L42}}{{qr:https://track/42}}",
            false,
        );
        let html = doc.to_authoring_html();
        assert!(html.contains("lbl-row"), "{html}");
        assert!(html.contains("<qr>https://track/42</qr>"), "{html}");
    }

    #[test]
    fn multiline_text_with_qr_on_same_line_uses_row() {
        let doc = Document::parse(
            "a\nb {{qr light=\"#FFFFFF\"}}https://example.com{{/qr}}\nc",
            false,
        );
        let html = doc.to_authoring_html();
        assert!(
            html.contains(
                "<div class=\"lbl-row lbl-center\"><span class=\"lbl-text\"><span class=\"lbl-text-inlines\">b </span></span><qr light=\"#FFFFFF\">https://example.com</qr></div>"
            ),
            "{html}"
        );
        assert_eq!(
            html.matches("<div class=\"lbl-text\">").count(),
            2,
            "{html}"
        );
    }

    #[test]
    fn multiline_text_stays_outside_row() {
        let doc = Document::parse("Line 1\nLine 2", false);
        let html = doc.to_authoring_html();
        assert!(!html.contains("lbl-row"), "{html}");
        assert_eq!(
            html.matches("<div class=\"lbl-text\">").count(),
            2,
            "{html}"
        );
    }

    #[test]
    fn multiline_after_row_emits_one_div_per_line() {
        let doc = Document::parse("OO{{barcode:O360}}\nOO\nOO\nOO", false);
        let html = doc.to_authoring_html();
        assert!(html.contains("lbl-row"), "{html}");
        assert_eq!(
            html.matches("<div class=\"lbl-text\">").count(),
            3,
            "{html}"
        );
        assert!(!html.contains("<br>"), "{html}");
    }

    #[test]
    fn html_is_escaped() {
        let doc = Document::parse("<script>", false);
        assert!(doc.to_authoring_html().contains("&lt;script&gt;"));
    }

    #[test]
    fn nested_color_with_barcode() {
        let doc = Document::parse("{{color:#e90b0b:aa {{barcode:12345}} cc}}", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Color {
                color: "#e90b0b".to_string(),
                content: vec![
                    Block::Text("aa ".to_string()),
                    Block::Barcode {
                        symbology: "CODE128".into(),
                        data: "12345".into(),
                        height_mode: BarcodeHeightMode::Normal,
                    },
                    Block::Text(" cc".to_string()),
                ]
            }]
        );
        let html = doc.to_authoring_html();
        assert!(html.contains(r#"style="color:#e90b0b""#), "{html}");
        assert!(html.contains(">aa "), "{html}");
        assert!(
            html.contains("<barcode type=\"CODE128\">12345</barcode>"),
            "{html}"
        );
        assert!(html.contains("> cc</span>"), "{html}");
    }

    #[test]
    fn nested_size_with_color() {
        let doc = Document::parse("{{size:1.5:big {{color:#f00:red}} text}}", false);
        assert_eq!(
            doc.blocks,
            vec![Block::Sized {
                scale: 1.5,
                content: vec![
                    Block::Text("big ".to_string()),
                    Block::Color {
                        color: "#ff0000".to_string(),
                        content: vec![Block::Text("red".to_string())]
                    },
                    Block::Text(" text".to_string()),
                ]
            }]
        );
    }

    #[test]
    fn image_directive_and_flag() {
        let mut doc = Document::parse("{{image:./a.png}}", false);
        doc.push_image("https://x/y.png");
        assert_eq!(
            doc.blocks,
            vec![
                Block::Image("./a.png".into()),
                Block::Image("https://x/y.png".into()),
            ]
        );
    }
}
