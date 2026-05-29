use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::agent::Agent;

#[derive(Deserialize)]
pub struct ToolDescriptionRequest {
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Serialize)]
pub struct ToolDescriptionResponse {
    pub description: String,
}

pub async fn handle_tool_description(
    State(agent): State<Arc<Agent>>,
    Json(req): Json<ToolDescriptionRequest>,
) -> impl IntoResponse {
    if req.name.is_empty() {
        return (StatusCode::UNPROCESSABLE_ENTITY, "name is required").into_response();
    }
    let description = agent.describe_tool_call(&req.name, &req.input).await;
    Json(ToolDescriptionResponse { description }).into_response()
}
