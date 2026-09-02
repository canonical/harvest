use axum::{
    body::Body,
    extract::{Extension, Path, Query, State},
    http::{header, HeaderName, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, StreamExt as _};
use uuid::Uuid;

use crate::agent::{Agent, AgentEvent};
use crate::artifacts::{bundle, handlers::{create_artifact, get_artifact_in_project, sanitize_filename, ArtifactKind}};
use crate::auth::jwt::Claims;
use crate::machines::{TerraformAction, TerraformFlavor};
use crate::neo4j::Neo4jClient;
use crate::projects::handlers::{require_project_access, ProjectState};

use super::{
    extract_json_block, load_deployment_context, needs_destroy_before_apply,
    record_run_and_update_state, reset_infra_state_to_none, shape_deployment,
    shape_execution_plan, shape_proposal, shape_proposals,
    topological_sort, InfraState, StepAction, StepNode,
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
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (t:ProductTemplate)
          RETURN t.id AS id, t.name AS name, t.description AS description,
                 t.created_by AS created_by, t.created_at AS created_at, t.updated_at AS updated_at
          ORDER BY t.name",
        json!({}),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

pub async fn get_template(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(template_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (t:ProductTemplate {id: $tid})
          RETURN t.id AS id, t.name AS name, t.description AS description, t.content AS content,
                 t.created_by AS created_by, t.created_at AS created_at, t.updated_at AS updated_at",
        json!({ "tid": template_id }),
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
    Json(body): Json<CreateTemplateBody>,
) -> Result<impl IntoResponse, ApiError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    state.neo4j.query_read(
        "CREATE (t:ProductTemplate {
              id: $id, name: $name, description: $description, content: $content,
              created_by: $uid, created_at: $now, updated_at: $now
          }) RETURN t.id AS id",
        json!({
            "id": id, "name": name, "description": body.description,
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
    Path(template_id): Path<String>,
    Json(body): Json<UpdateTemplateBody>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
    }
    let exists = state.neo4j.query_read(
        "MATCH (t:ProductTemplate {id: $tid}) RETURN 1",
        json!({ "tid": template_id }),
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
        "MATCH (t:ProductTemplate {{id: $tid}}) SET {} RETURN t.id",
        set_clauses.join(", ")
    );
    let mut params = json!({ "tid": template_id, "now": now });
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
    Path(template_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.neo4j.query_read(
        "MATCH (t:ProductTemplate {id: $tid}) DETACH DELETE t",
        json!({ "tid": template_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload_template(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    mut multipart: axum::extract::Multipart,
) -> Result<impl IntoResponse, ApiError> {
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut filename = "upload.harvest".to_string();

    while let Some(field) = multipart.next_field().await
        .map_err(|e| err(StatusCode::BAD_REQUEST, &format!("multipart error: {e}")))?
    {
        if field.name() == Some("file") {
            if let Some(fname) = field.file_name() {
                filename = fname.to_string();
            }
            let bytes = field.bytes().await
                .map_err(|e| err(StatusCode::BAD_REQUEST, &format!("failed to read file: {e}")))?;
            file_bytes = Some(bytes.to_vec());
        }
    }

    let bytes = file_bytes
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "no file field in multipart request"))?;

    let parsed = super::harvest::parse_harvest_archive(&bytes)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;

    let name = super::harvest::derive_template_name(&parsed);
    let content = serde_json::to_string(&super::harvest::harvest_to_json(&parsed))
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "failed to serialize template content"))?;

    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    state.neo4j.query_read(
        "CREATE (t:ProductTemplate {
              id: $id, name: $name, description: $description, content: $content,
              created_by: $uid, created_at: $now, updated_at: $now
          }) RETURN t.id AS id",
        json!({
            "id": id, "name": name, "description": format!("Uploaded from {filename}"),
            "content": content, "uid": user.sub, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "created_at": now }))))
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
     OPTIONAL MATCH (creator:User {id: design.created_by})
     OPTIONAL MATCH (d)-[:HAS_TERRAFORM_BUNDLE]->(tf:Artifact)
     OPTIONAL MATCH (d)-[:HAS_GUIDE]->(guide:Artifact)
     OPTIONAL MATCH (d)-[:HAS_CONTEXT_ARTIFACT]->(ca:Artifact)
     WITH d, t, design, creator, tf, guide, collect({id: ca.id, title: ca.title, kind: ca.kind}) AS context_artifacts
     RETURN d.id AS id, d.name AS name, d.environment_description AS environment_description,
            d.infra_state AS infra_state, d.last_applied_artifact_id AS last_applied_artifact_id,
            d.last_applied_at AS last_applied_at, d.created_by AS created_by,
            d.created_at AS created_at, d.updated_at AS updated_at,
            t.id AS template_id, t.name AS template_name,
            design.id AS design_doc_id, design.title AS design_doc_title,
            design.created_by AS design_doc_created_by, design.created_at AS design_doc_created_at,
            design.updated_at AS design_doc_updated_at,
            creator.name AS design_doc_created_by_name,
            tf.id AS terraform_bundle_id, tf.title AS terraform_bundle_title, tf.kind AS terraform_bundle_kind,
            guide.id AS guide_id, guide.title AS guide_title,
            context_artifacts"
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

