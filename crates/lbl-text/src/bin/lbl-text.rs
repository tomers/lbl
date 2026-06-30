//! `lbl-text` — turn text (and directives) into authoring HTML on stdout.

use std::io::Read;

use anyhow::Result;
use clap::Parser;
use lbl_text::Document;

#[derive(Parser)]
#[command(
    name = "lbl-text",
    about = "Convert plain text and directives into lbl authoring HTML",
    long_about = "Reads text from positional arguments (joined with spaces) or, if none are given, \
from stdin. Emits authoring HTML on stdout for the rest of the pipeline.\n\n\
Inline mini-syntax (default): {{qr:...}}, {{barcode:[SYMBOLOGY:]data}}, {{image:URI}}.",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
)]
struct Cli {
    /// The text to render. If omitted, text is read from stdin.
    #[arg(value_name = "TEXT")]
    text: Vec<String>,

    /// Treat input literally; do not parse inline {{...}} directives.
    #[arg(long)]
    raw: bool,

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

    let text = if cli.text.is_empty() {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf.trim_end_matches('\n').to_string()
    } else {
        cli.text.join(" ")
    };

    let mut doc = Document::parse(&text, cli.raw);
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
