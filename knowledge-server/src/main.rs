use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser as ClapParser, Subcommand};

use knowledge_server::config::Config;

#[derive(ClapParser)]
#[command(
    name = "knowledge-server",
    about = "Query code knowledge graphs via HTTP"
)]
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
    RecoverCluster,
    IssueClusterCertificate(IssueCertificateArgs),
}

#[derive(clap::Args)]
struct IssueCertificateArgs {
    #[arg(long)]
    dir: PathBuf,
    #[arg(long)]
    name: String,
    #[arg(long = "host", required = true)]
    hosts: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Serve);
    if let Command::IssueClusterCertificate(args) = &command {
        let issued = knowledge_server::cluster::tls::issue(&args.dir, &args.name, &args.hosts)?;
        println!("ca_cert = \"{}\"", issued.ca_cert.display());
        println!("cert = \"{}\"", issued.cert.display());
        println!("key = \"{}\"", issued.key.display());
        return Ok(());
    }
    let config = Config::from_file(&cli.config)?;

    match command {
        Command::IssueClusterCertificate(_) => Ok(()),
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
        Command::RecoverCluster => {
            let backup = knowledge_server::cluster::quorium::recover(&config.cluster).await?;
            println!("moved the quorium state to {}", backup.display());
            println!("start this server with cluster.bootstrap = \"init\", then join the other servers with new tokens");
            Ok(())
        }
        Command::SchemaVersion => {
            let db = harvest_db::Db::connect_with_without_migrating(&config.database.options())?;
            println!(
                "{} {}",
                db.schema_version().await?,
                harvest_db::LATEST_SCHEMA_VERSION
            );
            Ok(())
        }
    }
}
