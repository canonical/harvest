use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::agent::{Agent, AgentEvent};
use crate::artifacts::{bundle, handlers::{get_artifact_in_project, ArtifactKind}};
use crate::auth::jwt::Claims;
use crate::machines::{TerraformAction, TerraformFlavor};
use crate::neo4j::Neo4jClient;
use crate::projects::handlers::{require_project_access, ProjectState};

use super::{
    extract_json_block, load_deployment_context, needs_destroy_before_apply,
    record_run_and_update_state, reset_infra_state_to_none, shape_deployment, InfraState,
};

const DEFAULT_RUN_TIMEOUT_SECS: u64 = 300;
const MAX_RUN_TIMEOUT_SECS:     u64 = 1800;

fn default_run_timeout() -> u64 { DEFAULT_RUN_TIMEOUT_SECS }

pub(crate) type ApiError = (StatusCode, Json<Value>);

pub(crate) fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub async fn require_group_access(
    neo4j: &Neo4jClient,
    user_id: &str,
    user_role: &str,
    group_id: &str,
) -> Result<Value, ApiError> {
    let rows = neo4j.query_read(
        "MATCH (g:Group {id: $gid})
         WHERE $role = 'admin'
            OR EXISTS { MATCH (:User {id: $uid})-[:MEMBER_OF]->(g) }
         RETURN g.id AS id, g.name AS name",
        json!({ "gid": group_id, "uid": user_id, "role": user_role }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))
}

