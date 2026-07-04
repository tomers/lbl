//! Markdown -> authoring HTML, with lbl inline directives applied.

use lbl_text::{barcode_from_spec, scan_directive_at, Block};
use pulldown_cmark::{html, Options, Parser};

/// A parsed Markdown label: the Markdown body rendered to authoring HTML, plus
/// any directive blocks appended from CLI flags.
#[derive(Debug, Clone, Default)]
pub struct MarkdownDocument {
    /// Markdown rendered to HTML, with `{{...}}` directives substituted for
    /// their authoring elements (`<qr>`, `<barcode>`, `<img>`).
    body: String,
    /// Directives appended after the body (e.g. from `--qr`/`--barcode` flags).
    appended: Vec<Block>,
}

impl MarkdownDocument {
    /// Parse a Markdown `input` into an authoring document.
    ///
    /// Inline `{{...}}` directives are recognized anywhere in the source (even
    /// inside headings or list items) and rendered as the corresponding
    /// authoring element; everything else is converted from Markdown to HTML.
    pub fn parse(input: &str) -> Self {
        let (template, directives) = extract_directives(input);
        let template = expand_underline(&template);
        let mut body = markdown_to_html(&template);
        for (placeholder, block) in directives {
            body = body.replace(&placeholder, &block.to_authoring_html());
        }
        MarkdownDocument {
            body,
            appended: Vec::new(),
        }
    }

    /// Append a QR directive (from a flag).
    pub fn push_qr(&mut self, payload: impl Into<String>) {
        self.appended.push(Block::Qr {
            payload: payload.into(),
            options: lbl_text::QrOptions::default(),
        });
    }

    /// Append a barcode directive (from a flag). `spec` may be
    /// `SYMBOLOGY:data` or just `data` (defaulting the symbology).
    pub fn push_barcode(&mut self, spec: &str) {
        self.appended.push(barcode_from_spec(spec));
    }

    /// Append an image directive (from a flag).
    pub fn push_image(&mut self, uri: impl Into<String>) {
        self.appended.push(Block::Image(uri.into()));
    }

