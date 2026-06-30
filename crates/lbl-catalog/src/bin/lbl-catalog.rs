//! `lbl-catalog` — browse the media catalog and resolve SKUs.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lbl_catalog::Catalog;

#[derive(Parser)]
#[command(
    name = "lbl-catalog",
    about = "Browse known media and printer compatibility",
    color = clap::ColorChoice::Auto,
)]
struct Cli {
    /// Additional catalog files to overlay (TOML/JSON).
    #[arg(long = "catalog", value_name = "FILE", global = true)]
    overlays: Vec<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List all known media.
    List,
    /// Show a single entry by key/SKU/alias.
    Show {
        /// The SKU or alias, e.g. `11352` or `S0722520`.
        key: String,
    },
    /// List media compatible with a printer model.
    Compatible {
        /// Printer model, e.g. "LabelWriter 550".
        #[arg(long)]
        printer: String,
    },
    /// Free-text search.
    Search {
        /// Query string.
        query: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let catalog = if cli.overlays.is_empty() {
        Catalog::bundled()?
    } else {
        Catalog::load_with_overlays(&cli.overlays)?
    };

    match cli.command {
        Command::List => {
            for e in catalog.entries() {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
        Command::Show { key } => {
            let entry = catalog
                .lookup(&key)
                .with_context(|| format!("no catalog entry for key '{key}'"))?;
            println!("{}", serde_json::to_string_pretty(entry)?);
        }
        Command::Compatible { printer } => {
            for e in catalog.compatible_with(&printer) {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
        Command::Search { query } => {
            for e in catalog.search(&query) {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
    }
    Ok(())
}