pub async fn list_templates(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(group_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_group_access(&state.neo4j, &user.sub, &user.role, &group_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Group {id: $gid})-[:HAS_TEMPLATE]->(t:ProductTemplate)
         RETURN t.id AS id, t.name AS name, t.description AS description,
                t.created_by AS created_by, t.created_at AS created_at, t.updated_at AS updated_at
         ORDER BY t.name",
        json!({ "gid": group_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

pub async fn get_template(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((group_id, template_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_group_access(&state.neo4j, &user.sub, &user.role, &group_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Group {id: $gid})-[:HAS_TEMPLATE]->(t:ProductTemplate {id: $tid})
         RETURN t.id AS id, t.name AS name, t.description AS description, t.content AS content,
                t.created_by AS created_by, t.created_at AS created_at, t.updated_at AS updated_at",
        json!({ "gid": group_id, "tid": template_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(row))
}

#[derive(serde::Deserialize)]
pub struct CreateTemplateBody {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

pub async fn create_template(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(group_id): Path<String>,
    Json(body): Json<CreateTemplateBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_group_access(&state.neo4j, &user.sub, &user.role, &group_id).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    state.neo4j.query_read(
        "MATCH (g:Group {id: $gid})
         CREATE (t:ProductTemplate {
             id: $id, name: $name, description: $description, content: $content,
             created_by: $uid, created_at: $now, updated_at: $now
         })
         CREATE (g)-[:HAS_TEMPLATE]->(t)",
        json!({
            "gid": group_id, "id": id, "name": name, "description": body.description,
            "content": body.content, "uid": user.sub, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "created_at": now }))))
}

#[derive(serde::Deserialize)]
pub struct UpdateTemplateBody {
    pub name:        Option<String>,
    pub description: Option<String>,
    pub content:     Option<String>,
}

pub async fn update_template(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((group_id, template_id)): Path<(String, String)>,
    Json(body): Json<UpdateTemplateBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_group_access(&state.neo4j, &user.sub, &user.role, &group_id).await?;
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
    }
    let exists = state.neo4j.query_read(
        "MATCH (:Group {id: $gid})-[:HAS_TEMPLATE]->(t:ProductTemplate {id: $tid}) RETURN 1",
        json!({ "gid": group_id, "tid": template_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut set_clauses = vec!["t.updated_at = $now"];
    if body.name.is_some()        { set_clauses.push("t.name = $name"); }
    if body.description.is_some() { set_clauses.push("t.description = $description"); }
    if body.content.is_some()     { set_clauses.push("t.content = $content"); }
    let cypher = format!(
        "MATCH (:Group {{id: $gid}})-[:HAS_TEMPLATE]->(t:ProductTemplate {{id: $tid}}) SET {} RETURN t.id",
        set_clauses.join(", ")
    );
    let mut params = json!({ "gid": group_id, "tid": template_id, "now": now });
    if let Some(name)        = &body.name        { params["name"]        = json!(name.trim()); }
    if let Some(description) = &body.description { params["description"] = json!(description); }
    if let Some(content)     = &body.content     { params["content"]     = json!(content); }
    state.neo4j.query_read(&cypher, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_template(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((group_id, template_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_group_access(&state.neo4j, &user.sub, &user.role, &group_id).await?;
    state.neo4j.query_read(
        "MATCH (:Group {id: $gid})-[:HAS_TEMPLATE]->(t:ProductTemplate {id: $tid}) DETACH DELETE t",
        json!({ "gid": group_id, "tid": template_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_deployments(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)
         OPTIONAL MATCH (d)-[:USES_TEMPLATE]->(t:ProductTemplate)
         RETURN d.id AS id, d.name AS name, d.environment_description AS environment_description,
                d.infra_state AS infra_state, d.created_by AS created_by,
                d.created_at AS created_at, d.updated_at AS updated_at,
                t.id AS template_id, t.name AS template_name
         ORDER BY d.updated_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let shaped: Vec<Value> = rows.iter().map(shape_deployment).collect();
    Ok(Json(shaped))
}

fn deployment_detail_cypher() -> &'static str {
    "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
     OPTIONAL MATCH (d)-[:USES_TEMPLATE]->(t:ProductTemplate)
     OPTIONAL MATCH (d)-[:HAS_DESIGN_DOC]->(design:Artifact)
     OPTIONAL MATCH (d)-[:HAS_TERRAFORM_BUNDLE]->(tf:Artifact)
     OPTIONAL MATCH (d)-[:HAS_GUIDE]->(guide:Artifact)
     RETURN d.id AS id, d.name AS name, d.environment_description AS environment_description,
            d.infra_state AS infra_state, d.last_applied_artifact_id AS last_applied_artifact_id,
            d.last_applied_at AS last_applied_at, d.created_by AS created_by,
            d.created_at AS created_at, d.updated_at AS updated_at,
            t.id AS template_id, t.name AS template_name,
            design.id AS design_doc_id, design.title AS design_doc_title,
            tf.id AS terraform_bundle_id, tf.title AS terraform_bundle_title, tf.kind AS terraform_bundle_kind,
            guide.id AS guide_id, guide.title AS guide_title,
            d.diagnosis_status AS diagnosis_status, d.diagnosis_run_id AS diagnosis_run_id,
            d.diagnosis_explanation AS diagnosis_explanation, d.diagnosis_files AS diagnosis_files,
            d.diagnosis_error AS diagnosis_error"
}

async fn fetch_deployment_detail(
    neo4j: &Neo4jClient,
    project_id: &str,
    deployment_id: &str,
) -> Result<Value, ApiError> {
    let rows = neo4j.query_read(
        deployment_detail_cypher(),
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(shape_deployment(&row))
}

pub async fn get_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

#[derive(serde::Deserialize)]
pub struct CreateDeploymentBody {
    pub name:                     String,
    pub environment_description: String,
    pub product_template_id:      Option<String>,
}

pub async fn create_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateDeploymentBody>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }

    if let Some(template_id) = &body.product_template_id {
        let group_id = project["group_id"].as_str().unwrap_or_default();
        let exists = state.neo4j.query_read(
            "MATCH (:Group {id: $gid})-[:HAS_TEMPLATE]->(t:ProductTemplate {id: $tid}) RETURN 1",
            json!({ "gid": group_id, "tid": template_id }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        if exists.is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "template not found in this group"));
        }
    }

    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let cypher = if body.product_template_id.is_some() {
        "MATCH (p:Project {id: $pid}), (t:ProductTemplate {id: $tid})
         CREATE (d:Deployment {
             id: $id, name: $name, environment_description: $env_desc,
             infra_state: 'none', last_applied_content: null,
             last_applied_artifact_id: null, last_applied_at: null,
             created_by: $uid, created_at: $now, updated_at: $now
         })
         CREATE (p)-[:HAS_DEPLOYMENT]->(d)
         CREATE (d)-[:USES_TEMPLATE]->(t)
         RETURN d.id AS id"
    } else {
        "MATCH (p:Project {id: $pid})
         CREATE (d:Deployment {
             id: $id, name: $name, environment_description: $env_desc,
             infra_state: 'none', last_applied_content: null,
             last_applied_artifact_id: null, last_applied_at: null,
             created_by: $uid, created_at: $now, updated_at: $now
         })
         CREATE (p)-[:HAS_DEPLOYMENT]->(d)
         RETURN d.id AS id"
    };

    state.neo4j.query_read(
        cypher,
        json!({
            "pid": project_id, "tid": body.product_template_id, "id": id,
            "name": name, "env_desc": body.environment_description,
            "uid": user.sub, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "created_at": now }))))
}

#[derive(serde::Deserialize)]
pub struct UpdateDeploymentBody {
    pub name:                     Option<String>,
    pub environment_description: Option<String>,
}

pub async fn update_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<UpdateDeploymentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
    }
    let exists = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}) RETURN 1",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut set_clauses = vec!["d.updated_at = $now"];
    if body.name.is_some()                     { set_clauses.push("d.name = $name"); }
    if body.environment_description.is_some()  { set_clauses.push("d.environment_description = $env_desc"); }
    let cypher = format!(
        "MATCH (:Project {{id: $pid}})-[:HAS_DEPLOYMENT]->(d:Deployment {{id: $did}}) SET {} RETURN d.id",
        set_clauses.join(", ")
    );
    let mut params = json!({ "pid": project_id, "did": deployment_id, "now": now });
    if let Some(name)     = &body.name                    { params["name"]     = json!(name.trim()); }
    if let Some(env_desc) = &body.environment_description { params["env_desc"] = json!(env_desc); }
    state.neo4j.query_read(&cypher, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         OPTIONAL MATCH (d)-[:HAS_RUN]->(r:DeploymentRun)
         DETACH DELETE d, r",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct RunDeploymentBody {
    pub agent_id: String,
    #[serde(default = "default_run_timeout")]
    pub timeout_secs: u64,
}

pub(crate) struct RunnableBundle {
    pub(crate) artifact_id:          String,
    pub(crate) artifact_kind:        String,
    pub(crate) artifact_content:     String,
    pub(crate) infra_state:          InfraState,
    pub(crate) last_applied_content: Option<String>,
}

pub(crate) async fn load_runnable_bundle(
    neo4j: &Neo4jClient,
    project_id: &str,
    deployment_id: &str,
) -> Result<RunnableBundle, ApiError> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         OPTIONAL MATCH (d)-[:HAS_TERRAFORM_BUNDLE]->(a:Artifact)
         RETURN d.infra_state AS infra_state, d.last_applied_content AS last_applied_content,
                a.id AS artifact_id, a.kind AS artifact_kind, a.content AS artifact_content",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next().ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    let artifact_id = row["artifact_id"].as_str().map(str::to_string)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no terraform bundle linked"))?;
    Ok(RunnableBundle {
        artifact_id,
        artifact_kind:        row["artifact_kind"].as_str().unwrap_or("").to_string(),
        artifact_content:     row["artifact_content"].as_str().unwrap_or("").to_string(),
        infra_state:          InfraState::parse(row["infra_state"].as_str().unwrap_or("none")).unwrap_or(InfraState::None),
        last_applied_content: row["last_applied_content"].as_str().map(str::to_string),
    })
}

pub(crate) fn flavor_for_kind(kind: &str) -> Result<TerraformFlavor, ApiError> {
    match ArtifactKind::parse(kind) {
        Some(ArtifactKind::Terraform)  => Ok(TerraformFlavor::Terraform),
        Some(ArtifactKind::Terragrunt) => Ok(TerraformFlavor::Terragrunt),
        _ => Err(err(StatusCode::BAD_REQUEST, "linked artifact is not a terraform or terragrunt bundle")),
    }
}

fn require_agent_in_project(state: &ProjectState, agent_id: &str, project_id: &str) -> Result<(), ApiError> {
    let belongs = state.agent_builder.registry.agents.get(agent_id)
        .map(|a| a.project_id == project_id)
        .unwrap_or(false);
    if belongs { Ok(()) } else { Err(err(StatusCode::NOT_FOUND, "agent not found in this project")) }
}

/// Relays live terraform output lines onto the project's shared SSE bus (the same one
/// `/projects/:pid/events` uses for chat) so the browser can tail a run while it's in flight.
fn spawn_output_relay(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
) -> tokio::sync::mpsc::Sender<Value> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(256);
    let channels       = Arc::clone(&state.channels);
    let project_id     = project_id.to_string();
    let deployment_id  = deployment_id.to_string();

    tokio::spawn(async move {
        while let Some(mut line) = rx.recv().await {
            line["type"]          = json!("deployment_run_log");
            line["deployment_id"] = json!(deployment_id);
            let msg = line.to_string();
            let map = channels.lock().await;
            if let Some(sender) = map.get(&project_id) {
                let _ = sender.send(msg);
            }
        }
    });

    tx
}

fn spawn_progress_relay(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
) -> tokio::sync::mpsc::Sender<AgentEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let channels       = Arc::clone(&state.channels);
    let project_id     = project_id.to_string();
    let deployment_id  = deployment_id.to_string();

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let mut value = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
            value["deployment_id"] = json!(deployment_id);
            let msg = value.to_string();
            let map = channels.lock().await;
            if let Some(sender) = map.get(&project_id) {
                let _ = sender.send(msg);
            }
        }
    });

    tx
}

