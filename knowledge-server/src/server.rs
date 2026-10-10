use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

use crate::agent::{graph_tools, Agent};
use crate::api::{AppState, GraphCache, ProjectAgentBuilder};
use crate::auth::user_keys::UserKeyStore;
use crate::cluster::bus::ClusterBus;
use crate::cluster::node::ClusterNode;
use crate::cluster::peer::{require_secret, PeerClient};
use crate::cluster::{singleton, Shutdown};
use crate::config::Config;
use crate::crypto::Crypto;
use crate::machines::directory::AgentDirectory;
use crate::machines::MachineRegistry;
use crate::projects::live::ProjectLive;
use crate::skills::SkillStore;
use harvest_db::Db;

const SCHEMA_WAIT_INTERVAL: Duration = Duration::from_secs(2);
const SERVE_GRACE: Duration = Duration::from_secs(5);

pub struct RunningServer {
    pub public_addr:   SocketAddr,
    pub internal_addr: Option<SocketAddr>,
    pub node:          Arc<ClusterNode>,
    pub live:          Arc<ProjectLive>,
    drain:             Option<tokio::sync::oneshot::Sender<()>>,
    finished:          tokio::task::JoinHandle<()>,
}

impl RunningServer {
    pub fn request_drain(&mut self) {
        if let Some(drain) = self.drain.take() {
            let _ = drain.send(());
        }
    }

    pub async fn drain_and_wait(mut self) {
        self.request_drain();
        let _ = self.finished.await;
    }

    pub async fn wait(self) {
        let _ = self.finished.await;
    }
}

pub async fn connect_database(config: &Config) -> Result<Arc<Db>> {
    let options = config.database.options();
    if config.database.migrate_on_start {
        return Ok(Arc::new(Db::connect_with(&options).await?));
    }
    let db = Db::connect_with_without_migrating(&options)?;
    loop {
        match db.schema_version().await {
            Ok(version) if version >= harvest_db::LATEST_SCHEMA_VERSION => return Ok(Arc::new(db)),
            Ok(version) => tracing::info!(version, wanted = harvest_db::LATEST_SCHEMA_VERSION, "waiting for database migrations"),
            Err(e) => tracing::warn!(error = %e, "waiting for the database"),
        }
        tokio::time::sleep(SCHEMA_WAIT_INTERVAL).await;
    }
}

pub async fn migrate(config: &Config) -> Result<i32> {
    let db = Db::connect_with_without_migrating(&config.database.options())?;
    db.migrate().await?;
    db.schema_version().await
}

async fn turn_snapshot(State(live): State<Arc<ProjectLive>>, Path(conv_id): Path<String>) -> Response {
    match live.local_snapshot(&conv_id).await {
        Some(snapshot) => Json(snapshot).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn internal_router(live: Arc<ProjectLive>, registry: Arc<MachineRegistry>, secret: Arc<String>) -> Router {
    Router::new()
        .route("/internal/ping", get(|| async { Json(json!({ "ok": true })) }))
        .route("/internal/turns/:conv_id/snapshot", get(turn_snapshot).with_state(live))
        .merge(crate::machines::internal::router(registry))
        .layer(axum::middleware::from_fn_with_state(secret, require_secret))
}

async fn reap(db: &Db, live: &ProjectLive, directory: &AgentDirectory) {
    if let Err(e) = live.reap_dead_nodes().await {
        tracing::warn!(error = %e, "reaping turns and presence of lost nodes failed");
    }
    if let Err(e) = directory.reap_dead_nodes().await {
        tracing::warn!(error = %e, "reaping agent connections of lost nodes failed");
    }
    if let Err(e) = crate::ingestion::reap_dead_nodes(db, live.node()).await {
        tracing::warn!(error = %e, "reaping ingestion jobs of lost nodes failed");
    }
    let _ = crate::auth::ephemeral::purge_expired(db).await;
    let _ = db.execute("DELETE FROM bus_payloads WHERE created_at < now() - interval '10 minutes'", json!({})).await;
    let _ = db.execute("DELETE FROM cluster_nodes WHERE heartbeat_at < now() - interval '1 hour'", json!({})).await;
}

fn spawn_reaper(
    db: Arc<Db>,
    live: Arc<ProjectLive>,
    directory: Arc<AgentDirectory>,
    collocate: Arc<crate::collocate::sessions::SessionContainerRegistry>,
    interval: Duration,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let result = singleton::run_exclusive(&db, "reaper", || async {
                reap(&db, &live, &directory).await;
                collocate.reap_dead_nodes(live.node().timeout_secs()).await;
            }).await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "reaper could not run");
            }
        }
    });
}

