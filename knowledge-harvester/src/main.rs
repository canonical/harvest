use knowledge_harvester::{config, pipeline};

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "knowledge-harvester", about = "Harvest code knowledge from git repositories")]
struct Cli {
    #[arg(short, long, default_value = "harvester.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Ingest all configured repositories once and exit")]
    Run,
    #[command(about = "Continuously poll repositories and ingest new versions on a fixed interval")]
    Watch {
        #[arg(short, long, default_value = "300", help = "Polling interval in seconds")]
        interval_secs: u64,
    },
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = config::Config::from_file(&cli.config)?;
    let pipeline = pipeline::Pipeline::new(config).await?;

    match cli.command {
        Command::Run => pipeline.run().await,
        Command::Watch { interval_secs } => pipeline.watch(interval_secs).await,
        Command::Status => pipeline.status().await,
    }
}