#[allow(clippy::too_many_arguments)]
async fn execute_and_record(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
    artifact_id:   &str,
    flavor:        TerraformFlavor,
    action:        TerraformAction,
    content:       &str,
    agent_id:      &str,
    timeout:       u64,
    reasoning:     Option<&str>,
) -> Result<Value, ApiError> {
    let files = bundle::parse_bundle(content).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    let files_json = serde_json::to_string(&files)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let output_tx = spawn_output_relay(state, project_id, deployment_id);

    let result = state.agent_builder.registry
        .execute_terraform(agent_id, artifact_id.to_string(), flavor, action, files, timeout, Some(output_tx))
        .await;

    let (exit_code, stdout, stderr) = match &result {
        Ok(r)  => (Some(r.exit_code), r.stdout.clone(), r.stderr.clone()),
        Err(e) => (None, String::new(), e.clone()),
    };
    let success          = exit_code == Some(0);
    let applied_content  = (action == TerraformAction::Apply && success).then_some(files_json.as_str());
    let infra_state = record_run_and_update_state(
        &state.neo4j, project_id, artifact_id, action, exit_code, &stdout, &stderr,
        applied_content, "user", reasoning,
    ).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    match result {
        Ok(r) => {
            let mut value = json!({
                "action": action.as_str(), "stdout": r.stdout, "stderr": r.stderr, "exit_code": r.exit_code,
            });
            if let Some(reason) = reasoning {
                value["reasoning"] = json!(reason);
            }
            if let Some(s) = infra_state {
                value["infra_state"] = json!(s);
            }
            Ok(value)
        }
        Err(e) => Err(err(StatusCode::BAD_GATEWAY, &e)),
    }
}

