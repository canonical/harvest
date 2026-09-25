use anyhow::{Context, Result};
use clap::Parser as ClapParser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use knowledge_server::agent::{graph_tools, Agent};
use knowledge_server::api::{AppState, GraphCache, ProjectAgentBuilder};
use knowledge_server::ingestion;
use knowledge_server::skills::SkillStore;
use knowledge_server::auth::user_keys::UserKeyStore;
use knowledge_server::config::Config;
use knowledge_server::crypto::Crypto;
use knowledge_server::llm;
use knowledge_server::lxd;
use knowledge_server::machines::MachineRegistry;
use harvest_db::Db;

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

    let db = Arc::new(Db::connect(&config.database.url).await?);

    knowledge_server::skills::seed_defaults_if_needed(&db).await?;

    if config.llm.is_empty() {
        anyhow::bail!("at least one [[llm]] provider must be configured in server.toml");
    }

    let has_user_key_providers = config.llm.iter().any(|c| c.user_provided_key());
    let user_key_store = if has_user_key_providers {
        let crypto = match &config.security.user_key_encryption_key {
            Some(key_hex) => {
                Crypto::from_hex(key_hex).context("invalid user_key_encryption_key")?
            }
            None => {
                anyhow::bail!(
                    "security.user_key_encryption_key must be configured when any provider has user_provided_key = true.\n\
                     Generate one with: openssl rand -hex 32"
                );
            }
        };
        Some(Arc::new(UserKeyStore::new(Arc::clone(&db), Arc::new(crypto))))
    } else {
        None
    };

    let llm_provider                = llm::from_config(&config.llm);
    let llm_configs                 = Arc::new(config.llm.clone());
    let max_iterations              = config.agent.max_iterations;
    let compaction_threshold_chars  = config.agent.compaction_threshold_chars;
    let compaction_keep_last        = config.agent.compaction_keep_last;

    let global_tools = graph_tools::all_tools(Arc::clone(&db));
    let agent = Arc::new(
        Agent::new(Arc::clone(&llm_provider), global_tools, max_iterations)
            .with_compaction(compaction_threshold_chars, compaction_keep_last)
            .with_parallel_research(true),
    );

    let machine_registry = MachineRegistry::new();
    let skill_registry   = Arc::new(SkillStore::new(Arc::clone(&db)));

    let docs_dir    = config.documentation.docs_dir.map(Arc::new);
    let server_url  = config.agents.public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}", config.server.host, config.server.port));
    let binary_path = config.agents.binary_path.clone();
    let lxd = match &config.lxd {
        Some(cfg) => lxd::resolve_client(cfg, &db).await?.map(Arc::new),
        None => None,
    };

    let collocate_registry = knowledge_server::collocate::sessions::SessionContainerRegistry::new();
    let collocate = match &config.collocate {
        Some(cfg) => match knowledge_server::collocate::client::CollocateHandle::connect(cfg.clone()) {
            Ok(handle) => {
                tracing::info!("collocate client connected to {}", cfg.endpoint);
                Some(handle)
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to connect collocate client; collocate tools disabled");
                None
            }
        }
        None => None,
    };

    let agent_builder = Arc::new(ProjectAgentBuilder {
        llm:            Arc::clone(&llm_provider),
        db:             Arc::clone(&db),
        registry:       Arc::clone(&machine_registry),
        skills:         Arc::clone(&skill_registry),
        lxd:            lxd.clone(),
        collocate,
        collocate_registry: Arc::clone(&collocate_registry),
        server_url:     server_url.clone(),
        max_iterations,
        compaction_threshold_chars,
        compaction_keep_last,
    });

    let pricing = Arc::new(knowledge_server::cost::PricingTable::from_configs(&config.llm));

    let state = AppState {
        agent,
        db:               Arc::clone(&db),
        ingestion:        ingestion::new_registry(),
        docs_dir,
        auth:             Arc::new(config.auth),
        ui:               Arc::new(config.ui),
        machine_registry: Arc::clone(&machine_registry),
        agent_builder:    Arc::clone(&agent_builder),
        binary_path,
        llm:              Arc::clone(&llm_provider),
        llm_configs,
        user_key_store,
        lxd,
        collocate_registry,
        pricing,
    };

    let cache: Arc<GraphCache> = Arc::new(RwLock::new(HashMap::new()));
    tokio::spawn({
        let db = Arc::clone(&db);
        let cache = Arc::clone(&cache);
        async move { knowledge_server::api::graph::warm_graph_cache(db, cache).await; }
    });

    let app  = knowledge_server::api::router(state, cache, server_url).await;
    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
