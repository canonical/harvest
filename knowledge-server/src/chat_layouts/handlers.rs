use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::jwt::Claims;
use harvest_db::Db;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Deserialize)]
pub struct ProjectScopeParams {
    pub project_id: Option<String>,
}

fn scope_id(project_id: Option<String>) -> String {
    project_id.unwrap_or_default()
}

#[derive(Clone)]
pub struct ChatLayoutState {
    pub db: Arc<Db>,
}

fn parse_tree(row: &Value) -> Value {
    row.get("tree")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null)
}

pub async fn get_current(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Query(params): Query<ProjectScopeParams>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT tree, updated_at FROM chat_layouts
         WHERE user_id = $uid AND kind = 'current' AND project_id = $pid",
        json!({ "uid": user.sub, "pid": scope_id(params.project_id) }),
    ).await.map_err(|e| { tracing::error!(error = %e, "chat_layouts: db query failed"); err(StatusCode::INTERNAL_SERVER_ERROR, "server error") })?;

    let Some(row) = rows.into_iter().next() else {
        return Ok(Json(Value::Null));
    };

    Ok(Json(json!({
        "tree": parse_tree(&row),
        "updated_at": row.get("updated_at"),
    })))
}

#[derive(Deserialize)]
pub struct SaveCurrentBody {
    pub tree: Value,
}

pub async fn put_current(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Query(params): Query<ProjectScopeParams>,
    Json(body): Json<SaveCurrentBody>,
) -> Result<impl IntoResponse, ApiError> {
    let now       = harvest_db::now_rfc3339();
    let id        = Uuid::new_v4().to_string();
    let tree_json = serde_json::to_string(&body.tree)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid tree"))?;

    state.db.query(
        "INSERT INTO chat_layouts (id, user_id, kind, project_id, tree, created_at, updated_at)
         VALUES ($id, $uid, 'current', $pid, $tree, $now, $now)
         ON CONFLICT (user_id, project_id) WHERE kind = 'current'
             DO UPDATE SET tree = EXCLUDED.tree, updated_at = EXCLUDED.updated_at
         RETURNING id",
        json!({ "uid": user.sub, "id": id, "pid": scope_id(params.project_id), "tree": tree_json, "now": now }),
    ).await.map_err(|e| {
        tracing::error!(error = %e, "put_current: db query failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
    })?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn list_named(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Query(params): Query<ProjectScopeParams>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT id, name, tree, updated_at FROM chat_layouts
         WHERE user_id = $uid AND kind = 'named' AND project_id = $pid
         ORDER BY updated_at DESC",
        json!({ "uid": user.sub, "pid": scope_id(params.project_id) }),
    ).await.map_err(|e| { tracing::error!(error = %e, "chat_layouts: db query failed"); err(StatusCode::INTERNAL_SERVER_ERROR, "server error") })?;

    let layouts: Vec<Value> = rows.iter().map(|row| json!({
        "id": row.get("id"),
        "name": row.get("name"),
        "tree": parse_tree(row),
        "updated_at": row.get("updated_at"),
    })).collect();

    Ok(Json(layouts))
}

#[derive(Deserialize)]
pub struct CreateNamedBody {
    pub name: String,
    pub tree: Value,
    pub project_id: Option<String>,
}

pub async fn create_named(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Json(body): Json<CreateNamedBody>,
) -> Result<impl IntoResponse, ApiError> {
    let id        = Uuid::new_v4().to_string();
    let now       = harvest_db::now_rfc3339();
    let tree_json = serde_json::to_string(&body.tree)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid tree"))?;

    state.db.query(
        "INSERT INTO chat_layouts (id, user_id, kind, name, tree, project_id, created_at, updated_at)
         VALUES ($id, $uid, 'named', $name, $tree, $pid, $now, $now)
         RETURNING id",
        json!({
            "uid": user.sub, "id": id, "name": body.name, "tree": tree_json,
            "pid": scope_id(body.project_id), "now": now,
        }),
    ).await.map_err(|e| { tracing::error!(error = %e, "chat_layouts: db query failed"); err(StatusCode::INTERNAL_SERVER_ERROR, "server error") })?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": body.name, "created_at": now }))))
}

pub async fn get_named(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Path(layout_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT id, name, tree, created_at, updated_at FROM chat_layouts
         WHERE id = $lid AND user_id = $uid AND kind = 'named'",
        json!({ "uid": user.sub, "lid": layout_id }),
    ).await.map_err(|e| { tracing::error!(error = %e, "chat_layouts: db query failed"); err(StatusCode::INTERNAL_SERVER_ERROR, "server error") })?;

    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    Ok(Json(json!({
        "id": row.get("id"),
        "name": row.get("name"),
        "tree": parse_tree(&row),
        "created_at": row.get("created_at"),
        "updated_at": row.get("updated_at"),
    })))
}

#[derive(Deserialize)]
pub struct UpdateNamedBody {
    pub name: String,
    pub tree: Value,
}

pub async fn update_named(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Path(layout_id): Path<String>,
    Json(body): Json<UpdateNamedBody>,
) -> Result<impl IntoResponse, ApiError> {
    let now       = harvest_db::now_rfc3339();
    let tree_json = serde_json::to_string(&body.tree)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "invalid tree"))?;

    state.db.query(
        "UPDATE chat_layouts SET name = $name, tree = $tree, updated_at = $now
         WHERE id = $lid AND user_id = $uid AND kind = 'named'
         RETURNING id",
        json!({ "uid": user.sub, "lid": layout_id, "name": body.name, "tree": tree_json, "now": now }),
    ).await.map_err(|e| { tracing::error!(error = %e, "chat_layouts: db query failed"); err(StatusCode::INTERNAL_SERVER_ERROR, "server error") })?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_named(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ChatLayoutState>>,
    Path(layout_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.db.query(
        "DELETE FROM chat_layouts WHERE id = $lid AND user_id = $uid AND kind = 'named'",
        json!({ "uid": user.sub, "lid": layout_id }),
    ).await.map_err(|e| { tracing::error!(error = %e, "chat_layouts: db query failed"); err(StatusCode::INTERNAL_SERVER_ERROR, "server error") })?;

    Ok(Json(json!({ "ok": true })))
}
