use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::auth::jwt::Claims;
use crate::cost::{
    conversation_cost_by_turn, conversation_cost_summary, deployment_cost_summary,
    deployment_llm_calls, project_cost_by_scope, project_cost_summary,
    CostSummary,
};
use crate::projects::handlers::{require_project_access, ProjectState};

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

fn summary_json(summary: &CostSummary) -> Value {
    json!({
        "total_cost_microusd": summary.total_microusd,
        "total_cost_display": summary.formatted_cost(),
        "total_calls": summary.total_calls,
        "total_input_tokens": summary.total_input_tokens,
        "total_output_tokens": summary.total_output_tokens,
        "total_cache_read_tokens": summary.total_cache_read_tokens,
        "total_cache_creation_tokens": summary.total_cache_creation_tokens,
        "total_reasoning_tokens": summary.total_reasoning_tokens,
    })
}

pub async fn project_cost(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let total = project_cost_summary(&state.neo4j, &project_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let by_scope = project_cost_by_scope(&state.neo4j, &project_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let scopes = by_scope.iter().map(|(s, c)| (s.clone(), summary_json(c))).collect::<serde_json::Map<_, _>>();
    Ok(Json(json!({
        "project_id": project_id,
        "total": summary_json(&total),
        "by_scope": scopes,
    })))
}

pub async fn project_cost_by_model(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = crate::cost::project_cost_by_model(&state.neo4j, &project_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let models: Vec<Value> = rows.iter().map(|(kind, model, summary)| json!({
        "kind": kind, "model": model, "summary": summary_json(summary),
    })).collect();
    Ok(Json(json!({ "project_id": project_id, "by_model": models })))
}

pub async fn project_cost_by_user(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = crate::cost::project_cost_by_user(&state.neo4j, &project_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let users: Vec<Value> = rows.iter().map(|(uid, summary)| json!({
        "user_id": uid, "summary": summary_json(summary),
    })).collect();
    Ok(Json(json!({ "project_id": project_id, "by_user": users })))
}

pub async fn conversation_cost(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, conversation_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let total = conversation_cost_summary(&state.neo4j, &conversation_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let turns = conversation_cost_by_turn(&state.neo4j, &conversation_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let turns_json: Vec<Value> = turns.iter().map(|(tid, summary)| json!({
        "turn_id": tid, "summary": summary_json(summary),
    })).collect();
    Ok(Json(json!({
        "project_id": project_id,
        "conversation_id": conversation_id,
        "total": summary_json(&total),
        "turns": turns_json,
    })))
}

pub async fn deployment_cost(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let by_scope = deployment_cost_summary(&state.neo4j, &deployment_id).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let total = by_scope.values().fold(CostSummary::default(), |mut acc, s| { acc.add(s); acc });
    let scopes = by_scope.iter().map(|(s, c)| (s.clone(), summary_json(c))).collect::<serde_json::Map<_, _>>();
    Ok(Json(json!({
        "project_id": project_id,
        "deployment_id": deployment_id,
        "total": summary_json(&total),
        "by_scope": scopes,
    })))
}

pub async fn deployment_cost_calls(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let calls = deployment_llm_calls(&state.neo4j, &deployment_id, 200).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    Ok(Json(json!({
        "project_id": project_id,
        "deployment_id": deployment_id,
        "calls": calls,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_json_has_cost_and_tokens() {
        let summary = CostSummary {
            total_microusd: 42100,
            total_calls: 3,
            total_input_tokens: 1000,
            total_output_tokens: 500,
            total_cache_read_tokens: 200,
            total_cache_creation_tokens: 0,
            total_reasoning_tokens: 0,
        };
        let v = summary_json(&summary);
        assert_eq!(v["total_cost_microusd"], 42100);
        assert_eq!(v["total_cost_display"], "$0.042100");
        assert_eq!(v["total_calls"], 3);
        assert_eq!(v["total_input_tokens"], 1000);
        assert_eq!(v["total_output_tokens"], 500);
    }

    #[test]
    fn err_returns_json_error() {
        let (status, body) = err(StatusCode::NOT_FOUND, "missing");
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "missing");
    }
}
