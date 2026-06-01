use knowledge_harvester::{config, documentation, pipeline};

use anyhow::{bail, Result};
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
    Run {
        #[arg(short, long, help = "Re-ingest versions that were already processed")]
        force: bool,
    },
    #[command(about = "Continuously poll repositories and ingest new versions on a fixed interval")]
    Watch {
        #[arg(short, long, default_value = "300", help = "Polling interval in seconds")]
        interval_secs: u64,
    },
    #[command(about = "Mark all ingested versions as pending so the next run re-processes them")]
    Reingest,
    Status,
    #[command(about = "Generate Diataxis documentation for a repository version")]
    Document {
        #[arg(value_name = "REPOSITORY:VERSION", help = "e.g. my-repo:v1.0")]
        repository_version: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = config::Config::from_file(&cli.config)?;

    match cli.command {
        Command::Document { repository_version } => {
            let (repo, version) = parse_repo_version(&repository_version)?;
            let llm_config = config.llm.as_ref().ok_or_else(|| {
                anyhow::anyhow!("document command requires [llm] configuration in harvester.toml")
            })?;
            let doc_config = config.documentation.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "document command requires [documentation] configuration in harvester.toml"
                )
            })?;
            let pipeline =
                documentation::DocumentationPipeline::new(
                    &config.neo4j.uri,
                    &config.neo4j.user,
                    &config.neo4j.password,
                    llm_config,
                    doc_config,
                )
                .await?;
            pipeline.document(repo, version).await
        }
        _ => {
            let pipeline = pipeline::Pipeline::new(config).await?;
            match cli.command {
                Command::Run { force } => pipeline.run(force).await,
                Command::Watch { interval_secs } => pipeline.watch(interval_secs).await,
                Command::Reingest => pipeline.reingest().await,
                Command::Status => pipeline.status().await,
                Command::Document { .. } => unreachable!(),
            }
        }
    }
}

fn parse_repo_version(s: &str) -> Result<(&str, &str)> {
    match s.split_once(':') {
        Some((repo, version)) if !repo.is_empty() && !version.is_empty() => Ok((repo, version)),
        _ => bail!("expected REPOSITORY:VERSION (e.g. my-repo:v1.0), got: {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repo_version_valid() {
        let (repo, ver) = parse_repo_version("my-repo:v1.0").unwrap();
        assert_eq!(repo, "my-repo");
        assert_eq!(ver, "v1.0");
    }

    #[test]
    fn parse_repo_version_no_colon_errors() {
        assert!(parse_repo_version("my-repo-only").is_err());
    }

    #[test]
    fn parse_repo_version_empty_repo_errors() {
        assert!(parse_repo_version(":v1.0").is_err());
    }

    #[test]
    fn parse_repo_version_empty_version_errors() {
        assert!(parse_repo_version("my-repo:").is_err());
    }

    #[test]
    fn parse_repo_version_with_slash_in_version() {
        let (repo, ver) = parse_repo_version("my-repo:stable/2023.1").unwrap();
        assert_eq!(repo, "my-repo");
        assert_eq!(ver, "stable/2023.1");
    }
}
