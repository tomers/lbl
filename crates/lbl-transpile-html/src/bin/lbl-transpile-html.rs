//! `lbl-transpile-html` — authoring HTML to browser-ready HTML.

use std::io::{Read, Write};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use lbl_core::job::OutputMode;
use lbl_transpile_html::{transpile, AssetsBase, TranspileOptions};

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// Deterministic output for the rasterizer/printer.
    Print,
    /// Screen-oriented, gallery-friendly output.
    Preview,
}

impl From<Mode> for OutputMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Print => OutputMode::Print,
            Mode::Preview => OutputMode::Preview,
        }
    }
}

#[derive(Parser)]
#[command(
    name = "lbl-transpile-html",
    about = "Transpile authoring HTML (<qr>, <barcode>, flex) into browser-ready HTML"
)]
struct Cli {
    /// Input authoring HTML file. If omitted, read from stdin.
    input: Option<std::path::PathBuf>,

    /// Output target mode.
    #[arg(long, value_enum, default_value = "print")]
    mode: Mode,

    /// Serve third-party libraries from this base URL/path instead of the CDN.
    #[arg(long)]
    assets_base: Option<String>,

    /// Label index within a batch (preview addressing).
    #[arg(long)]
    index: Option<usize>,

    /// Total labels in the batch (preview addressing).
    #[arg(long)]
    count: Option<usize>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let input = match &cli.input {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    let opts = TranspileOptions {
        mode: cli.mode.into(),
        assets_base: cli
            .assets_base
            .map(AssetsBase::Local)
            .unwrap_or(AssetsBase::Cdn),
        index: cli.index,
        count: cli.count,
    };

    let out = transpile(&input, &opts);
    std::io::stdout().lock().write_all(out.as_bytes())?;
    Ok(())
}
