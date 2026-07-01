//! `lbl-config` — inspect the effective layered configuration.

use anyhow::Result;
use clap::{Parser, Subcommand};
use lbl_config::{describe_sources, format_effective, Loader};

#[derive(Parser)]
#[command(name = "lbl-config", about = "Inspect lbl's layered configuration", color = clap::ColorChoice::Auto, styles = lbl_cli::CLAP_STYLING)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the fully-merged effective configuration as JSON.
    Show,
    /// Print where each effective value came from (provenance).
    Sources,
    /// Print the resolved configuration file paths.
    Paths,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let loader = Loader::new();

    match cli.command {
        Command::Show => {
            println!("{}", format_effective(&loader, false)?);
        }
        Command::Sources => {
            for (key, source) in describe_sources(loader.figment()) {
                println!("{key}\t{source}");
            }
        }
        Command::Paths => {
            let catalog_extra = loader.load().map(|c| c.catalog.extra_paths).unwrap_or_default();
            print!(
                "{}",
                lbl_config::format_paths_report(
                    loader.paths(),
                    &catalog_extra,
                    lbl_config::stdout_color(),
                )
            );
        }
    }
    Ok(())
}
