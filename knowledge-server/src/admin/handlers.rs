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

// ── Users ─────────────────────────────────────────────────────────────────────

pub async fn list_users(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (u:User)
         OPTIONAL MATCH (u)-[:MEMBER_OF]->(g:Group)
         RETURN u.id AS id, u.email AS email, u.name AS name,
                u.role AS role, u.provider AS provider, u.created_at AS created_at,
                collect(g.id) AS group_ids",
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
    state.neo4j.query_read(
        "MATCH (u:User {id: $id}) SET u.role = $role RETURN u.id AS id",
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
    // Remove all existing memberships then add new ones
    state.neo4j.query_read(
        "MATCH (u:User {id: $id})
         OPTIONAL MATCH (u)-[r:MEMBER_OF]->()
         DELETE r
         RETURN u.id AS id",
        json!({ "id": user_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    if !body.group_ids.is_empty() {
        state.neo4j.query_read(
            "MATCH (u:User {id: $id})
             UNWIND $group_ids AS gid
             MATCH (g:Group {id: gid})
             MERGE (u)-[:MEMBER_OF]->(g)
             RETURN u.id AS id",
            json!({ "id": user_id, "group_ids": body.group_ids }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    }

    Ok(Json(json!({ "ok": true })))
}

// ── Groups ────────────────────────────────────────────────────────────────────

pub async fn list_groups(
    State(state): State<Arc<AuthState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (g:Group)
         RETURN g.id AS id, g.name AS name, g.description AS description, g.created_at AS created_at
         ORDER BY g.name",
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
    let now = chrono::Utc::now().to_rfc3339();
    let rows = state.neo4j.query_read(
        "CREATE (g:Group {id: $id, name: $name, description: $description, created_at: $created_at})
         RETURN g.id AS id, g.name AS name, g.description AS description",
        json!({
            "id": id,
            "name": body.name,
            "description": body.description.unwrap_or_default(),
            "created_at": now
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok((StatusCode::CREATED, Json(rows.into_iter().next().unwrap_or(json!({})))))
}

pub async fn delete_group(
    State(state): State<Arc<AuthState>>,
    Path(group_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.neo4j.query_read(
        "MATCH (g:Group {id: $id}) DETACH DELETE g RETURN count(g) AS n",
        json!({ "id": group_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}