pub async fn get_project_deployment(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)
         WITH d ORDER BY d.created_at ASC LIMIT 1
         OPTIONAL MATCH (d)-[:USES_TEMPLATE]->(t:ProductTemplate)
         OPTIONAL MATCH (d)-[:HAS_DESIGN_DOC]->(design:Artifact)
         OPTIONAL MATCH (creator:User {id: design.created_by})
         OPTIONAL MATCH (d)-[:HAS_TERRAFORM_BUNDLE]->(tf:Artifact)
         OPTIONAL MATCH (d)-[:HAS_GUIDE]->(guide:Artifact)
         OPTIONAL MATCH (d)-[:HAS_CONTEXT_ARTIFACT]->(ca:Artifact)
         WITH d, t, design, creator, tf, guide, collect({id: ca.id, title: ca.title, kind: ca.kind}) AS context_artifacts
         RETURN d.id AS id, d.name AS name, d.environment_description AS environment_description,
                d.infra_state AS infra_state, d.last_applied_artifact_id AS last_applied_artifact_id,
                d.last_applied_at AS last_applied_at, d.created_by AS created_by,
                d.created_at AS created_at, d.updated_at AS updated_at,
                t.id AS template_id, t.name AS template_name,
                design.id AS design_doc_id, design.title AS design_doc_title,
                design.created_by AS design_doc_created_by, design.created_at AS design_doc_created_at,
                design.updated_at AS design_doc_updated_at,
                creator.name AS design_doc_created_by_name,
                tf.id AS terraform_bundle_id, tf.title AS terraform_bundle_title, tf.kind AS terraform_bundle_kind,
                guide.id AS guide_id, guide.title AS guide_title,
                context_artifacts",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "deployment not found"))?;
    Ok(Json(shape_deployment(&row)))
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

    let existing = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment)
         RETURN d.id AS id LIMIT 1",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if !existing.is_empty() {
        return Err(err(StatusCode::CONFLICT, "project already has a deployment"));
    }

    if let Some(template_id) = &body.product_template_id {
        let exists = state.neo4j.query_read(
            "MATCH (t:ProductTemplate {id: $tid}) RETURN 1",
            json!({ "tid": template_id }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        if exists.is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "template not found"));
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

async fn build_deployment_agent_text_only(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
) -> Result<Arc<Agent>, ApiError> {
    let ctx = load_deployment_context(&state.neo4j, project_id, deployment_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(state.agent_builder.build_for_deployment_text_only(&ctx))
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
    body: Json<GenerateDesignBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (agent, prompt) = prepare_design_generation(&state, &user, &project_id, &deployment_id, &body).await?;

    agent.query(&prompt, &[], &[], None).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

async fn prepare_design_generation(
    state:         &ProjectState,
    user:          &Claims,
    project_id:    &str,
    deployment_id: &str,
    body:          &GenerateDesignBody,
) -> Result<(Arc<Agent>, String), ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let group_id = project["group_id"].as_str().unwrap_or_default().to_string();

    if let Some(template_id) = &body.product_template_id {
        let exists = state.neo4j.query_read(
            "MATCH (t:ProductTemplate {id: $tid}) RETURN 1",
            json!({ "tid": template_id }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        if exists.is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "template not found"));
        }
        state.neo4j.query_read(
            "MATCH (d:Deployment {id: $did})
             OPTIONAL MATCH (d)-[old:USES_TEMPLATE]->(:ProductTemplate)
             DELETE old
             WITH d
             MATCH (t:ProductTemplate {id: $tid})
             CREATE (d)-[:USES_TEMPLATE]->(t)",
            json!({ "did": deployment_id, "tid": template_id }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    }

    let selected_artifacts = if body.artifact_ids.is_empty() {
        Vec::new()
    } else {
        let artifacts = state.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact)
             WHERE a.id IN $ids
             RETURN a.id AS id, a.title AS title, a.kind AS kind, a.content AS content
             ORDER BY a.title",
            json!({ "pid": project_id, "ids": body.artifact_ids }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        state.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}),
                    (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact)
             WHERE a.id IN $ids
             MERGE (d)-[:HAS_CONTEXT_ARTIFACT]->(a)",
            json!({ "pid": project_id, "did": deployment_id, "ids": body.artifact_ids }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        artifacts
    };

    let agent = build_deployment_agent(state, project_id, &group_id, deployment_id).await?;

    let mut prompt = String::from(
        "Write a deployment design document in Markdown, based on the product template \
         and customer environment you were given. Cover the architecture, key \
         configuration choices, and how it fits the customer's environment."
    );
    if !selected_artifacts.is_empty() {
        prompt.push_str("\n\n## Selected context artifacts\n\nThe field engineer selected the \
                         following project artifacts as relevant context. Read and use them to \
                         inform the design.\n\n");
        for a in &selected_artifacts {
            let title = a["title"].as_str().unwrap_or("(untitled)");
            let kind  = a["kind"].as_str().unwrap_or("markdown");
            let content = a["content"].as_str().unwrap_or("");
            prompt.push_str(&format!("### {title} ({kind})\n\n{content}\n\n"));
        }
    }
    prompt.push_str("Then call generate_artifact with kind \"markdown\" to save it, and \
                     immediately call link_deployment_artifact with role \"design\" using the \
                     returned artifact id. Do not call any other tools.");

    Ok((agent, prompt))
}

pub async fn generate_design_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    body: Json<GenerateDesignBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (agent, prompt) = prepare_design_generation(&state, &user, &project_id, &deployment_id, &body).await?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    tokio::spawn(async move {
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
        tokio::spawn(async move {
            agent.query_streaming(&prompt, &[], &[], None, agent_tx).await;
        });
        while let Some(event) = agent_rx.recv().await {
            let _ = tx.send(event).await;
        }
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(response)
}

#[derive(serde::Deserialize, Default)]
pub struct GenerateDesignBody {
    #[serde(default)]
    pub artifact_ids: Vec<String>,
    #[serde(default)]
    pub product_template_id: Option<String>,
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

#[derive(serde::Deserialize)]
pub struct ProposeDesignChangeBody {
    pub explanation: String,
    #[serde(default)]
    pub artifact_ids: Vec<String>,
}

async fn prepare_design_change_proposal(
    state:         &ProjectState,
    user:          &Claims,
    project_id:    &str,
    deployment_id: &str,
    body:          &ProposeDesignChangeBody,
) -> Result<(Arc<Agent>, String), ApiError> {
    let explanation = body.explanation.trim();
    if explanation.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "explanation is required"));
    }

    require_project_access(&state.neo4j, &user.sub, &user.role, project_id).await?;
    let agent = build_deployment_agent_text_only(state, project_id, deployment_id).await?;

    let deployment = fetch_deployment_detail(&state.neo4j, project_id, deployment_id).await?;
    let design_doc_id = deployment["design_doc"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no design document yet"))?
        .to_string();
    let design = get_artifact_in_project(&state.neo4j, project_id, &design_doc_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "design document not found"))?;
    let current_content = design["content"].as_str().unwrap_or_default().to_string();

    let selected_artifacts = if body.artifact_ids.is_empty() {
        Vec::new()
    } else {
        state.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact)
             WHERE a.id IN $ids
             RETURN a.id AS id, a.title AS title, a.kind AS kind, a.content AS content
             ORDER BY a.title",
            json!({ "pid": project_id, "ids": body.artifact_ids }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
    };

    let mut prompt = format!(
        "Here is the current design document:\n\n{current_content}\n\n\
         The field engineer requested the following change:\n\n{explanation}\n\n"
    );
    if !selected_artifacts.is_empty() {
        prompt.push_str("## Additional context artifacts\n\nRead and use these to inform the change.\n\n");
        for a in &selected_artifacts {
            let title = a["title"].as_str().unwrap_or("(untitled)");
            let kind  = a["kind"].as_str().unwrap_or("markdown");
            let content = a["content"].as_str().unwrap_or("");
            prompt.push_str(&format!("### {title} ({kind})\n\n{content}\n\n"));
        }
    }
    prompt.push_str(
        "Write the complete revised design document in Markdown, incorporating this change. \
         Respond with ONLY the full revised document text — no commentary, explanation, or \
         surrounding code fences. Do not call generate_artifact, link_deployment_artifact, or any \
         other tool for this request — the change will only be applied after the user reviews and \
         approves it."
    );

    Ok((agent, prompt))
}

pub async fn propose_design_change_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<ProposeDesignChangeBody>,
) -> Result<impl IntoResponse, ApiError> {
    let (agent, prompt) = prepare_design_change_proposal(&state, &user, &project_id, &deployment_id, &body).await?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    tokio::spawn(async move {
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
        tokio::spawn(async move {
            agent.query_streaming(&prompt, &[], &[], None, agent_tx).await;
        });
        while let Some(event) = agent_rx.recv().await {
            let _ = tx.send(event).await;
        }
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(response)
}

#[derive(serde::Deserialize, Default)]
pub struct DesignPdfQuery {
    #[serde(default)]
    pub download: bool,
}

pub async fn get_design_pdf(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Query(query): Query<DesignPdfQuery>,
) -> Result<Response, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let design_title = deployment["design_doc"]["title"].as_str().unwrap_or("Design").to_string();

    let resolved = super::design_cache::resolve_for_serving(state.neo4j.clone(), project_id.clone(), deployment_id.clone())
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    match resolved {
        super::design_cache::ResolvedPdf::NoDesignDoc => Err(err(StatusCode::BAD_REQUEST, "deployment has no design document yet")),
        super::design_cache::ResolvedPdf::Failed(msg) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, &msg)),
        super::design_cache::ResolvedPdf::Pending => {
            Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .header(HeaderName::from_static("retry-after"), HeaderValue::from_static("2"))
                .body(Body::from(json!({ "error": "design document preview is still being generated" }).to_string()))
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
        }
        super::design_cache::ResolvedPdf::Bytes { data, stale } => {
            let slug = sanitize_filename(&design_title);
            let disposition = if query.download {
                format!("attachment; filename=\"{slug}.pdf\"")
            } else {
                format!("inline; filename=\"{slug}.pdf\"")
            };
            let status_header = if stale { "stale" } else { "ready" };

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, HeaderValue::from_static("application/pdf"))
                .header(header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).unwrap())
                .header(HeaderName::from_static("x-design-pdf-status"), HeaderValue::from_static(status_header))
                .body(Body::from(data))
                .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))
        }
    }
}

