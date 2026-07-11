//! `lbl-server` — serve the HTTP API for the lbl pipeline.

use anyhow::Result;
use clap::Parser;
use lbl_server::{init_tracing, router, AppState};

#[derive(Parser)]
#[command(name = "lbl-server", about = "HTTP API for the lbl pipeline", color = clap::ColorChoice::Auto, styles = lbl_cli::CLAP_STYLING)]
struct Cli {
    /// Address to bind, e.g. 127.0.0.1:8787.
    #[arg(long, default_value = "127.0.0.1:8787")]
    bind: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let state = AppState::discover()?;
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(&cli_bind()).await?;
    tracing::info!("lbl-server listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}

fn cli_bind() -> String {
    Cli::parse().bind
}
