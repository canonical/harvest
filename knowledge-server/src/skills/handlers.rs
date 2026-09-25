use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use super::SkillStore;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub async fn list_global_skills(
    State(state): State<Arc<SkillStore>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT id, name, description, created_at, updated_at
         FROM skills WHERE project_id IS NULL
         ORDER BY name",
        json!({}),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

pub async fn get_global_skill(
    State(state): State<Arc<SkillStore>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT id, name, description, content, created_at, updated_at
         FROM skills WHERE id = $id AND project_id IS NULL",
        json!({ "id": id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(row))
}

#[derive(serde::Deserialize)]
pub struct CreateGlobalSkillBody {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

async fn global_name_taken(state: &SkillStore, name: &str, exclude_id: &str) -> Result<bool, ApiError> {
    let rows = state.db.query(
        "SELECT id FROM skills
         WHERE name = $name AND project_id IS NULL AND id <> $exclude_id
         LIMIT 1",
        json!({ "name": name, "exclude_id": exclude_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(!rows.is_empty())
}

pub async fn create_global_skill(
    State(state): State<Arc<SkillStore>>,
    Json(body): Json<CreateGlobalSkillBody>,
) -> Result<impl IntoResponse, ApiError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    if global_name_taken(&state, &name, "").await? {
        return Err(err(StatusCode::CONFLICT, "a global skill with this name already exists"));
    }
    let id  = Uuid::new_v4().to_string();
    let now = harvest_db::now_rfc3339();
    state.db.query(
        "INSERT INTO skills (id, project_id, name, description, content, created_by, created_at, updated_at)
             VALUES ($id, NULL, $name, $description, $content, 'system', $now, $now)",
        json!({
            "id": id, "name": name, "description": body.description,
            "content": body.content, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "created_at": now }))))
}

#[derive(serde::Deserialize)]
pub struct UpdateGlobalSkillBody {
    pub name:        Option<String>,
    pub description: Option<String>,
    pub content:     Option<String>,
}

pub async fn update_global_skill(
    State(state): State<Arc<SkillStore>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateGlobalSkillBody>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
        if global_name_taken(&state, name.trim(), &id).await? {
            return Err(err(StatusCode::CONFLICT, "a global skill with this name already exists"));
        }
    }
    let exists = state.db.query(
        "SELECT 1 AS ok FROM skills WHERE id = $id AND project_id IS NULL",
        json!({ "id": id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }

    let now = harvest_db::now_rfc3339();
    let params = json!({
        "id": id, "now": now,
        "name": body.name.as_deref().map(str::trim),
        "description": body.description,
        "content": body.content,
    });
    let sql = "UPDATE skills SET
                   name        = COALESCE($name::text, name),
                   description = COALESCE($description::text, description),
                   content     = COALESCE($content::text, content),
                   updated_at  = $now
               WHERE id = $id AND project_id IS NULL
               RETURNING id";
    state.db.query(sql, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_global_skill(
    State(state): State<Arc<SkillStore>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.db.query(
        "DELETE FROM skills WHERE id = $id AND project_id IS NULL",
        json!({ "id": id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}