/// Core logic behind `POST .../deploy`, extractor-free so it stays testable independent of
/// the axum handler wrapper below.
pub(crate) async fn deploy_deployment_core(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
    agent_id:      &str,
    timeout_secs:  u64,
) -> Result<Value, ApiError> {
    require_agent_in_project(state, agent_id, project_id)?;
    let run = load_runnable_bundle(&state.neo4j, project_id, deployment_id).await?;
    let flavor = flavor_for_kind(&run.artifact_kind)?;
    let timeout = timeout_secs.min(MAX_RUN_TIMEOUT_SECS);

    let result = execute_and_record(
        state, project_id, deployment_id, &run.artifact_id, flavor, TerraformAction::Apply, &run.artifact_content,
        agent_id, timeout, None,
    ).await?;
    Ok(json!({ "runs": [result] }))
}

pub(crate) async fn redeploy_deployment_core(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
    agent_id:      &str,
    timeout_secs:  u64,
) -> Result<Value, ApiError> {
    let run = load_runnable_bundle(&state.neo4j, project_id, deployment_id).await?;
    let flavor = flavor_for_kind(&run.artifact_kind)?;
    let timeout = timeout_secs.min(MAX_RUN_TIMEOUT_SECS);

    let mut runs = Vec::new();
    let mut destroyed_first = false;

    if needs_destroy_before_apply(run.infra_state) {
        let Some(snapshot) = run.last_applied_content.clone() else {
            reset_infra_state_to_none(&state.neo4j, project_id, deployment_id).await
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
            return Err(err(
                StatusCode::BAD_REQUEST,
                "infra is in a broken state but nothing was ever successfully applied — reset to allow a fresh deploy",
            ));
        };
        let destroy_result = execute_and_record(
            state, project_id, deployment_id, &run.artifact_id, flavor, TerraformAction::Destroy, &snapshot,
            agent_id, timeout, Some("auto-destroy: previous run left infrastructure in a broken state"),
        ).await?;
        let destroy_ok = destroy_result["exit_code"].as_i64() == Some(0);
        runs.push(destroy_result);
        if !destroy_ok {
            return Ok(json!({ "runs": runs }));
        }
        destroyed_first = true;
    }

    let apply_reasoning = if destroyed_first {
        "applying after auto-destroy"
    } else {
        "infra already clean — applying directly"
    };
    let apply_result = execute_and_record(
        state, project_id, deployment_id, &run.artifact_id, flavor, TerraformAction::Apply, &run.artifact_content,
        agent_id, timeout, Some(apply_reasoning),
    ).await?;
    runs.push(apply_result);

    Ok(json!({ "runs": runs }))
}