#[derive(serde::Deserialize)]
pub struct UpdateDesignContentBody {
    pub title:   String,
    pub content: String,
}

pub async fn update_design_content(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<UpdateDesignContentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    let design_doc_id = deployment["design_doc"]["id"].as_str()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "deployment has no design document yet"))?
        .to_string();

    crate::artifacts::handlers::update_artifact(
        &state.neo4j, &design_doc_id, ArtifactKind::Markdown, ArtifactKind::Markdown, &body.title, &body.content,
    ).await.map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;

    super::design_cache::schedule_regeneration(state.neo4j.clone(), project_id.clone(), deployment_id.clone());

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

pub async fn generate_provision(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let (agent, prompt) = prepare_provision_generation(&state, &user, &project_id, &deployment_id).await?;

    let progress_tx = spawn_progress_relay(&state, &project_id, &deployment_id);
    agent.query_with_progress(&prompt, &[], &[], None, progress_tx).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

pub async fn generate_provision_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let (agent, prompt) = prepare_provision_generation(&state, &user, &project_id, &deployment_id).await?;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    tokio::spawn(async move {
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
        tokio::spawn(async move {
            agent.query_streaming(&prompt, &[], &[], None, agent_tx).await;
        });
        while let Some(event) = agent_rx.recv().await {
            let _ = tx.send(event).await;
        }
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    Ok(response)
}

async fn prepare_provision_generation(
    state:         &ProjectState,
    user:          &Claims,
    project_id:    &str,
    deployment_id: &str,
) -> Result<(Arc<Agent>, String), ApiError> {
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
         Write the Terraform or Terragrunt bundle (and any bash prep scripts the design calls for) \
         implementing this design. For each artifact, call generate_artifact with the appropriate \
         kind (\"terraform\", \"terragrunt\", or \"bash\"), then call link_deployment_artifact with \
         role \"terraform\" for each terraform/terragrunt bundle. \
         \
         After all artifacts are generated and linked, call set_execution_plan to define the \
         deployment DAG. The deploy plan should list every step needed to bring the infrastructure \
         up in the right order (bash scripts with action \"run\", terraform bundles with action \
         \"apply\"), using depends_on to express ordering. The destroy plan must include a \
         \"destroy\" step for every artifact that has an \"apply\" step in the deploy plan — \
         terraform destroy is the inverse of apply. If a bash script needs teardown, include it \
         with action \"run\" in the destroy plan. Use depends_on in the destroy plan to tear down \
         in the reverse order from deploy. \
         \
         You may call generate_artifact, link_deployment_artifact, and set_execution_plan. \
         Do not call run_terraform_plan, run_terraform_apply, run_terraform_destroy, \
         deploy_deployment, redeploy_deployment, or destroy_deployment."
    );

    Ok((agent, prompt))
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

    let deployment = fetch_deployment_detail(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(deployment))
}

pub(crate) async fn add_context_artifact_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id:  &str,
    title:         &str,
    kind:          ArtifactKind,
    content:       &str,
) -> Result<Value, ApiError> {
    let created = create_artifact(neo4j, project_id, kind, title, content, "user")
        .await.map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let artifact_id = created["id"].as_str().unwrap_or_default().to_string();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}),
                (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact {id: $aid})
         CREATE (d)-[:HAS_CONTEXT_ARTIFACT]->(a)",
        json!({ "pid": project_id, "did": deployment_id, "aid": artifact_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let deployment = fetch_deployment_detail(neo4j, project_id, deployment_id).await?;
    Ok(deployment)
}

pub(crate) async fn link_context_artifact_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    artifact_id:   &str,
) -> Result<Value, ApiError> {
    let artifact = get_artifact_in_project(neo4j, project_id, artifact_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "artifact not found in this project"))?;
    let aid = artifact["id"].as_str().unwrap_or_default().to_string();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}),
                (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact {id: $aid})
         MERGE (d)-[:HAS_CONTEXT_ARTIFACT]->(a)",
        json!({ "pid": project_id, "did": deployment_id, "aid": aid }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let deployment = fetch_deployment_detail(neo4j, project_id, deployment_id).await?;
    Ok(deployment)
}

