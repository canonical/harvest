use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser as ClapParser, Subcommand};

use knowledge_server::config::Config;

#[derive(ClapParser)]
#[command(name = "knowledge-server", about = "Query code knowledge graphs via HTTP")]
struct Cli {
    #[arg(short, long, default_value = "server.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Serve,
    Migrate,
    CheckConfig,
    SchemaVersion,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = Config::from_file(&cli.config)?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => knowledge_server::server::run(config).await,
        Command::Migrate => {
            let version = knowledge_server::server::migrate(&config).await?;
            println!("{version}");
            Ok(())
        }
        Command::CheckConfig => {
            println!("ok");
            Ok(())
        }
        Command::SchemaVersion => {
            let db = harvest_db::Db::connect_with_without_migrating(&config.database.options())?;
            println!("{} {}", db.schema_version().await?, harvest_db::LATEST_SCHEMA_VERSION);
            Ok(())
        }
    }
}