pub(crate) async fn destroy_deployment_core(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
    agent_id:      &str,
    timeout_secs:  u64,
) -> Result<Value, ApiError> {
    require_agent_in_project(state, agent_id, project_id)?;
    let run = load_runnable_bundle(&state.neo4j, project_id, deployment_id).await?;
    if matches!(run.infra_state, InfraState::None | InfraState::Destroyed) {
        return Err(err(StatusCode::BAD_REQUEST, "nothing to destroy"));
    }
    let Some(snapshot) = run.last_applied_content.clone() else {
        reset_infra_state_to_none(&state.neo4j, project_id, deployment_id).await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        return Err(err(
            StatusCode::BAD_REQUEST,
            "nothing was ever successfully applied for this deployment — reset to allow a fresh deploy",
        ));
    };
    let flavor = flavor_for_kind(&run.artifact_kind)?;
    let timeout = timeout_secs.min(MAX_RUN_TIMEOUT_SECS);

    let result = execute_and_record(
        state, project_id, deployment_id, &run.artifact_id, flavor, TerraformAction::Destroy, &snapshot,
        agent_id, timeout, None,
    ).await?;
    Ok(json!({ "runs": [result] }))
}

pub async fn deploy_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<RunDeploymentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let value = deploy_deployment_core(&state, &project_id, &deployment_id, &body.agent_id, body.timeout_secs).await?;
    Ok(Json(value))
}

pub async fn redeploy_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<RunDeploymentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    require_agent_in_project(&state, &body.agent_id, &project_id)?;
    let value = redeploy_deployment_core(&state, &project_id, &deployment_id, &body.agent_id, body.timeout_secs).await?;
    Ok(Json(value))
}

pub async fn destroy_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<RunDeploymentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let value = destroy_deployment_core(&state, &project_id, &deployment_id, &body.agent_id, body.timeout_secs).await?;
    Ok(Json(value))
}

