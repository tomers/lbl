//! Parsing of text + inline directives into an authoring document.

use crate::DEFAULT_SYMBOLOGY;

/// A content block in a label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Literal text (may contain newlines).
    Text(String),
    /// A QR code carrying the given payload.
    Qr(String),
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

/// A parsed label document: an ordered list of [`Block`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
        self.blocks.push(Block::Qr(payload.into()));
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
            match block {
                Block::Text(t) => {
                    out.push_str("<div class=\"lbl-text\">");
                    out.push_str(&text_to_html(t));
                    out.push_str("</div>");
                }
                Block::Qr(payload) => {
                    out.push_str("<qr>");
                    out.push_str(&escape(payload));
                    out.push_str("</qr>");
                }
                Block::Barcode { symbology, data } => {
                    out.push_str("<barcode type=\"");
                    out.push_str(&escape(symbology));
                    out.push_str("\">");
                    out.push_str(&escape(data));
                    out.push_str("</barcode>");
                }
                Block::Image(uri) => {
                    out.push_str("<img src=\"");
                    out.push_str(&escape(uri));
                    out.push_str("\" />");
                }
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

fn barcode_from_spec(spec: &str) -> Block {
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

/// Scan `input` for `{{type:...}}` directives, returning interleaved text and
/// directive blocks. Unrecognized `{{...}}` is kept as literal text.
fn parse_inline(input: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut text_buf = String::new();
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < input.len() {
        if bytes[i] == b'{' && i + 1 < input.len() && bytes[i + 1] == b'{' {
            if let Some(close) = input[i + 2..].find("}}") {
                let inner = &input[i + 2..i + 2 + close];
                if let Some(block) = directive_from_inner(inner) {
                    flush_text(&mut text_buf, &mut blocks);
                    blocks.push(block);
                    i = i + 2 + close + 2;
                    continue;
                }
            }
        }
        // Not a recognized directive: consume one char as literal text.
        let ch = input[i..].chars().next().unwrap();
        text_buf.push(ch);
        i += ch.len_utf8();
    }

    flush_text(&mut text_buf, &mut blocks);
    blocks
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

fn directive_from_inner(inner: &str) -> Option<Block> {
    let (kind, rest) = inner.split_once(':')?;
    let kind = kind.trim().to_ascii_lowercase();
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    match kind.as_str() {
        "qr" => Some(Block::Qr(rest.to_string())),
        "barcode" => Some(barcode_from_spec(rest)),
        "image" | "img" => Some(Block::Image(rest.to_string())),
        _ => None,
    }
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
                Block::Qr("https://x.y".to_string()),
            ]
        );
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