    /// Render the authoring HTML fragment (the `<div class="lbl-label">` root
    /// and its children) consumed by `lbl-transpile-html`.
    pub fn to_authoring_html(&self) -> String {
        let mut out = String::from("<div class=\"lbl-label\">");
        out.push_str(self.body.trim());
        for block in &self.appended {
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

/// A unique, Markdown-safe placeholder for the `n`th directive.
///
/// Uses only lowercase ASCII letters and digits so that no Markdown construct
/// (emphasis, code, links, ...) can alter it between extraction and
/// substitution.
fn placeholder(n: usize) -> String {
    format!("lblxdirectivexplaceholderx{n}xend")
}

/// Scan `input` for `{{type:...}}` directives, replacing each recognized one
/// with a placeholder and returning the rewritten source plus the ordered list
/// of `(placeholder, block)` pairs. Unrecognized `{{...}}` is left untouched.
fn extract_directives(input: &str) -> (String, Vec<(String, Block)>) {
    let mut out = String::with_capacity(input.len());
    let mut directives = Vec::new();
    let mut i = 0;

    while i < input.len() {
        if input[i..].starts_with("{{") {
            if let Some((block, end)) = scan_directive_at(input, i) {
                let ph = placeholder(directives.len());
                out.push_str(&ph);
                directives.push((ph, block));
                i = end;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    (out, directives)
}

/// Expand `++underline++` spans to `<u>…</u>` before Markdown parsing.
///
/// Skips fenced and inline code so literal `++` in code is preserved.
fn expand_underline(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < input.len() {
        if input[i..].starts_with("```") {
            if let Some(end) = input[i + 3..].find("```") {
                let end_idx = i + 3 + end + 3;
                out.push_str(&input[i..end_idx]);
                i = end_idx;
                continue;
            }
        }
        if input.as_bytes()[i] == b'`' {
            if let Some(end) = input[i + 1..].find('`') {
                let end_idx = i + 1 + end + 1;
                out.push_str(&input[i..end_idx]);
                i = end_idx;
                continue;
            }
        }
        if input[i..].starts_with("++") {
            if let Some(rel) = input[i + 2..].find("++") {
                let inner = &input[i + 2..i + 2 + rel];
                out.push_str("<u>");
                out.push_str(inner);
                out.push_str("</u>");
                i = i + 2 + rel + 2;
                continue;
            }
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn markdown_to_html(input: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);

    let parser = Parser::new_ext(input, options);
    let mut html_out = String::new();
    html::push_html(&mut html_out, parser);
    html_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_emphasis_become_html() {
        let doc = MarkdownDocument::parse("# Title\n\nsome **bold** text");
        let html = doc.to_authoring_html();
        assert!(html.contains("<h1>Title</h1>"), "{html}");
        assert!(html.contains("<strong>bold</strong>"), "{html}");
    }

    #[test]
    fn inline_qr_directive_with_options() {
        let doc = MarkdownDocument::parse("**Hello** {{qr ec=low}}hi{{/qr}}");
        let html = doc.to_authoring_html();
        assert!(html.contains("<strong>Hello</strong>"), "{html}");
        assert!(html.contains(r#"<qr ec="L">hi</qr>"#), "{html}");
    }

    #[test]
    fn inline_qr_directive_is_applied() {
        let doc = MarkdownDocument::parse("ship to {{qr:https://x.y}}");
        let html = doc.to_authoring_html();
        assert!(html.contains("<qr>https://x.y</qr>"), "{html}");
        assert!(
            !html.contains("lblxdirective"),
            "placeholder leaked: {html}"
        );
    }

    #[test]
    fn directive_inside_heading() {
        let doc = MarkdownDocument::parse("# Ship {{qr:abc}}");
        let html = doc.to_authoring_html();
        assert!(html.contains("<qr>abc</qr>"), "{html}");
        assert!(html.contains("<h1>"), "{html}");
    }

    #[test]
    fn size_directive_renders_inline() {
        let doc = MarkdownDocument::parse("Hello, {{size:1.5:World}}!");
        let html = doc.to_authoring_html();
        // The sized span stays inline within the paragraph alongside the text.
        assert!(
            html.contains(
                "Hello, <span class=\"lbl-text\" style=\"font-size:1.5em\">World</span>!"
            ),
            "{html}"
        );
    }

    #[test]
    fn size_directive_inside_heading() {
        let doc = MarkdownDocument::parse("# Order {{size:2:#42}}");
        let html = doc.to_authoring_html();
        assert!(html.contains("<h1>Order <span"), "{html}");
        assert!(html.contains("font-size:2em\">#42</span></h1>"), "{html}");
    }

    #[test]
    fn barcode_with_and_without_symbology() {
        let doc = MarkdownDocument::parse("{{barcode:EAN13:123}} {{barcode:456}}");
        let html = doc.to_authoring_html();
        assert!(
            html.contains("<barcode type=\"EAN13\">123</barcode>"),
            "{html}"
        );
        assert!(
            html.contains("<barcode type=\"CODE128\">456</barcode>"),
            "{html}"
        );
    }

    #[test]
    fn image_directive_and_flag() {
        let mut doc = MarkdownDocument::parse("{{image:./a.png}}");
        doc.push_image("https://x/y.png");
        let html = doc.to_authoring_html();
        assert!(html.contains("<img src=\"./a.png\" />"), "{html}");
        assert!(html.contains("<img src=\"https://x/y.png\" />"), "{html}");
    }

    #[test]
    fn unrecognized_directive_kept_literal() {
        let doc = MarkdownDocument::parse("a {{unknown:y}} b");
        let html = doc.to_authoring_html();
        assert!(html.contains("{{unknown:y}}"), "{html}");
    }

    #[test]
    fn flag_directives_appended() {
        let mut doc = MarkdownDocument::parse("hello");
        doc.push_qr("https://x.y");
        doc.push_barcode("EAN13:42");
        let html = doc.to_authoring_html();
        assert!(html.contains("<qr>https://x.y</qr>"), "{html}");
        assert!(
            html.contains("<barcode type=\"EAN13\">42</barcode>"),
            "{html}"
        );
    }

    #[test]
    fn underline_marker_becomes_u_tag() {
        let doc = MarkdownDocument::parse("Ship ++fast++");
        let html = doc.to_authoring_html();
        assert!(html.contains("<u>fast</u>"), "{html}");
        assert!(!html.contains("++"), "{html}");
    }

    #[test]
    fn underline_with_nested_emphasis() {
        let doc = MarkdownDocument::parse("++**fast**++");
        let html = doc.to_authoring_html();
        assert!(html.contains("<u><strong>fast</strong></u>"), "{html}");
    }

    #[test]
    fn underline_marker_skipped_in_inline_code() {
        let doc = MarkdownDocument::parse("use `++x++` literal");
        let html = doc.to_authoring_html();
        assert!(html.contains("<code>++x++</code>"), "{html}");
        assert!(!html.contains("<u>"), "{html}");
    }

    #[test]
    fn wrapped_in_label_root() {
        let doc = MarkdownDocument::parse("hi");
        assert!(doc
            .to_authoring_html()
            .starts_with("<div class=\"lbl-label\">"));
    }
}
