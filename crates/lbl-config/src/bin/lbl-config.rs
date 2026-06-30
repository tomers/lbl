//! `lbl-config` — inspect the effective layered configuration.

use anyhow::Result;
use clap::{Parser, Subcommand};
use lbl_config::{describe_sources, Loader};

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
            let cfg = loader.load()?;
            println!("{}", serde_json::to_string_pretty(&cfg)?);
        }
        Command::Sources => {
            for (key, source) in describe_sources(loader.figment()) {
                println!("{key}\t{source}");
            }
        }
        Command::Paths => {
            let p = loader.paths();
            println!("system   {}", p.system.display());
            println!("user     {}", p.user.display());
            println!("project  {}", p.project.display());
            println!("profiles {}", p.profiles.display());
            println!("cache    {}", p.cache.display());
        }
    }
    Ok(())
}