pub(crate) async fn remove_context_artifact_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    artifact_id:   &str,
) -> Result<(), ApiError> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         MATCH (d)-[r:HAS_CONTEXT_ARTIFACT]->(:Artifact {id: $aid})
         DELETE r RETURN 1",
        json!({ "pid": project_id, "did": deployment_id, "aid": artifact_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if rows.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "context artifact not linked to this deployment"));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct AddContextArtifactBody {
    pub title:   String,
    pub kind:    String,
    pub content: String,
}

pub async fn add_context_artifact(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<AddContextArtifactBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let title = body.title.trim();
    if title.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "title is required"));
    }
    let kind = ArtifactKind::parse(&body.kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "kind must be 'markdown', 'pdf', 'terraform', 'terragrunt', or 'bash'"))?;
    let deployment = add_context_artifact_core(
        &state.neo4j, &project_id, &deployment_id, title, kind, &body.content,
    ).await?;
    Ok((StatusCode::CREATED, Json(deployment)))
}

#[derive(serde::Deserialize)]
pub struct LinkContextArtifactBody {
    pub artifact_id: String,
}

pub async fn link_context_artifact(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<LinkContextArtifactBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let deployment = link_context_artifact_core(
        &state.neo4j, &project_id, &deployment_id, &body.artifact_id,
    ).await?;
    Ok(Json(deployment))
}