fn spawn_graph_cache_invalidation(bus: Arc<ClusterBus>, cache: Arc<GraphCache>) {
    let mut receiver = bus.subscribe(crate::api::repositories::GRAPH_TOPIC);
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(prefix) => crate::api::repositories::drop_cached_graphs(&cache, &prefix).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => cache.write().await.clear(),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
}

async fn drain(
    node: Arc<ClusterNode>,
    live: Arc<ProjectLive>,
    directory: Arc<AgentDirectory>,
    shutdown: Shutdown,
    grace: Duration,
    timeout: Duration,
) {
    tracing::info!(node_id = node.node_id(), "draining: refusing new turns and failing readiness");
    if let Err(e) = node.set_draining().await {
        tracing::warn!(error = %e, "could not record draining state");
    }
    tokio::time::sleep(grace).await;
    shutdown.trigger();
    let deadline = tokio::time::Instant::now() + timeout;
    while live.local_turn_count().await > 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let aborted = live.abort_local_turns().await;
    if aborted > 0 {
        tracing::warn!(aborted, "aborted turns that did not finish within the drain timeout");
    }
    let _ = live.release_presence().await;
    let _ = directory.release_node().await;
    let _ = node.deregister().await;
    tracing::info!(node_id = node.node_id(), "drained");
}

async fn bind(address: &str) -> Result<TcpListener> {
    TcpListener::bind(address).await.with_context(|| format!("binding {address}"))
}

