//! `lbl-markdown` — turn Markdown (and directives) into authoring HTML on stdout.

use std::io::Read;

use anyhow::Result;
use clap::Parser;
use lbl_markdown::MarkdownDocument;

#[derive(Parser)]
#[command(
    name = "lbl-markdown",
    about = "Convert Markdown and directives into lbl authoring HTML",
    long_about = "Reads Markdown from a positional argument, or, if none is given, from stdin. \
Emits authoring HTML on stdout for the rest of the pipeline.\n\n\
Standard Markdown (headings, lists, emphasis, tables, ...) is supported, and the \
lbl inline mini-syntax is still applied anywhere in the document: \
[[qr:...]], [[barcode:[SYMBOLOGY:]data]], [[image:URI]].",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    /// The Markdown to render. If omitted, it is read from stdin.
    #[arg(value_name = "MARKDOWN")]
    markdown: Vec<String>,

    /// Add a QR code with this payload (repeatable).
    #[arg(long = "qr", value_name = "PAYLOAD")]
    qr: Vec<String>,

    /// Add a barcode `[SYMBOLOGY:]data` (repeatable).
    #[arg(long = "barcode", value_name = "SPEC")]
    barcode: Vec<String>,

    /// Add an image by local path or URL (repeatable).
    #[arg(long = "image", value_name = "URI")]
    image: Vec<String>,

    /// Emit only the `<div class="lbl-label">` fragment, not a full document.
    #[arg(long)]
    fragment: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let markdown = if cli.markdown.is_empty() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        cli.markdown.join(" ")
    };

    let mut doc = MarkdownDocument::parse(&markdown);
    for q in cli.qr {
        doc.push_qr(q);
    }
    for b in cli.barcode {
        doc.push_barcode(&b);
    }
    for img in cli.image {
        doc.push_image(img);
    }

    if cli.fragment {
        println!("{}", doc.to_authoring_html());
    } else {
        print!("{}", doc.to_authoring_document());
    }
    Ok(())
}