pub async fn remove_context_artifact(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id, artifact_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    remove_context_artifact_core(&state.neo4j, &project_id, &deployment_id, &artifact_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn propose_artifact_change_core(
    neo4j:           &Neo4jClient,
    project_id:      &str,
    deployment_id:   &str,
    artifact_id:     &str,
    source:          &str,
    explanation:     &str,
    current_content: &str,
    proposed_content: &str,
) -> Result<Value, ApiError> {
    let artifact = get_artifact_in_project(neo4j, project_id, artifact_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "artifact not found in this project"))?;
    let kind = artifact["kind"].as_str().unwrap_or("").to_string();
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}),
                (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact {id: $aid})
         CREATE (p:Proposal {
             id: $id, source: $source, explanation: $explanation,
             current_content: $current_content, proposed_content: $proposed_content,
             status: 'pending', created_at: $now
         })
         CREATE (d)-[:HAS_PROPOSAL]->(p)
         CREATE (p)-[:TARGETS]->(a)",
        json!({
            "pid": project_id, "did": deployment_id, "aid": artifact_id, "id": id,
            "source": source, "explanation": explanation,
            "current_content": current_content, "proposed_content": proposed_content, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let rows = neo4j.query_read(
        "MATCH (:Deployment {id: $did})-[:HAS_PROPOSAL]->(p:Proposal {id: $id})
         RETURN p.id AS id, p.source AS source, p.explanation AS explanation,
                p.current_content AS current_content, p.proposed_content AS proposed_content,
                p.status AS status, p.created_at AS created_at, $aid AS target_artifact_id, $kind AS target_artifact_kind",
        json!({ "did": deployment_id, "id": id, "aid": artifact_id, "kind": kind }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(shape_proposal(&row))
}

pub(crate) async fn list_proposals_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    status_filter: Option<&str>,
) -> Result<Value, ApiError> {
    let (cypher, params) = match status_filter {
        Some(status) => (
            "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_PROPOSAL]->(p:Proposal {status: $status})
             MATCH (p)-[:TARGETS]->(a:Artifact)
             RETURN p.id AS id, p.source AS source, p.explanation AS explanation,
                    p.current_content AS current_content, p.proposed_content AS proposed_content,
                    p.status AS status, p.created_at AS created_at,
                    a.id AS target_artifact_id, a.kind AS target_artifact_kind
             ORDER BY p.created_at DESC",
            json!({ "pid": project_id, "did": deployment_id, "status": status }),
        ),
        None => (
            "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_PROPOSAL]->(p:Proposal)
             MATCH (p)-[:TARGETS]->(a:Artifact)
             RETURN p.id AS id, p.source AS source, p.explanation AS explanation,
                    p.current_content AS current_content, p.proposed_content AS proposed_content,
                    p.status AS status, p.created_at AS created_at,
                    a.id AS target_artifact_id, a.kind AS target_artifact_kind
             ORDER BY p.created_at DESC",
            json!({ "pid": project_id, "did": deployment_id }),
        ),
    };
    let rows = neo4j.query_read(cypher, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(shape_proposals(&rows))
}

pub(crate) async fn approve_proposal_core(
    neo4j:         Arc<Neo4jClient>,
    project_id:    &str,
    deployment_id: &str,
    proposal_id:   &str,
    edited_content: Option<&str>,
) -> Result<Value, ApiError> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_PROPOSAL]->(p:Proposal {id: $propid})
         MATCH (p)-[:TARGETS]->(a:Artifact)
         RETURN p.status AS status, p.proposed_content AS proposed_content, a.id AS artifact_id, a.kind AS artifact_kind, a.title AS artifact_title",
        json!({ "pid": project_id, "did": deployment_id, "propid": proposal_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "proposal not found"))?;
    let status = row["status"].as_str().unwrap_or("");
    if status != "pending" {
        return Err(err(StatusCode::BAD_REQUEST, "proposal is not pending"));
    }
    let artifact_id = row["artifact_id"].as_str().unwrap_or_default().to_string();
    let artifact_kind_str = row["artifact_kind"].as_str().unwrap_or("").to_string();
    let artifact_title = row["artifact_title"].as_str().unwrap_or("Artifact").to_string();
    let kind = ArtifactKind::parse(&artifact_kind_str)
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "target artifact has unknown kind"))?;
    let new_content = edited_content
        .map(str::to_string)
        .unwrap_or_else(|| row["proposed_content"].as_str().unwrap_or_default().to_string());
    crate::artifacts::handlers::update_artifact(&neo4j, &artifact_id, kind, kind, &artifact_title, &new_content)
        .await.map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;
    if kind == ArtifactKind::Markdown {
        let _ = super::design_cache::on_artifact_changed(neo4j.clone(), project_id.to_string(), artifact_id.clone()).await;
    }
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_PROPOSAL]->(p:Proposal {id: $propid})
         SET p.status = 'approved', p.proposed_content = $content",
        json!({ "pid": project_id, "did": deployment_id, "propid": proposal_id, "content": new_content, "now": now }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let deployment = fetch_deployment_detail(&neo4j, project_id, deployment_id).await?;
    Ok(json!({ "proposal_id": proposal_id, "status": "approved", "deployment": deployment }))
}

