use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::agent::{Agent, AgentEvent};
use crate::auth::jwt::Claims;
use crate::neo4j::Neo4jClient;

#[derive(Clone)]
pub struct ProjectState {
    pub neo4j:  Arc<Neo4jClient>,
    pub agent:  Arc<Agent>,
}

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

async fn require_project_access(
    neo4j: &Neo4jClient,
    user_id: &str,
    user_role: &str,
    project_id: &str,
) -> Result<Value, ApiError> {
    let rows = neo4j.query_read(
        "MATCH (g:Group)-[:HAS_PROJECT]->(p:Project {id: $pid})
         WHERE $role = 'admin'
            OR EXISTS { MATCH (:User {id: $uid})-[:MEMBER_OF]->(g) }
         RETURN p.id AS id, p.name AS name, p.description AS description,
                p.group_id AS group_id, g.name AS group_name,
                p.created_by AS created_by, p.created_at AS created_at",
        json!({ "pid": project_id, "uid": user_id, "role": user_role }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))
}

pub async fn list_my_groups(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = if user.role == "admin" {
        state.neo4j.query_read(
            "MATCH (g:Group)
             RETURN g.id AS id, g.name AS name, g.description AS description
             ORDER BY g.name",
            json!({}),
        ).await
    } else {
        state.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:MEMBER_OF]->(g:Group)
             RETURN g.id AS id, g.name AS name, g.description AS description
             ORDER BY g.name",
            json!({ "uid": user.sub }),
        ).await
    }
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(rows))
}

pub async fn list_projects(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = if user.role == "admin" {
        state.neo4j.query_read(
            "MATCH (g:Group)-[:HAS_PROJECT]->(p:Project)
             RETURN p.id AS id, p.name AS name, p.description AS description,
                    p.group_id AS group_id, g.name AS group_name,
                    p.created_by AS created_by, p.created_at AS created_at
             ORDER BY p.created_at DESC",
            json!({}),
        ).await
    } else {
        state.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:MEMBER_OF]->(g:Group)-[:HAS_PROJECT]->(p:Project)
             RETURN p.id AS id, p.name AS name, p.description AS description,
                    p.group_id AS group_id, g.name AS group_name,
                    p.created_by AS created_by, p.created_at AS created_at
             ORDER BY p.created_at DESC",
            json!({ "uid": user.sub }),
        ).await
    }
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct CreateProjectBody {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub group_id: String,
}

pub async fn create_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Json(body): Json<CreateProjectBody>,
) -> Result<impl IntoResponse, ApiError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    if name.len() > 100 {
        return Err(err(StatusCode::BAD_REQUEST, "name must be at most 100 characters"));
    }

    let group_rows = state.neo4j.query_read(
        "MATCH (g:Group {id: $gid}) RETURN g.id AS id",
        json!({ "gid": body.group_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    if group_rows.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "group not found"));
    }

    if user.role != "admin" {
        let member = state.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:MEMBER_OF]->(:Group {id: $gid}) RETURN 1 AS ok",
            json!({ "uid": user.sub, "gid": body.group_id }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

        if member.is_empty() {
            return Err(err(StatusCode::FORBIDDEN, "not a member of this group"));
        }
    }

    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let rows = state.neo4j.query_read(
        "MATCH (g:Group {id: $gid})
         CREATE (p:Project {
             id: $id, name: $name, description: $description,
             group_id: $gid, created_by: $uid, created_at: $now
         })
         CREATE (g)-[:HAS_PROJECT]->(p)
         RETURN p.id AS id, p.name AS name, p.description AS description,
                p.group_id AS group_id, g.name AS group_name,
                p.created_by AS created_by, p.created_at AS created_at",
        json!({
            "gid": body.group_id, "id": id, "name": name,
            "description": body.description, "uid": user.sub, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let project = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "failed to create project"))?;

    Ok((StatusCode::CREATED, Json(project)))
}

pub async fn get_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let project = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    Ok(Json(project))
}

