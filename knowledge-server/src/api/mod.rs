pub mod query;
pub mod repositories;

use axum::{
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::agent::Agent;
use crate::neo4j::Neo4jClient;

#[derive(Clone)]
pub struct AppState {
    pub agent: Arc<Agent>,
    pub neo4j: Arc<Neo4jClient>,
}

pub fn router(state: AppState) -> Router {
    let agent_router = Router::new()
        .route("/query", post(query::handle_query))
        .route("/query/stream", post(query::handle_query_stream))
        .with_state(Arc::clone(&state.agent));

    let neo4j_router = Router::new()
        .route("/repositories", get(repositories::handle_list_repositories))
        .with_state(Arc::clone(&state.neo4j));

    Router::new()
        .merge(agent_router)
        .merge(neo4j_router)
        .route("/health", get(|| async { Json(serde_json::json!({ "status": "ok" })) }))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