pub(crate) async fn discard_proposal_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    proposal_id:   &str,
) -> Result<(), ApiError> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_PROPOSAL]->(p:Proposal {id: $propid})
         WHERE p.status = 'pending'
         SET p.status = 'discarded' RETURN 1",
        json!({ "pid": project_id, "did": deployment_id, "propid": proposal_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if rows.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "pending proposal not found"));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct ProposeChangeBody {
    pub artifact_id:      String,
    pub source:           Option<String>,
    pub explanation:      String,
    pub current_content:  String,
    pub proposed_content: String,
}

pub async fn propose_artifact_change(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<ProposeChangeBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    if body.explanation.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "explanation is required"));
    }
    if body.proposed_content.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "proposed_content is required"));
    }
    let source = body.source.as_deref().unwrap_or("agent").trim();
    let source = if source.is_empty() { "agent" } else { source };
    let proposal = propose_artifact_change_core(
        &state.neo4j, &project_id, &deployment_id, &body.artifact_id,
        source, &body.explanation, &body.current_content, &body.proposed_content,
    ).await?;
    Ok((StatusCode::CREATED, Json(proposal)))
}

pub async fn list_proposals(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let proposals = list_proposals_core(&state.neo4j, &project_id, &deployment_id, None).await?;
    Ok(Json(proposals))
}

#[derive(serde::Deserialize)]
pub struct ApproveProposalBody {
    pub edited_content: Option<String>,
}

pub async fn approve_proposal(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id, proposal_id)): Path<(String, String, String)>,
    Json(body): Json<ApproveProposalBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let result = approve_proposal_core(
        state.neo4j.clone(), &project_id, &deployment_id, &proposal_id,
        body.edited_content.as_deref(),
    ).await?;
    Ok(Json(result))
}

pub async fn discard_proposal(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id, proposal_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    discard_proposal_core(&state.neo4j, &project_id, &deployment_id, &proposal_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone, serde::Deserialize)]
pub struct ParsedStepInput {
    pub artifact_id: String,
    pub action:      String,
    pub label:       String,
    pub depends_on:  Vec<usize>,
}

