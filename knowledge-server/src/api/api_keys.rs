use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::auth::{api_keys::ApiKeyStore, jwt::Claims};

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Clone)]
pub struct ApiKeyState {
    pub store: Arc<ApiKeyStore>,
}

#[derive(Deserialize)]
pub struct CreateApiKeyBody {
    pub name: String,
}

#[derive(Deserialize)]
pub struct RenameApiKeyBody {
    pub name: String,
}

fn validate_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && trimmed.len() <= 120
}

pub async fn list_api_keys(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ApiKeyState>>,
) -> Result<impl IntoResponse, ApiError> {
    let keys = state
        .store
        .list(&user.sub)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "keys": keys })))
}

pub async fn create_api_key(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ApiKeyState>>,
    Json(body): Json<CreateApiKeyBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !validate_name(&body.name) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "name must be between 1 and 120 characters",
        ));
    }
    let created = state
        .store
        .create(&user.sub, body.name.trim())
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": created.id,
            "name": created.name,
            "key_prefix": created.key_prefix,
            "api_key": created.secret,
            "created_at": created.created_at,
        })),
    ))
}

pub async fn rename_api_key(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ApiKeyState>>,
    Path(key_id): Path<String>,
    Json(body): Json<RenameApiKeyBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !validate_name(&body.name) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "name must be between 1 and 120 characters",
        ));
    }
    let updated = state
        .store
        .rename(&user.sub, &key_id, body.name.trim())
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if !updated {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn revoke_api_key(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ApiKeyState>>,
    Path(key_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let updated = state
        .store
        .revoke(&user.sub, &key_id)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if !updated {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    Ok(Json(json!({ "ok": true })))
}
