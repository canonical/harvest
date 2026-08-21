use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::agent::AgentEvent;
use crate::artifacts::{bundle, handlers::{update_artifact, ArtifactKind}};
use crate::auth::jwt::Claims;
use crate::conversations::handlers::history_messages_from_raw;
use crate::deployments::{handlers::{err, redeploy_deployment_core, ApiError}, load_deployment_context};
use crate::issues::{self, IssueStatus};
use crate::neo4j::Neo4jClient;
use crate::projects::handlers::{require_project_access, ProjectState};

const DEFAULT_APPLY_TIMEOUT_SECS: u64 = 300;
const MAX_APPLY_TIMEOUT_SECS:     u64 = 1800;

fn default_apply_timeout() -> u64 { DEFAULT_APPLY_TIMEOUT_SECS }

fn require_agent_in_project(state: &ProjectState, agent_id: &str, project_id: &str) -> Result<(), ApiError> {
    let belongs = state.agent_builder.registry.agents.get(agent_id)
        .map(|a| a.project_id == project_id)
        .unwrap_or(false);
    if belongs { Ok(()) } else { Err(err(StatusCode::NOT_FOUND, "agent not found in this project")) }
}

#[derive(serde::Deserialize)]
pub struct ListIssuesParams {
    pub status:     Option<String>,
    pub deployment: Option<String>,
}

pub async fn list_issues(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Query(params): Query<ListIssuesParams>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let issue_list = issues::list_issues_for_project(
        &state.neo4j, &project_id, params.status.as_deref(), params.deployment.as_deref(),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(issue_list))
}

pub async fn get_issue(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, issue_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(issue))
}

#[derive(serde::Deserialize)]
pub struct UpdateIssueStatusBody {
    pub status: String,
}

pub async fn update_issue_status_route(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(body): Json<UpdateIssueStatusBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let requested = IssueStatus::parse(&body.status).ok_or_else(|| err(
        StatusCode::BAD_REQUEST,
        "status must be one of untriaged, in_progress, fixed, rejected",
    ))?;
    let outcome = issues::update_issue_status(&state.neo4j, &project_id, &issue_id, requested, &user.name)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    match outcome {
        issues::UpdateStatusOutcome::NotFound => Err(err(StatusCode::NOT_FOUND, "not found")),
        issues::UpdateStatusOutcome::Invalid(msg) => Err(err(StatusCode::BAD_REQUEST, &msg)),
        issues::UpdateStatusOutcome::Applied => {
            let issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
                .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
                .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
            Ok(Json(issue))
        }
    }
}

#[derive(serde::Deserialize)]
pub struct CreateIssueCommentBody {
    pub body: String,
}

pub async fn create_issue_comment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(body): Json<CreateIssueCommentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let text = body.body.trim().to_string();
    if text.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "body is required"));
    }
    issues::append_issue_comment(&state.neo4j, &project_id, &issue_id, "user", &user.name, &text)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(issue))
}

async fn linked_terraform_bundle(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> Result<(String, String, String), ApiError> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_TERRAFORM_BUNDLE]->(a:Artifact)
         RETURN a.id AS id, a.kind AS kind, a.title AS title",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no terraform bundle"))?;
    Ok((
        row["id"].as_str().unwrap_or_default().to_string(),
        row["kind"].as_str().unwrap_or_default().to_string(),
        row["title"].as_str().unwrap_or("Infrastructure").to_string(),
    ))
}

#[derive(serde::Deserialize)]
pub struct ApplyIssueSolutionBody {
    pub agent_id: String,
    #[serde(default = "default_apply_timeout")]
    pub timeout_secs: u64,
}

pub async fn apply_issue_solution(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(body): Json<ApplyIssueSolutionBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    require_agent_in_project(&state, &body.agent_id, &project_id)?;

    let issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    let proposed_files = issue["proposed_files"].clone();
    if proposed_files.is_null() {
        return Err(err(StatusCode::BAD_REQUEST, "issue has no proposed solution to apply"));
    }
    let summary = issue["proposed_solution_summary"].as_str().unwrap_or("").to_string();
    let deployment_id = issue["deployment"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .to_string();

    let files: BTreeMap<String, String> = serde_json::from_value(proposed_files)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    bundle::validate_bundle(&files).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;

    let (bundle_id, kind_str, title) = linked_terraform_bundle(&state.neo4j, &project_id, &deployment_id).await?;
    let kind = ArtifactKind::parse(&kind_str)
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let content = serde_json::to_string(&files)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    update_artifact(&state.neo4j, &bundle_id, kind, kind, &title, &content)
        .await.map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;
    issues::clear_proposed_solution_and_record_apply(&state.neo4j, &project_id, &issue_id, &summary)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let timeout = body.timeout_secs.min(MAX_APPLY_TIMEOUT_SECS);
    let redeploy = redeploy_deployment_core(&state, &project_id, &deployment_id, &body.agent_id, timeout).await?;

    let updated_issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(json!({ "issue": updated_issue, "redeploy": redeploy })))
}