pub(crate) async fn set_execution_plan_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    deploy_steps:  &[ParsedStepInput],
    destroy_steps: &[ParsedStepInput],
) -> Result<(), ApiError> {
    for step in deploy_steps.iter().chain(destroy_steps.iter()) {
        let artifact = get_artifact_in_project(neo4j, project_id, &step.artifact_id)
            .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
            .ok_or_else(|| err(StatusCode::NOT_FOUND, &format!("artifact {} not found in this project", step.artifact_id)))?;
        let kind_str = artifact["kind"].as_str().unwrap_or("");
        if !crate::agent::deployment_tools::action_valid_for_kind(
            ArtifactKind::parse(kind_str).unwrap_or(ArtifactKind::Markdown),
            &step.action,
        ) {
            return Err(err(StatusCode::BAD_REQUEST, &format!(
                "action '{}' is not valid for artifact {} of kind '{}'",
                step.action, step.artifact_id, kind_str,
            )));
        }
    }

    let coverage_plan = crate::agent::deployment_tools::ParsedExecutionPlan {
        deploy_steps:  deploy_steps.iter().map(|s| crate::agent::deployment_tools::ParsedStep {
            artifact_id: s.artifact_id.clone(),
            action:      s.action.clone(),
            label:       s.label.clone(),
            depends_on:  s.depends_on.clone(),
        }).collect(),
        destroy_steps: destroy_steps.iter().map(|s| crate::agent::deployment_tools::ParsedStep {
            artifact_id: s.artifact_id.clone(),
            action:      s.action.clone(),
            label:       s.label.clone(),
            depends_on:  s.depends_on.clone(),
        }).collect(),
    };
    crate::agent::deployment_tools::validate_terraform_destroy_coverage(&coverage_plan)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e.to_string()))?;

    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         OPTIONAL MATCH (d)-[:HAS_EXECUTION_STEP]->(s:ExecutionStep)
         DETACH DELETE s",
        json!({ "pid": project_id, "did": deployment_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let now = chrono::Utc::now().to_rfc3339();
    let deploy_ids: Vec<String> = (0..deploy_steps.len()).map(|_| Uuid::new_v4().to_string()).collect();
    let destroy_ids: Vec<String> = (0..destroy_steps.len()).map(|_| Uuid::new_v4().to_string()).collect();

    for (i, step) in deploy_steps.iter().enumerate() {
        let step_id = &deploy_ids[i];
        neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}),
                    (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact {id: $aid})
             CREATE (s:ExecutionStep {
                 id: $sid, action: $action, phase: 'deploy', label: $label,
                 step_index: $idx, created_at: $now
             })
             CREATE (d)-[:HAS_EXECUTION_STEP]->(s)
             CREATE (s)-[:RUNS]->(a)",
            json!({
                "pid": project_id, "did": deployment_id, "aid": step.artifact_id,
                "sid": step_id, "action": step.action, "label": step.label,
                "idx": i, "now": now,
            }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    }
    for (i, step) in destroy_steps.iter().enumerate() {
        let step_id = &destroy_ids[i];
        neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did}),
                    (:Project {id: $pid})-[:HAS_ARTIFACT]->(a:Artifact {id: $aid})
             CREATE (s:ExecutionStep {
                 id: $sid, action: $action, phase: 'destroy', label: $label,
                 step_index: $idx, created_at: $now
             })
             CREATE (d)-[:HAS_EXECUTION_STEP]->(s)
             CREATE (s)-[:RUNS]->(a)",
            json!({
                "pid": project_id, "did": deployment_id, "aid": step.artifact_id,
                "sid": step_id, "action": step.action, "label": step.label,
                "idx": i, "now": now,
            }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    }

    for (i, step) in deploy_steps.iter().enumerate() {
        for &dep in &step.depends_on {
            neo4j.query_read(
                "MATCH (dep:ExecutionStep {id: $dep_id}), (target:ExecutionStep {id: $target_id})
                 CREATE (target)-[:DEPENDS_ON]->(dep)",
                json!({ "dep_id": &deploy_ids[dep], "target_id": &deploy_ids[i] }),
            ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        }
    }
    for (i, step) in destroy_steps.iter().enumerate() {
        for &dep in &step.depends_on {
            neo4j.query_read(
                "MATCH (dep:ExecutionStep {id: $dep_id}), (target:ExecutionStep {id: $target_id})
                 CREATE (target)-[:DEPENDS_ON]->(dep)",
                json!({ "dep_id": &destroy_ids[dep], "target_id": &destroy_ids[i] }),
            ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        }
    }

    Ok(())
}

async fn fetch_execution_plan_rows(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    phase:         &str,
) -> Result<Vec<Value>, ApiError> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(:Deployment {id: $did})-[:HAS_EXECUTION_STEP]->(s:ExecutionStep {phase: $phase})
         OPTIONAL MATCH (s)-[:RUNS]->(a:Artifact)
         OPTIONAL MATCH (s)-[:DEPENDS_ON]->(dep:ExecutionStep)
         RETURN s.id AS id, s.action AS action, s.phase AS phase, s.label AS label,
                s.step_index AS step_index,
                a.id AS artifact_id, a.kind AS artifact_kind, a.title AS artifact_title,
                [x IN collect(dep.id) WHERE x IS NOT NULL] AS depends_on
         ORDER BY s.step_index",
        json!({ "pid": project_id, "did": deployment_id, "phase": phase }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(rows)
}

pub(crate) async fn get_execution_plan_core(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
) -> Result<Value, ApiError> {
    let deploy_rows  = fetch_execution_plan_rows(neo4j, project_id, deployment_id, "deploy").await?;
    let destroy_rows = fetch_execution_plan_rows(neo4j, project_id, deployment_id, "destroy").await?;
    Ok(json!({
        "deploy_steps":  shape_execution_plan(&deploy_rows),
        "destroy_steps": shape_execution_plan(&destroy_rows),
    }))
}

pub async fn get_execution_plan(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let plan = get_execution_plan_core(&state.neo4j, &project_id, &deployment_id).await?;
    Ok(Json(plan))
}

#[derive(serde::Deserialize)]
pub struct SetExecutionPlanBody {
    pub deploy_steps:  Vec<ParsedStepInput>,
    pub destroy_steps: Vec<ParsedStepInput>,
}

pub async fn set_execution_plan(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<SetExecutionPlanBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let deploy_nodes: Vec<StepNode> = body.deploy_steps.iter()
        .map(|s| StepNode {
            id:         s.artifact_id.clone(),
            depends_on: s.depends_on.iter()
                .map(|&idx| body.deploy_steps.get(idx)
                    .map(|x| x.artifact_id.clone())
                    .unwrap_or_default())
                .collect(),
        })
        .collect();
    let destroy_nodes: Vec<StepNode> = body.destroy_steps.iter()
        .map(|s| StepNode {
            id:         s.artifact_id.clone(),
            depends_on: s.depends_on.iter()
                .map(|&idx| body.destroy_steps.get(idx)
                    .map(|x| x.artifact_id.clone())
                    .unwrap_or_default())
                .collect(),
        })
        .collect();
    topological_sort(&deploy_nodes).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    topological_sort(&destroy_nodes).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;

    set_execution_plan_core(
        &state.neo4j, &project_id, &deployment_id,
        &body.deploy_steps, &body.destroy_steps,
    ).await?;
    let plan = get_execution_plan_core(&state.neo4j, &project_id, &deployment_id).await?;
    Ok((StatusCode::CREATED, Json(plan)))
}

const DAG_STDOUT_PREVIEW_CHARS: usize = 2000;
const DAG_STDERR_PREVIEW_CHARS: usize = 2000;

fn dag_preview(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[allow(clippy::too_many_arguments)]
async fn record_dag_step_run(
    neo4j:         &Neo4jClient,
    project_id:    &str,
    deployment_id: &str,
    step_id:       &str,
    artifact_id:   &str,
    action:        &str,
    exit_code:     Option<i32>,
    stdout:        &str,
    stderr:        &str,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let rid = uuid::Uuid::new_v4().to_string();
    let success = exit_code == Some(0);
    neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         CREATE (r:DeploymentRun {
             id: $rid, action: $action, status: $status, exit_code: $exit_code,
             stdout_preview: $stdout_preview, stderr_preview: $stderr_preview,
             step_id: $step_id, artifact_id: $aid,
             initiated_by: 'user', created_at: $now
         })
         CREATE (d)-[:HAS_RUN]->(r)",
        json!({
            "pid": project_id, "did": deployment_id, "rid": rid,
            "action": action, "status": if success { "success" } else { "failed" },
            "exit_code": exit_code,
            "stdout_preview": dag_preview(stdout, DAG_STDOUT_PREVIEW_CHARS),
            "stderr_preview": dag_preview(stderr, DAG_STDERR_PREVIEW_CHARS),
            "step_id": step_id, "aid": artifact_id, "now": now,
        }),
    ).await?;
    Ok(())
}

async fn execute_dag_step(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
    step:          &Value,
    agent_id:      &str,
    timeout:       u64,
) -> Result<Value, ApiError> {
    let step_id    = step["id"].as_str().unwrap_or_default().to_string();
    let action_str = step["action"].as_str().unwrap_or_default().to_string();
    let artifact_id = step["artifact_id"].as_str().unwrap_or_default().to_string();
    let artifact_kind_str = step["artifact_kind"].as_str().unwrap_or_default().to_string();
    let label      = step["label"].as_str().unwrap_or_default().to_string();

    let artifact = get_artifact_in_project(&state.neo4j, project_id, &artifact_id)
        .await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "artifact not found"))?;
    let content = artifact["content"].as_str().unwrap_or_default().to_string();

    let output_tx = spawn_output_relay(state, project_id, deployment_id);

    let (exit_code, stdout, stderr) = match artifact_kind_str.as_str() {
        "bash" => {
            let result = state.agent_builder.registry
                .execute(agent_id, content, timeout)
                .await;
            match &result {
                Ok(r) => (Some(r.exit_code), r.stdout.clone(), r.stderr.clone()),
                Err(e) => (None, String::new(), e.clone()),
            }
        }
        "terraform" | "terragrunt" => {
            let flavor = match artifact_kind_str.as_str() {
                "terraform"  => TerraformFlavor::Terraform,
                "terragrunt" => TerraformFlavor::Terragrunt,
                _            => unreachable!(),
            };
            let action = StepAction::parse(&action_str)
                .and_then(|a| a.to_terraform_action())
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, &format!("invalid action '{}' for terraform", action_str)))?;
            let files = bundle::parse_bundle(&content)
                .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
            let result = state.agent_builder.registry
                .execute_terraform(agent_id, artifact_id.clone(), flavor, action, files, timeout, Some(output_tx))
                .await;
            match &result {
                Ok(r) => (Some(r.exit_code), r.stdout.clone(), r.stderr.clone()),
                Err(e) => (None, String::new(), e.clone()),
            }
        }
        _ => return Err(err(StatusCode::BAD_REQUEST, &format!("cannot execute artifact of kind '{}'", artifact_kind_str))),
    };

    let success = exit_code == Some(0);
    record_dag_step_run(
        &state.neo4j, project_id, deployment_id, &step_id, &artifact_id,
        &action_str, exit_code, &stdout, &stderr,
    ).await.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    let mut value = json!({
        "step_id": step_id,
        "action":  action_str,
        "label":   label,
        "artifact_id": artifact_id,
        "stdout":  stdout,
        "stderr":  stderr,
        "exit_code": exit_code,
    });
    value["success"] = json!(success);
    Ok(value)
}

