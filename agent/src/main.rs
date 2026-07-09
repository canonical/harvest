mod config;
mod console;
mod executor;
mod sse_client;
mod tunnel;
mod ws_url;

use anyhow::Result;
use clap::Parser;
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(name = "harvest-agent", about = "Harvest remote agent daemon")]
struct Cli {
    #[arg(short, long, default_value = "/etc/harvest-agent/config.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = config::Config::from_file(&cli.config)?;
    tracing::info!(server_url = %cfg.server_url, "harvest-agent starting");

    let shared = Arc::new(Mutex::new(cfg));
    sse_client::run_with_reconnect(shared, &cli.config).await;
    Ok(())
}