#[derive(serde::Deserialize)]
pub struct RedeployFromIssueBody {
    pub agent_id: String,
    #[serde(default = "default_apply_timeout")]
    pub timeout_secs: u64,
}

pub async fn redeploy_from_issue(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(body): Json<RedeployFromIssueBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    require_agent_in_project(&state, &body.agent_id, &project_id)?;

    let issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    let deployment_id = issue["deployment"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .to_string();

    let timeout = body.timeout_secs.min(MAX_APPLY_TIMEOUT_SECS);
    let value = redeploy_deployment_core(&state, &project_id, &deployment_id, &body.agent_id, timeout).await?;
    Ok(Json(value))
}

fn spawn_issue_chat_relay(
    state:      &ProjectState,
    project_id: &str,
    issue_id:   &str,
) -> (tokio::sync::mpsc::Sender<AgentEvent>, tokio::sync::oneshot::Receiver<Vec<Value>>) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let (chain_tx, chain_rx) = tokio::sync::oneshot::channel::<Vec<Value>>();
    let channels    = Arc::clone(&state.channels);
    let project_id  = project_id.to_string();
    let issue_id    = issue_id.to_string();

    tokio::spawn(async move {
        let mut chain_builder = crate::agent::chain::ChainBuilder::new();
        while let Some(event) = rx.recv().await {
            match &event {
                AgentEvent::TextDelta { text } => chain_builder.text_delta(text),
                AgentEvent::Thinking { text } => chain_builder.thinking(text),
                AgentEvent::ToolCall { name, input } => chain_builder.tool_call(name, input, None, None),
                AgentEvent::ToolResult { name, preview } => chain_builder.tool_result(name, preview),
                _ => {}
            }
            let mut value = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
            value["issue_id"] = json!(issue_id);
            let msg = value.to_string();
            let map = channels.lock().await;
            if let Some(sender) = map.get(&project_id) {
                let _ = sender.send(msg);
            }
        }
        let _ = chain_tx.send(chain_builder.finish());
    });

    (tx, chain_rx)
}

#[derive(serde::Deserialize)]
pub struct IssueChatBody {
    pub message: String,
}

pub async fn issue_chat(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(body): Json<IssueChatBody>,
) -> Result<impl IntoResponse, ApiError> {
    let message = body.message.trim().to_string();
    if message.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "message is required"));
    }

    let project  = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default().to_string();

    let issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    let deployment_id = issue["deployment"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .to_string();
    let title = issue["title"].as_str().unwrap_or_default().to_string();
    let description = issue["description"].as_str().unwrap_or_default().to_string();
    let raw_history = issue["chat_messages"].as_array().cloned().unwrap_or_default();
    let history = history_messages_from_raw(&raw_history);

    let ctx = load_deployment_context(&state.neo4j, &project_id, &deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    let agent = state.agent_builder.build_for_issue_chat(
        project_id.clone(), group_id, &ctx, &issue_id, &title, &description,
    );

    let (progress_tx, chain_rx) = spawn_issue_chat_relay(&state, &project_id, &issue_id);
    let response = agent.query_with_progress(&message, &history, &[], None, progress_tx).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let chain = chain_rx.await.unwrap_or_default();

    issues::append_issue_chat_turn(
        &state.neo4j, &project_id, &issue_id, &message, &user.name,
        &response.answer, chain.clone(), response.tool_calls_made,
    ).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let proposed_solution = chain.iter()
        .any(|entry| entry["type"] == "tool_call" && entry["name"] == "propose_issue_solution");

    let updated_issue = issues::get_issue_detail(&state.neo4j, &project_id, &issue_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    Ok(Json(json!({
        "answer":            response.answer,
        "chain":             chain,
        "proposed_solution": proposed_solution,
        "issue":             updated_issue,
    })))
}
