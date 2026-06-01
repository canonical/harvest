pub mod docs;
pub mod graph;
pub mod query;
pub mod repositories;
pub mod tool_description;

use axum::{
    routing::{get, post},
    Json, Router,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::agent::Agent;
use crate::neo4j::Neo4jClient;

/// Serialised JSON per `"repo:version"` key.  Never expires during a server
/// run; the server must be restarted after reingestion to pick up fresh data.
pub type GraphCache = RwLock<HashMap<String, Arc<String>>>;

/// State shared by the graph and repository HTTP handlers.
#[derive(Clone)]
pub struct GraphState {
    pub neo4j: Arc<Neo4jClient>,
    pub cache: Arc<GraphCache>,
}

/// State shared by the LLM-agent HTTP handlers.
#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub neo4j: Arc<Neo4jClient>,
    pub docs_dir: Option<Arc<PathBuf>>,
}

pub fn router(state: AppState, cache: Arc<GraphCache>) -> Router {
    let graph_state = Arc::new(GraphState {
        neo4j: Arc::clone(&state.neo4j),
        cache,
    });

    let agent_router = Router::new()
        .route("/query", post(query::handle_query))
        .route("/query/stream", post(query::handle_query_stream))
        .route("/tool-description", post(tool_description::handle_tool_description))
        .with_state(Arc::clone(&state.agent));

    let graph_router = Router::new()
        .route("/repositories", get(repositories::handle_list_repositories))
        .route("/graph/:repo/:version", get(graph::handle_get_graph))
        .route("/graph/:repo/:version/source", get(graph::handle_get_symbol_source))
        .with_state(Arc::clone(&graph_state));

    let mut router = Router::new()
        .merge(agent_router)
        .merge(graph_router)
        .route("/health", get(|| async { Json(serde_json::json!({ "status": "ok" })) }));

    if let Some(docs_dir) = state.docs_dir {
        let docs_router = Router::new()
            .route("/docs/:repo/:version", get(docs::handle_get_index))
            .route("/docs/:repo/:version/:section/*filename", get(docs::handle_get_page))
            .with_state(docs_dir);
        router = router.merge(docs_router);
    }

    router
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
