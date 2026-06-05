use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::agent::HistoryMessage;
use crate::auth::jwt::Claims;
use crate::neo4j::Neo4jClient;

type ApiError = (StatusCode, Json<Value>);

// ── Internal helpers for server-side history management ───────────────────────

/// Loads the message history for a user-owned conversation.
pub async fn load_user_history(
    neo4j: &Neo4jClient,
    user_id: &str,
    conv_id: &str,
) -> anyhow::Result<Vec<HistoryMessage>> {
    let rows = neo4j.query_read(
        "MATCH (:User {id: $uid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         RETURN c.messages AS messages",
        json!({ "uid": user_id, "cid": conv_id }),
    ).await?;
    parse_history_from_rows(rows)
}

/// Appends a user+assistant turn to a user-owned conversation. Creates the
/// conversation node if it does not exist yet.
pub async fn append_user_turn(
    neo4j: &Neo4jClient,
    user_id: &str,
    conv_id: &str,
    user_text: &str,
    username: &str,
    attachments_meta: &[Value],
    compacted_history: &[HistoryMessage],
    assistant_text: &str,
    sources: &[crate::agent::Source],
    tool_calls_made: usize,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let title = if user_text.len() > 60 {
        format!("{}…", &user_text[..57])
    } else {
        user_text.to_string()
    };

    // Build the new full messages array from the (possibly compacted) history
    // plus the new turn.
    let mut msgs: Vec<Value> = compacted_history.iter().map(|h| {
        json!({ "role": h.role, "text": h.text, "attachments": [] })
    }).collect();
    msgs.push(json!({
        "role": "user",
        "text": user_text,
        "username": username,
        "attachments": attachments_meta,
    }));
    msgs.push(json!({
        "role": "assistant",
        "text": assistant_text,
        "sources": sources,
        "tool_calls": [],
        "tool_calls_made": tool_calls_made,
    }));

    let messages_json = serde_json::to_string(&msgs)?;
    let count = msgs.len() as i64;

    neo4j.query_read(
        "MATCH (u:User {id: $uid})
         MERGE (u)-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         ON CREATE SET c.title = $title, c.messages = $messages,
                       c.message_count = $count,
                       c.created_at = $now, c.updated_at = $now
         ON MATCH  SET c.messages = $messages, c.message_count = $count,
                       c.updated_at = $now
         RETURN c.id AS id",
        json!({
            "uid": user_id, "cid": conv_id,
            "title": title, "messages": messages_json,
            "count": count, "now": now,
        }),
    ).await?;
    Ok(())
}

fn parse_history_from_rows(rows: Vec<Value>) -> anyhow::Result<Vec<HistoryMessage>> {
    let row = match rows.into_iter().next() {
        Some(r) => r,
        None => return Ok(vec![]),
    };
    let messages_str = match row.get("messages").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return Ok(vec![]),
    };
    let history: Vec<HistoryMessage> = serde_json::from_str(&messages_str)
        .unwrap_or_default();
    Ok(history)
}

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Clone)]
pub struct ConvState {
    pub neo4j: Arc<Neo4jClient>,
}

// ── List ──────────────────────────────────────────────────────────────────────

pub async fn list(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ConvState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (:User {id: $uid})-[:HAS_CONVERSATION]->(c:Conversation)
         RETURN c.id AS id, c.title AS title,
                c.message_count AS message_count,
                c.created_at AS created_at, c.updated_at AS updated_at
         ORDER BY c.updated_at DESC",
        json!({ "uid": user.sub }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

// ── Create ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateBody {
    pub title: Option<String>,
}

pub async fn create(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ConvState>>,
    Json(body): Json<CreateBody>,
) -> Result<impl IntoResponse, ApiError> {
    let id    = Uuid::new_v4().to_string();
    let now   = chrono::Utc::now().to_rfc3339();
    let title = body.title.unwrap_or_else(|| "New conversation".to_string());

    state.neo4j.query_read(
        "MATCH (u:User {id: $uid})
         CREATE (c:Conversation {
           id: $id, title: $title, messages: '[]',
           message_count: 0, created_at: $now, updated_at: $now
         })
         CREATE (u)-[:HAS_CONVERSATION]->(c)
         RETURN c.id AS id",
        json!({ "uid": user.sub, "id": id, "title": title, "now": now }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok((StatusCode::CREATED, Json(json!({ "id": id, "title": title, "created_at": now }))))
}

// ── Get ───────────────────────────────────────────────────────────────────────

pub async fn get(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ConvState>>,
    Path(conv_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (:User {id: $uid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         RETURN c.id AS id, c.title AS title, c.messages AS messages,
                c.created_at AS created_at, c.updated_at AS updated_at",
        json!({ "uid": user.sub, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;

    // Parse the stored messages JSON string into an actual array
    let mut obj = row.as_object().cloned().unwrap_or_default();
    if let Some(Value::String(s)) = obj.get("messages") {
        if let Ok(parsed) = serde_json::from_str::<Value>(s) {
            obj.insert("messages".to_string(), parsed);
        }
    }

    Ok(Json(Value::Object(obj)))
}

// ── Update ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateBody {
    pub title: String,
    pub messages: Value,
}

pub async fn update(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ConvState>>,
    Path(conv_id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, ApiError> {
    let now   = chrono::Utc::now().to_rfc3339();
    let count = body.messages.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let msgs  = body.messages.to_string();

    state.neo4j.query_read(
        "MATCH (:User {id: $uid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         SET c.title = $title, c.messages = $messages,
             c.message_count = $count, c.updated_at = $now
         RETURN c.id AS id",
        json!({
            "uid": user.sub, "cid": conv_id,
            "title": body.title, "messages": msgs,
            "count": count, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}

// ── Delete ────────────────────────────────────────────────────────────────────

pub async fn delete(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ConvState>>,
    Path(conv_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.neo4j.query_read(
        "MATCH (:User {id: $uid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         DETACH DELETE c RETURN count(c) AS n",
        json!({ "uid": user.sub, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}
