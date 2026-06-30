//! Parsing of text + inline directives into an authoring document.

use crate::qr::{parse_qr_attrs, QrOptions};
use crate::DEFAULT_SYMBOLOGY;

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
        /// The text to render at this size.
        text: String,
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
                format!("<div class=\"lbl-text\">{}</div>", text_to_html(t))
            }
            Block::Sized { scale, text } => format!(
                "<span class=\"lbl-text\" style=\"font-size:{}em\">{}</span>",
                fmt_scale(*scale),
                text_to_html(text)
            ),
            Block::Qr { payload, options } => {
                format!("<qr{}>{}</qr>", options.to_attrs(), escape(payload))
            }
            Block::Barcode { symbology, data } => format!(
                "<barcode type=\"{}\">{}</barcode>",
                escape(symbology),
                escape(data)
            ),
            Block::Image(uri) => format!("<img src=\"{}\" />", escape(uri)),
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
        let mut out = String::from("<div class=\"lbl-label\">");
        for block in &self.blocks {
            out.push_str(&block.to_authoring_html());
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

/// Parse a barcode `spec` (`SYMBOLOGY:data` or just `data`) into a
/// [`Block::Barcode`], defaulting the symbology when none is given.
pub fn barcode_from_spec(spec: &str) -> Block {
    match spec.split_once(':') {
        Some((sym, data)) if !sym.is_empty() && !data.is_empty() => Block::Barcode {
            symbology: sym.to_string(),
            data: data.to_string(),
        },
        _ => Block::Barcode {
            symbology: DEFAULT_SYMBOLOGY.to_string(),
            data: spec.to_string(),
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

    let close = input[start + 2..].find("}}")?;
    let inner = &input[start + 2..start + 2 + close];
    let block = directive_from_inner(inner)?;
    Some((block, start + 2 + close + 2))
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

/// Push the accumulated text as a block, trimming surrounding whitespace (which
/// is usually just separation around directives) and dropping it if empty.
fn flush_text(buf: &mut String, blocks: &mut Vec<Block>) {
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        blocks.push(Block::Text(trimmed.to_string()));
    }
    buf.clear();
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
        "barcode" => Some(barcode_from_spec(rest)),
        "image" | "img" => Some(Block::Image(rest.to_string())),
        "size" | "font-size" | "fs" => sized_from_spec(rest),
        _ => None,
    }
}

/// Parse a sizing spec `SCALE:TEXT` (e.g. `1.5:World`) into a [`Block::Sized`].
/// `SCALE` accepts a bare multiplier (`1.5`), an explicit `x` suffix (`1.5x`),
/// or a percentage (`150%`). Returns `None` if the scale is invalid or the text
/// is empty, so the original `{{...}}` is left literal.
fn sized_from_spec(spec: &str) -> Option<Block> {
    let (scale_str, text) = spec.split_once(':')?;
    let scale = parse_scale(scale_str.trim())?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(Block::Sized {
        scale,
        text: text.to_string(),
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
                Block::Text("ship to".to_string()),
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
                    data: "123".into()
                },
                Block::Barcode {
                    symbology: "CODE128".into(),
                    data: "456".into()
                },
            ]
        );
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
    fn inline_size_directive_is_parsed() {
        let doc = Document::parse("Hello, {{size:1.5:World}}", false);
        assert_eq!(
            doc.blocks,
            vec![
                Block::Text("Hello,".to_string()),
                Block::Sized {
                    scale: 1.5,
                    text: "World".to_string()
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
                    text: "Big".to_string()
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
    fn html_is_escaped() {
        let doc = Document::parse("<script>", false);
        assert!(doc.to_authoring_html().contains("&lt;script&gt;"));
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
