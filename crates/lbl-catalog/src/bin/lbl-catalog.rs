//! `lbl-catalog` — browse the media and printer catalog.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lbl_catalog::Catalog;

#[derive(Parser)]
#[command(
    name = "lbl-catalog",
    about = "Browse known media, printers, and compatibility",
    color = clap::ColorChoice::Auto,
    styles = lbl_cli::CLAP_STYLING,
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
    /// Show a single media entry by key/SKU/alias.
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
    /// Free-text search over media and printers.
    Search {
        /// Query string.
        query: String,
    },
    /// Browse known printer models.
    Printers {
        #[command(subcommand)]
        command: PrinterCommand,
    },
}

#[derive(Subcommand)]
enum PrinterCommand {
    /// List all known printers.
    List,
    /// Show a single printer entry by key/alias.
    Show {
        /// Printer key, e.g. `LabelWriter 550` or `D110`.
        key: String,
    },
}

fn load_catalog(overlays: &[String]) -> Result<Catalog> {
    if overlays.is_empty() {
        Ok(Catalog::bundled()?)
    } else {
        Ok(Catalog::load_with_overlays(overlays)?)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let catalog = load_catalog(&cli.overlays)?;

    match cli.command {
        Command::List => {
            for e in catalog.entries() {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
        Command::Show { key } => {
            let entry = catalog
                .lookup(&key)
                .with_context(|| format!("no media entry for key '{key}'"))?;
            println!("{}", serde_json::to_string_pretty(entry)?);
        }
        Command::Compatible { printer } => {
            for e in catalog.compatible_with(&printer) {
                println!("{:<12} {}", e.canonical_key(), e.name);
            }
        }
        Command::Search { query } => {
            for e in catalog.search(&query) {
                println!("media  {:<12} {}", e.canonical_key(), e.name);
            }
            for p in catalog.search_printers(&query) {
                println!("printer {:<20} {}", p.canonical_key(), p.name);
            }
        }
        Command::Printers { command } => match command {
            PrinterCommand::List => {
                for p in catalog.printers() {
                    println!("{:<20} {}", p.canonical_key(), p.name);
                }
            }
            PrinterCommand::Show { key } => {
                let printer = catalog
                    .lookup_printer(&key)
                    .or_else(|| catalog.match_printer(&key))
                    .with_context(|| format!("no printer entry for key '{key}'"))?;
                println!("{}", serde_json::to_string_pretty(printer)?);
            }
        },
    }
    Ok(())
}