pub async fn start(config: Config) -> Result<RunningServer> {
    config.validate()?;
    let db = connect_database(&config).await?;

    crate::skills::seed_defaults_if_needed(&db).await?;

    if config.llm.is_empty() {
        anyhow::bail!("at least one [[llm]] provider must be configured in server.toml");
    }

    let public_listener = bind(&format!("{}:{}", config.server.host, config.server.port)).await?;
    let public_addr = public_listener.local_addr()?;
    let internal_listener = match &config.cluster.internal_listen {
        Some(address) => Some(bind(address).await?),
        None => None,
    };
    let internal_addr = internal_listener.as_ref().map(|l| l.local_addr()).transpose()?;

    let mut cluster_config = config.cluster.clone();
    if let Some(addr) = internal_addr {
        cluster_config.internal_url = Some(cluster_config.advertised_url(addr)?);
    }

    let node = ClusterNode::new(Arc::clone(&db), &cluster_config);
    node.heartbeat().await.context("registering this node in the cluster")?;
    node.spawn_heartbeat();
    tracing::info!(node_id = node.node_id(), internal_url = ?node.internal_url(), "cluster node registered");

    let shutdown = Shutdown::new();
    let bus = ClusterBus::start(
        Arc::clone(&db),
        node.node_id().to_string(),
        Duration::from_millis(cluster_config.bus_batch_window_ms),
    );
    let peers = match (&cluster_config.shared_secret, internal_addr) {
        (Some(secret), Some(_)) => Some(PeerClient::new(node.node_id(), secret.clone())),
        _ => None,
    };
    let live = ProjectLive::new(Arc::clone(&db), Arc::clone(&node), Arc::clone(&bus), peers.clone(), shutdown.clone());

    let has_user_key_providers = config.llm.iter().any(|c| c.user_provided_key());
    let user_key_store = if has_user_key_providers {
        let key_hex = config.security.user_key_encryption_key.as_ref().context(
            "security.user_key_encryption_key must be configured when any provider has user_provided_key = true.\n\
             Generate one with: openssl rand -hex 32",
        )?;
        let crypto = Crypto::from_hex(key_hex).context("invalid user_key_encryption_key")?;
        Some(Arc::new(UserKeyStore::new(Arc::clone(&db), Arc::new(crypto))))
    } else {
        None
    };

    let llm_provider               = crate::llm::from_config(&config.llm);
    let llm_configs                = Arc::new(config.llm.clone());
    let max_iterations             = config.agent.max_iterations;
    let compaction_threshold_chars = config.agent.compaction_threshold_chars;
    let compaction_keep_last       = config.agent.compaction_keep_last;

    let system_one_client = match &config.system_one {
        Some(so_cfg) if so_cfg.is_enabled() && !so_cfg.api_key.is_empty() => {
            Some(crate::llm::system_one::SystemOneClient::from_config(so_cfg))
        }
        _ => None,
    };

    let semantic_handle = match crate::agent::semantic::build_handle(&config.semantic, &config.llm) {
        Ok(handle) => handle,
        Err(err) => {
            tracing::error!(error = %err, "semantic search is configured but could not be initialised; continuing with lexical search only");
            None
        }
    };
    if let Some(handle) = semantic_handle.clone() {
        if config.semantic.backfill_on_start {
            if let Some(embedder) = handle.embedder.clone() {
                let db = Arc::clone(&db);
                let semantic_cfg = config.semantic.clone();
                tokio::spawn(async move {
                    let ran = singleton::run_exclusive(&db, "semantic-backfill", || async {
                        match crate::agent::semantic::run_backfill_if_configured(db.as_ref(), embedder.as_ref(), &semantic_cfg).await {
                            Ok(stats) => tracing::info!(scanned = stats.scanned, embedded = stats.embedded, "semantic backfill finished"),
                            Err(err) => tracing::warn!(error = %err, "semantic backfill failed"),
                        }
                    }).await;
                    if matches!(ran, Ok(false)) {
                        tracing::info!("semantic backfill already running on another node");
                    }
                });
            }
        }
    }

    graph_tools::set_run_sql_available(graph_tools::probe_run_sql(&db).await);
    let global_tools = graph_tools::all_tools_with_semantic(Arc::clone(&db), semantic_handle.clone());
    let thresholds = crate::api::Thresholds::from_system_one(config.system_one.as_ref());

    let agent = Arc::new(thresholds.apply(
        Agent::new(Arc::clone(&llm_provider), global_tools, max_iterations)
            .with_system_one(system_one_client.clone())
            .with_compaction(compaction_threshold_chars, compaction_keep_last)
            .with_parallel_research(true),
    ));

    let machine_registry = MachineRegistry::new();
    let directory = AgentDirectory::new(Arc::clone(&db), Arc::clone(&node), peers.clone());
    machine_registry.attach_directory(Arc::clone(&directory));
    machine_registry.attach_shutdown(shutdown.clone());
    let skill_registry = Arc::new(SkillStore::new(Arc::clone(&db)));

    let docs_dir    = config.documentation.docs_dir.clone().map(Arc::new);
    let server_url  = config.agents.public_url
        .clone()
        .or_else(|| config.auth.public_url.clone())
        .unwrap_or_else(|| format!("http://{}:{}", config.server.host, public_addr.port()));
    let binary_path = config.agents.binary_path.clone();
    let lxd = match &config.lxd {
        Some(cfg) => crate::lxd::resolve_client(cfg, &db).await?.map(Arc::new),
        None => None,
    };

    let collocate_registry = crate::collocate::sessions::SessionContainerRegistry::new();
    let collocate = match &config.collocate {
        Some(cfg) => match crate::collocate::client::CollocateHandle::connect(cfg.clone()) {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::warn!(error = %e, "failed to connect collocate client; collocate tools disabled");
                None
            }
        },
        None => None,
    };
    collocate_registry.attach_store(Arc::clone(&db), node.node_id().to_string(), collocate.clone().map(Arc::new));

    let agent_builder = Arc::new(ProjectAgentBuilder {
        llm:            Arc::clone(&llm_provider),
        system_one:     system_one_client.clone(),
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
        thresholds: Some(thresholds),
        semantic: semantic_handle.clone(),
    });

    let pricing = Arc::new(crate::cost::PricingTable::from_configs(&config.llm));
    let cluster_timeout = node.node_timeout();
    let reap_interval = node.heartbeat_interval().max(Duration::from_millis(200)) * 2;
    let drain_grace = Duration::from_secs(cluster_config.drain_grace_secs);
    let drain_timeout = Duration::from_secs(cluster_config.drain_timeout_secs);
    let secret = Arc::new(cluster_config.shared_secret.clone().unwrap_or_default());

    let collocate_reaper = Arc::clone(&collocate_registry);
    let state = AppState {
        agent,
        db:               Arc::clone(&db),
        ingestion:        crate::ingestion::IngestionJobs::new(Arc::clone(&db), Arc::clone(&node)),
        docs_dir,
        auth:             Arc::new(config.auth),
        ui:               Arc::new(config.ui),
        machine_registry: Arc::clone(&machine_registry),
        agent_builder:    Arc::clone(&agent_builder),
        binary_path,
        llm:              Arc::clone(&llm_provider),
        llm_configs,
        system_one_config: config.system_one.clone(),
        user_key_store,
        lxd,
        collocate_registry,
        pricing,
        semantic: semantic_handle,
        live: Arc::clone(&live),
    };

    let cache: Arc<GraphCache> = Arc::new(RwLock::new(HashMap::new()));
    tokio::spawn({
        let db = Arc::clone(&db);
        let cache = Arc::clone(&cache);
        async move { crate::api::graph::warm_graph_cache(db, cache).await; }
    });
    spawn_graph_cache_invalidation(Arc::clone(&bus), Arc::clone(&cache));
    spawn_reaper(Arc::clone(&db), Arc::clone(&live), Arc::clone(&directory), Arc::clone(&collocate_reaper), reap_interval);

    let app = crate::api::router(state, cache, server_url).await;
    tracing::info!(%public_addr, ?internal_addr, timeout_ms = cluster_timeout.as_millis() as u64, "listening");

    let public = tokio::spawn({
        let app = app.clone();
        let shutdown = shutdown.clone();
        async move {
            let _ = axum::serve(public_listener, app.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(shutdown.wait())
                .await;
        }
    });
    let internal = internal_listener.map(|listener| {
        let app = app.clone().merge(internal_router(Arc::clone(&live), Arc::clone(&machine_registry), secret));
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                .with_graceful_shutdown(shutdown.wait())
                .await;
        })
    });

    let (drain_tx, drain_rx) = tokio::sync::oneshot::channel::<()>();
    let finished = tokio::spawn({
        let node = Arc::clone(&node);
        let live = Arc::clone(&live);
        async move {
            let _ = drain_rx.await;
            drain(Arc::clone(&node), Arc::clone(&live), directory, shutdown, drain_grace, drain_timeout).await;
            let _ = tokio::time::timeout(SERVE_GRACE, public).await;
            if let Some(internal) = internal {
                let _ = tokio::time::timeout(SERVE_GRACE, internal).await;
            }
        }
    });

    Ok(RunningServer { public_addr, internal_addr, node, live, drain: Some(drain_tx), finished })
}

pub async fn wait_for_termination() {
    #[cfg(unix)]
    {
        let mut terminate = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub async fn run(config: Config) -> Result<()> {
    let mut server = start(config).await?;
    wait_for_termination().await;
    server.request_drain();
    server.wait().await;
    Ok(())
}
