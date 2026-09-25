use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::AuthState;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub async fn list_users(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT u.id, u.email, u.name, u.role, u.provider, u.created_at,
                COALESCE(array_agg(ug.group_id ORDER BY ug.group_id) FILTER (WHERE ug.group_id IS NOT NULL),
                         '{}') AS group_ids
         FROM users u LEFT JOIN user_groups ug ON ug.user_id = u.id
         GROUP BY u.id
         ORDER BY u.created_at",
        json!({}),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct SetRoleBody {
    pub role: String,
}

pub async fn set_user_role(
    State(state): State<Arc<AuthState>>,
    Path(user_id): Path<String>,
    Json(body): Json<SetRoleBody>,
) -> Result<impl IntoResponse, ApiError> {
    if body.role != "admin" && body.role != "regular" {
        return Err(err(StatusCode::BAD_REQUEST, "role must be admin or regular"));
    }
    state.db.query(
        "UPDATE users SET role = $role WHERE id = $id RETURNING id",
        json!({ "id": user_id, "role": body.role }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SetGroupsBody {
    pub group_ids: Vec<String>,
}

pub async fn set_user_groups(
    State(state): State<Arc<AuthState>>,
    Path(user_id): Path<String>,
    Json(body): Json<SetGroupsBody>,
) -> Result<impl IntoResponse, ApiError> {
    let server_error = |_: anyhow::Error| err(StatusCode::INTERNAL_SERVER_ERROR, "server error");
    let tx = state.db.begin().await.map_err(server_error)?;
    tx.execute("DELETE FROM user_groups WHERE user_id = $id", json!({ "id": user_id }))
        .await.map_err(server_error)?;
    tx.execute(
        "INSERT INTO user_groups (user_id, group_id)
         SELECT u.id, g.id FROM users u JOIN groups g ON g.id = ANY($group_ids)
         WHERE u.id = $id",
        json!({ "id": user_id, "group_ids": body.group_ids }),
    ).await.map_err(server_error)?;
    tx.commit().await.map_err(server_error)?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn list_groups(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT id, name, description, created_at, is_default
         FROM groups ORDER BY name",
        json!({}),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct CreateGroupBody {
    pub name: String,
    pub description: Option<String>,
}

pub async fn create_group(
    State(state): State<Arc<AuthState>>,
    Json(body): Json<CreateGroupBody>,
) -> Result<impl IntoResponse, ApiError> {
    if body.name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    let id = Uuid::new_v4().to_string();
    let now = harvest_db::now_rfc3339();
    let rows = state.db.query(
        "INSERT INTO groups (id, name, description, created_at, is_default)
         VALUES ($id, $name, $description, $created_at, false)
         RETURNING id, name, description, is_default",
        json!({
            "id": id,
            "name": body.name,
            "description": body.description.unwrap_or_default(),
            "created_at": now
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok((StatusCode::CREATED, Json(rows.into_iter().next().unwrap_or(json!({})))))
}

#[derive(Deserialize)]
pub struct SetGroupDefaultBody {
    pub is_default: bool,
}

pub async fn set_group_default(
    State(state): State<Arc<AuthState>>,
    Path(group_id): Path<String>,
    Json(body): Json<SetGroupDefaultBody>,
) -> Result<impl IntoResponse, ApiError> {
    state.db.query(
        "UPDATE groups SET is_default = $is_default WHERE id = $id RETURNING id",
        json!({ "id": group_id, "is_default": body.is_default }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_group(
    State(state): State<Arc<AuthState>>,
    Path(group_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    match state.db.execute("DELETE FROM groups WHERE id = $id", json!({ "id": group_id })).await {
        Ok(_) => Ok(Json(json!({ "ok": true }))),
        Err(e) if harvest_db::is_foreign_key_violation(&e) => {
            Err(err(StatusCode::CONFLICT, "group still has projects; move or delete them first"))
        }
        Err(_) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "server error")),
    }
}
