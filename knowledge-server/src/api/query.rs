use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::agent::Agent;

#[derive(Deserialize)]
pub struct QueryRequest {
    pub query: String,
    pub repositories: Option<Vec<String>>,
    pub versions: Option<Vec<String>>,
}

pub async fn handle_query(
    State(agent): State<Arc<Agent>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    match agent.query(&req.query).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