#[derive(Deserialize)]
pub struct UpdateProjectBody {
    pub name:        Option<String>,
    pub description: Option<String>,
}

pub async fn update_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<UpdateProjectBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    if let Some(ref n) = body.name {
        if n.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
    }

    let mut sets: Vec<&str> = Vec::new();
    if body.name.is_some()        { sets.push("p.name = $name"); }
    if body.description.is_some() { sets.push("p.description = $description"); }

    if !sets.is_empty() {
        let cypher = format!(
            "MATCH (p:Project {{id: $pid}}) SET {} RETURN p.id AS id",
            sets.join(", ")
        );
        let mut params = json!({ "pid": project_id });
        if let Some(n) = &body.name        { params["name"]        = json!(n.trim()); }
        if let Some(d) = &body.description { params["description"] = json!(d); }
        state.neo4j.query_read(&cypher, params)
            .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    }

    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation) DETACH DELETE c",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    state.neo4j.query_read(
        "MATCH (p:Project {id: $pid}) DETACH DELETE p",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn list_conversations(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation)
         OPTIONAL MATCH (u:User {id: c.created_by})
         RETURN c.id AS id, c.title AS title,
                c.created_by AS created_by, u.name AS created_by_name,
                c.message_count AS message_count,
                c.created_at AS created_at, c.updated_at AS updated_at
         ORDER BY c.updated_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct CreateConvBody {
    pub title: Option<String>,
}

pub async fn create_conversation(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateConvBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    let id    = Uuid::new_v4().to_string();
    let now   = chrono::Utc::now().to_rfc3339();
    let title = body.title.unwrap_or_else(|| "New conversation".to_string());

    state.neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         CREATE (c:Conversation {
             id: $id, title: $title, messages: '[]',
             message_count: 0, created_by: $uid,
             created_at: $now, updated_at: $now
         })
         CREATE (p)-[:HAS_CONVERSATION]->(c)
         RETURN c.id AS id",
        json!({ "pid": project_id, "id": id, "title": title, "uid": user.sub, "now": now }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id, "title": title, "created_at": now }))))
}

pub async fn get_conversation(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, conv_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         RETURN c.id AS id, c.title AS title, c.messages AS messages,
                c.created_by AS created_by,
                c.created_at AS created_at, c.updated_at AS updated_at",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    let mut obj = row.as_object().cloned().unwrap_or_default();
    if let Some(Value::String(s)) = obj.get("messages") {
        if let Ok(parsed) = serde_json::from_str::<Value>(s) {
            obj.insert("messages".to_string(), parsed);
        }
    }

    Ok(Json(Value::Object(obj)))
}

#[derive(Deserialize)]
pub struct UpdateConvBody {
    pub title:    String,
    pub messages: Value,
}

pub async fn update_conversation(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, conv_id)): Path<(String, String)>,
    Json(body): Json<UpdateConvBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    let exists = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid}) RETURN 1",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }

    let now   = chrono::Utc::now().to_rfc3339();
    let count = body.messages.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let msgs  = body.messages.to_string();

    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         SET c.title = $title, c.messages = $messages,
             c.message_count = $count, c.updated_at = $now
         RETURN c.id AS id",
        json!({
            "pid": project_id, "cid": conv_id,
            "title": body.title, "messages": msgs,
            "count": count, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_conversation(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, conv_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         RETURN c.created_by AS created_by",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    let creator = row["created_by"].as_str().unwrap_or("");
    if user.role != "admin" && creator != user.sub {
        return Err(err(StatusCode::FORBIDDEN, "only the creator can delete this conversation"));
    }

    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         DETACH DELETE c",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct ProjectQueryBody {
    pub query: String,
}

pub async fn project_query(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectQueryBody>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }
    match state.agent.query(&body.query).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "project query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn project_query_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectQueryBody>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    let agent = Arc::clone(&state.agent);
    tokio::spawn(async move { agent.query_streaming(&body.query, tx).await; });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}
