use anyhow::Result;
use clap::Parser as ClapParser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use knowledge_server::agent::{graph_tools, Agent};
use knowledge_server::api::{AppState, GraphCache, ProjectAgentBuilder};
use knowledge_server::skills::SkillRegistry;
use knowledge_server::auth;
use knowledge_server::config::Config;
use knowledge_server::llm;
use knowledge_server::machines::MachineRegistry;
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

    let cli    = Cli::parse();
    let config = Config::from_file(&cli.config)?;

    let neo4j = Arc::new(
        Neo4jClient::new(&config.neo4j.uri, &config.neo4j.user, &config.neo4j.password).await?,
    );

    auth::setup_constraints(&neo4j).await?;
    neo4j.run("CREATE CONSTRAINT conversation_id IF NOT EXISTS FOR (c:Conversation) REQUIRE c.id IS UNIQUE").await?;
    neo4j.run("CREATE CONSTRAINT project_id    IF NOT EXISTS FOR (p:Project)      REQUIRE p.id IS UNIQUE").await?;
    neo4j.run("CREATE CONSTRAINT machine_id    IF NOT EXISTS FOR (m:Machine)      REQUIRE m.id IS UNIQUE").await?;
    neo4j.run("CREATE CONSTRAINT memory_id     IF NOT EXISTS FOR (m:Memory)       REQUIRE m.id IS UNIQUE").await?;

    if config.llm.is_empty() {
        anyhow::bail!("at least one [[llm]] provider must be configured in server.toml");
    }
    let llm_provider                = llm::from_config(&config.llm);
    let max_iterations              = config.agent.max_iterations;
    let compaction_threshold_chars  = config.agent.compaction_threshold_chars;
    let compaction_keep_last        = config.agent.compaction_keep_last;

    let global_tools = graph_tools::all_tools(Arc::clone(&neo4j));
    let agent = Arc::new(
        Agent::new(Arc::clone(&llm_provider), global_tools, max_iterations)
            .with_compaction(compaction_threshold_chars, compaction_keep_last),
    );

    let machine_registry = MachineRegistry::new();
    let skill_registry   = Arc::new(SkillRegistry::new());

    let agent_builder = Arc::new(ProjectAgentBuilder {
        llm:            Arc::clone(&llm_provider),
        neo4j:          Arc::clone(&neo4j),
        registry:       Arc::clone(&machine_registry),
        skills:         Arc::clone(&skill_registry),
        max_iterations,
        compaction_threshold_chars,
        compaction_keep_last,
    });

    let docs_dir    = config.documentation.docs_dir.map(Arc::new);
    let server_url  = config.agents.public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}", config.server.host, config.server.port));
    let binary_path = config.agents.binary_path.clone();

    let state = AppState {
        agent,
        neo4j:            Arc::clone(&neo4j),
        docs_dir,
        auth:             Arc::new(config.auth),
        ui:               Arc::new(config.ui),
        machine_registry: Arc::clone(&machine_registry),
        agent_builder:    Arc::clone(&agent_builder),
        binary_path,
        llm:              Arc::clone(&llm_provider),
    };

    let cache: Arc<GraphCache> = Arc::new(RwLock::new(HashMap::new()));
    tokio::spawn({
        let neo4j = Arc::clone(&neo4j);
        let cache = Arc::clone(&cache);
        async move { knowledge_server::api::graph::warm_graph_cache(neo4j, cache).await; }
    });

    let app  = knowledge_server::api::router(state, cache, server_url).await;
    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