pub async fn list_deployment_runs(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_RUN]->(r:DeploymentRun)
         RETURN r.id AS id, r.action AS action, r.status AS status, r.exit_code AS exit_code,
                r.stdout_preview AS stdout_preview, r.stderr_preview AS stderr_preview,
                r.initiated_by AS initiated_by, r.reasoning AS reasoning, r.created_at AS created_at
         ORDER BY r.created_at DESC",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

async fn build_deployment_agent(
    state:         &ProjectState,
    project_id:    &str,
    group_id:      &str,
    deployment_id: &str,
) -> Result<Arc<Agent>, ApiError> {
    let ctx = load_deployment_context(&state.neo4j, project_id, deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(state.agent_builder.build_for_deployment(project_id.to_string(), group_id.to_string(), &ctx))
}

fn generation_failed(response_text: &str) -> ApiError {
    err(
        StatusCode::UNPROCESSABLE_ENTITY,
        &format!("could not parse a structured response: {}", response_text.chars().take(500).collect::<String>()),
    )
}

pub async fn generate_environment_questions(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default();
    let agent = build_deployment_agent(&state, &project_id, group_id, &deployment_id).await?;

    let prompt = "List 4 to 8 short, concrete questions a field engineer should answer about the \
                  customer's environment before designing this deployment, based on the product \
                  template and context you were given. Respond with a one-sentence intro, then a \
                  ```json fenced array of objects shaped like {\"id\": \"short-slug\", \"text\": \"the question\"}. \
                  Do not call any tools.";

    let response = agent.query(prompt, &[], &[], None).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let questions = extract_json_block(&response.answer)
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(|| generation_failed(&response.answer))?;

    Ok(Json(json!({ "questions": questions })))
}

pub async fn generate_design(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default();
    let agent = build_deployment_agent(&state, &project_id, group_id, &deployment_id).await?;

    let prompt = "Write a deployment design document in Markdown, based on the product template \
                  and customer environment you were given. Cover the architecture, key \
                  configuration choices, and how it fits the customer's environment. Then call \
                  generate_artifact with kind \"markdown\" to save it, and immediately call \
                  link_deployment_artifact with role \"design\" using the returned artifact id. \
                  Do not call any other tools.";

    agent.query(prompt, &[], &[], None).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

pub async fn generate_design_decisions(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default();
    let agent = build_deployment_agent(&state, &project_id, group_id, &deployment_id).await?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let design_doc_id = deployment["design_doc"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no design document yet"))?;
    let design = get_artifact_in_project(&state.neo4j, &project_id, design_doc_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "design document not found"))?;
    let design_content = design["content"].as_str().unwrap_or_default();

    let prompt = format!(
        "Here is the current design document:\n\n{design_content}\n\n\
         List 3 to 6 concrete design decisions the field engineer should confirm or override \
         before this design is finalized (e.g. sizing, networking, redundancy choices). Respond \
         with a one-sentence intro, then a ```json fenced array of objects shaped like \
         {{\"id\": \"short-slug\", \"text\": \"the decision to confirm\", \"suggested\": \"your suggested answer\"}}. \
         Do not call any tools."
    );

    let response = agent.query(&prompt, &[], &[], None).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let decisions = extract_json_block(&response.answer)
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(|| generation_failed(&response.answer))?;

    Ok(Json(json!({ "decisions": decisions })))
}

#[derive(serde::Deserialize)]
pub struct DesignDecisionAnswer {
    pub question: String,
    pub answer:   String,
}

#[derive(serde::Deserialize)]
pub struct ReviseDesignBody {
    #[serde(default)]
    pub decisions:    Vec<DesignDecisionAnswer>,
    pub instructions: Option<String>,
}

pub async fn revise_design(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<ReviseDesignBody>,
) -> Result<impl IntoResponse, ApiError> {
    if body.decisions.is_empty() && body.instructions.as_deref().unwrap_or("").trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "decisions or instructions are required"));
    }
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default();
    let agent = build_deployment_agent(&state, &project_id, group_id, &deployment_id).await?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let design_doc_id = deployment["design_doc"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no design document yet"))?
        .to_string();

    let decisions_text = body.decisions.iter()
        .map(|d| format!("- {}: {}", d.question, d.answer))
        .collect::<Vec<_>>()
        .join("\n");
    let instructions_text = body.instructions.as_deref().unwrap_or("").trim();

    let prompt = format!(
        "Revise the existing design document (artifact id: {design_doc_id}) to incorporate the \
         following. Confirmed decisions:\n{decisions_text}\n\nAdditional instructions:\n{instructions_text}\n\n\
         Call generate_artifact with kind \"markdown\", the same artifact_id \"{design_doc_id}\" \
         to revise it in place, and the complete updated document content. Then call \
         link_deployment_artifact with role \"design\" using that artifact id. Do not call any \
         other tools."
    );

    agent.query(&prompt, &[], &[], None).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

pub async fn generate_provision(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default();
    let agent = build_deployment_agent(&state, &project_id, group_id, &deployment_id).await?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let design_doc_id = deployment["design_doc"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no design document yet"))?;
    let design = get_artifact_in_project(&state.neo4j, &project_id, design_doc_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "design document not found"))?;
    let design_content = design["content"].as_str().unwrap_or_default();

    let prompt = format!(
        "Here is the design document:\n\n{design_content}\n\n\
         Write a Terraform or Terragrunt bundle implementing this design. Call generate_artifact \
         with kind \"terraform\" or \"terragrunt\" (content is a JSON object mapping file path to \
         file text), then immediately call link_deployment_artifact with role \"terraform\" using \
         the returned artifact id. Do not call any other tools."
    );

    let progress_tx = spawn_progress_relay(&state, &project_id, &deployment_id);
    agent.query_with_progress(&prompt, &[], &[], None, progress_tx).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

#[derive(serde::Deserialize)]
pub struct ProposeProvisionChangeBody {
    pub instructions:  Option<String>,
    pub error_context: Option<String>,
}

pub async fn propose_provision_change(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<ProposeProvisionChangeBody>,
) -> Result<impl IntoResponse, ApiError> {
    let instructions  = body.instructions.as_deref().unwrap_or("").trim().to_string();
    let error_context = body.error_context.as_deref().unwrap_or("").trim().to_string();
    if instructions.is_empty() && error_context.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "instructions or error_context are required"));
    }

    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default();
    let agent = build_deployment_agent(&state, &project_id, group_id, &deployment_id).await?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let bundle_id = deployment["terraform_bundle"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no terraform bundle yet"))?
        .to_string();
    let artifact = get_artifact_in_project(&state.neo4j, &project_id, &bundle_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "terraform bundle not found"))?;
    let current_content = artifact["content"].as_str().unwrap_or_default().to_string();
    let current_files: Value = serde_json::from_str(&current_content).unwrap_or_else(|_| json!({}));

    let error_section = if error_context.is_empty() {
        String::new()
    } else {
        format!("\n\nThe last run failed with this output:\n{error_context}")
    };
    let instructions_section = if instructions.is_empty() {
        String::new()
    } else {
        format!("\n\nThe user requested this change:\n{instructions}")
    };

    let prompt = format!(
        "Here is the current Terraform/Terragrunt bundle (JSON file map):\n{current_content}\
         {error_section}{instructions_section}\n\n\
         Propose a revised version of this bundle. Respond with a short explanation, then a \
         ```json fenced object mapping every file path (all files, whether changed or not) to its \
         complete new content. Do not call generate_artifact, link_deployment_artifact, \
         run_terraform_plan, run_terraform_apply, or run_terraform_destroy for this request — the \
         proposal will only be applied after the user reviews and approves it. You may use other \
         tools (e.g. run_command) to investigate first if that helps."
    );

    let response = agent.query(&prompt, &[], &[], None).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let proposed_files = extract_json_block(&response.answer)
        .ok_or_else(|| generation_failed(&response.answer))?;

    let explanation = match response.answer.find("```json") {
        Some(pos) => response.answer[..pos].trim().to_string(),
        None => response.answer.trim().to_string(),
    };

    Ok(Json(json!({
        "explanation":     explanation,
        "current_files":   current_files,
        "proposed_files":  proposed_files,
    })))
}