pub(crate) async fn run_dag_core(
    state:         &ProjectState,
    project_id:    &str,
    deployment_id: &str,
    agent_id:      &str,
    timeout_secs:  u64,
) -> Result<Value, ApiError> {
    require_agent_in_project(state, agent_id, project_id)?;
    let rows = fetch_execution_plan_rows(&state.neo4j, project_id, deployment_id, "deploy").await?;
    if rows.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "no deploy steps configured — set an execution plan first"));
    }

    let step_map: HashMap<String, &Value> = rows.iter()
        .map(|r| (r["id"].as_str().unwrap_or_default().to_string(), r))
        .collect();
    let step_nodes: Vec<StepNode> = rows.iter()
        .map(|r| {
            let id = r["id"].as_str().unwrap_or_default().to_string();
            let deps: Vec<String> = r.get("depends_on")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter()
                    .filter_map(|d| d.as_str().map(String::from))
                    .collect())
                .unwrap_or_default();
            StepNode { id, depends_on: deps }
        })
        .collect();
    let order = topological_sort(&step_nodes).map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    let timeout = timeout_secs.min(MAX_RUN_TIMEOUT_SECS);
    let mut runs = Vec::new();
    let mut all_success = true;

    for step_id in &order {
        let step = step_map.get(step_id).copied()
            .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "step not found"))?;
        let result = execute_dag_step(state, project_id, deployment_id, step, agent_id, timeout).await?;
        let success = result["success"].as_bool().unwrap_or(false);
        runs.push(result);
        if !success {
            all_success = false;
            break;
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let new_state = if all_success { InfraState::Up } else { InfraState::Broken };
    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         SET d.infra_state = $new_state, d.updated_at = $now",
        json!({ "pid": project_id, "did": deployment_id, "new_state": new_state.as_str(), "now": now }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(json!({ "runs": runs, "infra_state": new_state.as_str() }))
}

pub async fn run_dag(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, deployment_id)): Path<(String, String)>,
    Json(body): Json<RunDeploymentBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let value = run_dag_core(&state, &project_id, &deployment_id, &body.agent_id, body.timeout_secs).await?;
    Ok(Json(value))
}
