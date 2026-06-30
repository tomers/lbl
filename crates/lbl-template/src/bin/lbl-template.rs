//! `lbl-template` — render data through a template into N authoring-HTML labels.

use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use lbl_template::{data, DataFormat, DefaultResolver, Engine, RenderOptions};

#[derive(Parser)]
#[command(
    name = "lbl-template",
    about = "Render data (JSON/TOML/YAML) through a template into one or more labels",
    color = clap::ColorChoice::Auto,
)]
struct Cli {
    /// Template file. If omitted, the template is read from stdin.
    #[arg(long)]
    template: Option<PathBuf>,

    /// Data source: a file path or an http(s) URL. If omitted, only frontmatter
    /// data (if any) is used.
    #[arg(long)]
    data: Option<String>,

    /// Force the data format (otherwise inferred from extension/content).
    #[arg(long, value_parser = ["json", "toml", "yaml"])]
    data_format: Option<String>,

    /// JSON-pointer to an array within the data to batch over (e.g. `/items`).
    #[arg(long)]
    each: Option<String>,

    /// Fetch and inline `<img>` resources as data URIs.
    #[arg(long)]
    inline_resources: bool,

    /// Write labels to this directory (label-0000.html + manifest.json) instead
    /// of stdout.
    #[arg(long)]
    out_dir: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let template = match &cli.template {
        Some(path) => std::fs::read_to_string(path)
            .with_context(|| format!("reading template {}", path.display()))?,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };

    let external_data = match &cli.data {
        Some(src) => Some(load_data(src, cli.data_format.as_deref())?),
        None => None,
    };

    let opts = RenderOptions {
        each: cli.each.clone(),
    };
    let engine = Engine::new();

    let labels = if cli.inline_resources {
        engine.render_with_resources(&template, external_data, &opts, &DefaultResolver)?
    } else {
        engine.render(&template, external_data, &opts)?
    };

    match &cli.out_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            let mut manifest = Vec::new();
            for label in &labels {
                let name = format!("label-{:04}.html", label.index);
                std::fs::write(dir.join(&name), &label.html)?;
                manifest.push(serde_json::json!({"index": label.index, "file": name}));
            }
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::to_string_pretty(&serde_json::json!({"labels": manifest}))?,
            )?;
            eprintln!("wrote {} label(s) to {}", labels.len(), dir.display());
        }
        None => {
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            if labels.len() == 1 {
                write!(out, "{}", labels[0].html)?;
            } else {
                for label in &labels {
                    let line = serde_json::json!({"index": label.index, "html": label.html});
                    writeln!(out, "{}", serde_json::to_string(&line)?)?;
                }
            }
        }
    }
    Ok(())
}

fn load_data(src: &str, format: Option<&str>) -> Result<serde_json::Value> {
    let text = if src.starts_with("http://") || src.starts_with("https://") {
        reqwest::blocking::get(src)
            .and_then(|r| r.error_for_status())
            .with_context(|| format!("fetching data {src}"))?
            .text()?
    } else {
        std::fs::read_to_string(src).with_context(|| format!("reading data {src}"))?
    };

    let fmt = format
        .and_then(|f| match f {
            "json" => Some(DataFormat::Json),
            "toml" => Some(DataFormat::Toml),
            "yaml" => Some(DataFormat::Yaml),
            _ => None,
        })
        .or_else(|| {
            PathBuf::from(src)
                .extension()
                .and_then(|e| e.to_str())
                .and_then(DataFormat::from_extension)
        });

    Ok(match fmt {
        Some(f) => data::parse(&text, f)?,
        None => data::parse_auto(&text)?,
    })
}
