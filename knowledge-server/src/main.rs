mod agent;
mod api;
mod config;
mod llm;
mod neo4j;

use anyhow::Result;
use clap::Parser as ClapParser;
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent::Agent;
use crate::api::AppState;
use crate::config::Config;
use crate::neo4j::Neo4jClient;

#[derive(ClapParser)]
#[command(name = "knowledge-server", about = "Query code knowledge graphs via HTTP")]
struct Cli {
    #[arg(short, long, default_value = "server.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let config = Config::from_file(&cli.config)?;

    let neo4j = Arc::new(
        Neo4jClient::new(&config.neo4j.uri, &config.neo4j.user, &config.neo4j.password).await?,
    );

    let llm_provider = llm::from_config(&config.llm);
    let max_iterations = config.llm.max_iterations();
    let tools = agent::graph_tools::all_tools(Arc::clone(&neo4j));
    let agent = Arc::new(Agent::new(llm_provider, tools, max_iterations));

    let state = AppState { agent, neo4j };
    let app = api::router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
