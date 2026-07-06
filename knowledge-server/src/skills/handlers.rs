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
    let rows = state.neo4j.query_read(
        "MATCH (s:Skill {is_global: true})
         RETURN s.id AS id, s.name AS name, s.description AS description,
                s.created_at AS created_at, s.updated_at AS updated_at
         ORDER BY s.name",
        json!({}),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

pub async fn get_global_skill(
    State(state): State<Arc<SkillStore>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (s:Skill {id: $id, is_global: true})
         RETURN s.id AS id, s.name AS name, s.description AS description, s.content AS content,
                s.created_at AS created_at, s.updated_at AS updated_at",
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
    let rows = state.neo4j.query_read(
        "MATCH (s:Skill {name: $name, is_global: true})
         WHERE s.id <> $exclude_id
         RETURN s.id AS id LIMIT 1",
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
    let now = chrono::Utc::now().to_rfc3339();
    state.neo4j.query_read(
        "CREATE (s:Skill {
             id: $id, name: $name, description: $description, content: $content,
             is_global: true, created_by: 'system', created_at: $now, updated_at: $now
         })",
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
    let exists = state.neo4j.query_read(
        "MATCH (s:Skill {id: $id, is_global: true}) RETURN 1",
        json!({ "id": id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut set_clauses = vec!["s.updated_at = $now"];
    if body.name.is_some()        { set_clauses.push("s.name = $name"); }
    if body.description.is_some() { set_clauses.push("s.description = $description"); }
    if body.content.is_some()     { set_clauses.push("s.content = $content"); }
    let cypher = format!(
        "MATCH (s:Skill {{id: $id, is_global: true}}) SET {} RETURN s.id",
        set_clauses.join(", ")
    );
    let mut params = json!({ "id": id, "now": now });
    if let Some(name)        = &body.name        { params["name"]        = json!(name.trim()); }
    if let Some(description) = &body.description { params["description"] = json!(description); }
    if let Some(content)     = &body.content     { params["content"]     = json!(content); }
    state.neo4j.query_read(&cypher, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_global_skill(
    State(state): State<Arc<SkillStore>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.neo4j.query_read(
        "MATCH (s:Skill {id: $id, is_global: true}) DETACH DELETE s",
        json!({ "id": id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}
