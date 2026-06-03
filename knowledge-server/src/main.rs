use anyhow::Result;
use clap::Parser as ClapParser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use knowledge_server::agent::{graph_tools, Agent};
use knowledge_server::api::{AppState, GraphCache};
use knowledge_server::auth;
use knowledge_server::config::Config;
use knowledge_server::llm;
use knowledge_server::neo4j::Neo4jClient;

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

    auth::setup_constraints(&neo4j).await?;
    neo4j.run("CREATE CONSTRAINT conversation_id IF NOT EXISTS FOR (c:Conversation) REQUIRE c.id IS UNIQUE").await?;

    let llm_provider = llm::from_config(&config.llm);
    let max_iterations = config.llm.max_iterations();
    let tools = graph_tools::all_tools(Arc::clone(&neo4j));
    let agent = Arc::new(Agent::new(llm_provider, tools, max_iterations));

    let docs_dir = config.documentation.docs_dir.map(Arc::new);
    let state = AppState {
        agent,
        neo4j: Arc::clone(&neo4j),
        docs_dir,
        auth: Arc::new(config.auth),
    };

    let cache: Arc<GraphCache> = Arc::new(RwLock::new(HashMap::new()));
    tokio::spawn({
        let neo4j  = Arc::clone(&neo4j);
        let cache  = Arc::clone(&cache);
        async move { knowledge_server::api::graph::warm_graph_cache(neo4j, cache).await; }
    });

    let app = knowledge_server::api::router(state, cache);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
