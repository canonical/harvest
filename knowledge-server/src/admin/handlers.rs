use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::auth::AuthState;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub async fn list_users(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state
        .db
        .query(
            "SELECT u.id, u.email, u.name, u.role, u.provider, u.created_at
         FROM users u
         ORDER BY u.created_at",
            json!({}),
        )
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
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
        return Err(err(
            StatusCode::BAD_REQUEST,
            "role must be admin or regular",
        ));
    }
    state
        .db
        .query(
            "UPDATE users SET role = $role WHERE id = $id RETURNING id",
            json!({ "id": user_id, "role": body.role }),
        )
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_conversation_moves(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = crate::conversations::moves::code_only_project_conversations(&state.db)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(Deserialize)]
pub struct MoveConversationsBody {
    pub ids: Vec<String>,
}

pub async fn move_conversations(
    State(state): State<Arc<AuthState>>,
    Json(body): Json<MoveConversationsBody>,
) -> Result<impl IntoResponse, ApiError> {
    let moved = crate::conversations::moves::move_to_authors(&state.db, &body.ids)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "moved": moved })))
}