#[derive(serde::Deserialize)]
pub struct ApplyProvisionChangeBody {
    pub files: std::collections::BTreeMap<String, String>,
}

pub async fn apply_provision_change(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<ApplyProvisionChangeBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    bundle::validate_bundle(&body.files).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let bundle_id = deployment["terraform_bundle"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no terraform bundle yet"))?
        .to_string();
    let existing = get_artifact_in_project(&state.neo4j, &project_id, &bundle_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "terraform bundle not found"))?;
    let kind = ArtifactKind::parse(existing["kind"].as_str().unwrap_or(""))
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let title = existing["title"].as_str().unwrap_or("Infrastructure");
    let content = serde_json::to_string(&body.files)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    crate::artifacts::handlers::update_artifact(&state.neo4j, &bundle_id, kind, kind, title, &content)
        .await.map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;
    super::dismiss_diagnosis(&state.neo4j, &project_id, &deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

fn diagnosis_already_running(deployment: &Value, run_id: &str) -> bool {
    deployment["diagnosis"]["status"] == "running" && deployment["diagnosis"]["run_id"] == run_id
}

pub async fn diagnose_provision_failure(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default().to_string();

    let run = super::latest_failed_run(&state.neo4j, &project_id, &deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "no failed run to diagnose"))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    if diagnosis_already_running(&deployment, &run.id) {
        return Ok(Json(json!({ "started": false })));
    }

    let ctx = load_deployment_context(&state.neo4j, &project_id, &deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    super::start_diagnosis(&state.neo4j, &project_id, &deployment_id, &run.id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let agent = state.agent_builder.build_for_provision_diagnosis(project_id.clone(), group_id, &ctx, &run.id);
    let query = crate::agent::prompt::provision_diagnosis_query(&run);
    let progress_tx = spawn_progress_relay(&state, &project_id, &deployment_id);

    let neo4j          = Arc::clone(&state.neo4j);
    let project_id_bg  = project_id.clone();
    let deployment_id_bg = deployment_id.clone();
    let run_id_bg      = run.id.clone();
    tokio::spawn(async move {
        let outcome = agent.query_with_progress(&query, &[], &[], None, progress_tx).await;
        if let Err(e) = outcome {
            let _ = super::fail_diagnosis(&neo4j, &project_id_bg, &deployment_id_bg, &run_id_bg, &e.to_string()).await;
            return;
        }
        let status = super::diagnosis_status(&neo4j, &project_id_bg, &deployment_id_bg).await.ok().flatten();
        if status.as_deref() != Some("proposed") {
            let _ = super::fail_diagnosis(
                &neo4j, &project_id_bg, &deployment_id_bg, &run_id_bg,
                "diagnosis finished without proposing a fix",
            ).await;
        }
    });

    Ok(Json(json!({ "started": true })))
}

pub async fn dismiss_provision_diagnosis(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    super::dismiss_diagnosis(&state.neo4j, &project_id, &deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployment_with_diagnosis(status: &str, run_id: &str) -> Value {
        json!({ "diagnosis": { "status": status, "run_id": run_id } })
    }

    #[test]
    fn diagnosis_already_running_true_for_matching_running_run() {
        let d = deployment_with_diagnosis("running", "run-1");
        assert!(diagnosis_already_running(&d, "run-1"));
    }

    #[test]
    fn diagnosis_already_running_false_for_a_different_run_id() {
        let d = deployment_with_diagnosis("running", "run-1");
        assert!(!diagnosis_already_running(&d, "run-2"));
    }

    #[test]
    fn diagnosis_already_running_false_when_proposed() {
        let d = deployment_with_diagnosis("proposed", "run-1");
        assert!(!diagnosis_already_running(&d, "run-1"));
    }

    #[test]
    fn diagnosis_already_running_false_when_no_diagnosis_yet() {
        let d = json!({ "diagnosis": Value::Null });
        assert!(!diagnosis_already_running(&d, "run-1"));
    }
}
