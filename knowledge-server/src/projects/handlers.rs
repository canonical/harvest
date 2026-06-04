use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::StreamExt as _;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::{broadcast, Mutex, RwLock};
use tokio_stream::{wrappers::BroadcastStream, Stream};
use uuid::Uuid;

use crate::agent::{Agent, AgentEvent, Attachment, HistoryMessage};
use crate::auth::jwt::Claims;
use crate::neo4j::Neo4jClient;

// ── State ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ProjectState {
    pub neo4j:    Arc<Neo4jClient>,
    pub agent:    Arc<Agent>,
    /// project_id → name of user currently generating a response
    pub locks:    Arc<RwLock<HashMap<String, String>>>,
    /// project_id → broadcast sender for real-time events
    pub channels: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
    /// project_id → { user_id → UserPresence }
    pub presence: Arc<RwLock<HashMap<String, HashMap<String, UserPresence>>>>,
}

#[derive(Clone)]
pub struct UserPresence {
    pub name:    String,
    pub conv_id: Option<String>,
}

impl ProjectState {
    pub fn new(neo4j: Arc<Neo4jClient>, agent: Arc<Agent>) -> Self {
        Self {
            neo4j,
            agent,
            locks:    Arc::new(RwLock::new(HashMap::new())),
            channels: Arc::new(Mutex::new(HashMap::new())),
            presence: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn get_or_create_channel(&self, project_id: &str) -> broadcast::Sender<String> {
        let mut channels = self.channels.lock().await;
        channels
            .entry(project_id.to_string())
            .or_insert_with(|| broadcast::channel(128).0)
            .clone()
    }

    async fn broadcast(&self, project_id: &str, msg: String) {
        let channels = self.channels.lock().await;
        if let Some(tx) = channels.get(project_id) {
            let _ = tx.send(msg);
        }
    }
}

// ── Disconnect-detecting stream wrapper ───────────────────────────────────────

/// Holds a `PresenceGuard` alive until the SSE stream is dropped (client disconnects).
struct GuardedStream<S: Unpin> {
    inner: S,
    _guard: PresenceGuard,
}

impl<S: Stream + Unpin> Stream for GuardedStream<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

struct PresenceGuard {
    project_id: String,
    user_id:    String,
    user_name:  String,
    channels:   Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
    presence:   Arc<RwLock<HashMap<String, HashMap<String, UserPresence>>>>,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        let project_id = self.project_id.clone();
        let user_id    = self.user_id.clone();
        let user_name  = self.user_name.clone();
        let channels   = Arc::clone(&self.channels);
        let presence   = Arc::clone(&self.presence);
        tokio::spawn(async move {
            {
                let mut p = presence.write().await;
                if let Some(users) = p.get_mut(&project_id) {
                    users.remove(&user_id);
                }
            }
            let leave = json!({
                "type": "user_leave",
                "user_id": user_id,
                "name": user_name,
            }).to_string();
            let ch = channels.lock().await;
            if let Some(tx) = ch.get(&project_id) {
                let _ = tx.send(leave);
            }
        });
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

// ── Project events SSE ────────────────────────────────────────────────────────

pub async fn project_events(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    let conv_id = params.get("conv").cloned();

    // Ensure channel exists and subscribe before mutating presence, so we
    // don't miss the user_join we're about to send.
    let tx = state.get_or_create_channel(&project_id).await;
    let rx = tx.subscribe();

    // Add user to presence map.
    {
        let mut presence = state.presence.write().await;
        presence
            .entry(project_id.clone())
            .or_default()
            .insert(user.sub.clone(), UserPresence {
                name: user.name.clone(),
                conv_id: conv_id.clone(),
            });
    }

    // Snapshot presence and lock state for the initial burst.
    let presence_users: Vec<Value> = {
        let p = state.presence.read().await;
        p.get(&project_id)
            .map(|m| m.iter().map(|(id, up)| json!({
                "user_id": id,
                "name": up.name,
                "conv_id": up.conv_id,
            })).collect())
            .unwrap_or_default()
    };
    let lock_by = state.locks.read().await.get(&project_id).cloned();

    // Broadcast user_join to everyone already connected.
    let _ = tx.send(json!({
        "type": "user_join",
        "user_id": user.sub,
        "name": user.name,
        "conv_id": conv_id,
    }).to_string());

    // Build initial events sent only to this client.
    let mut init: Vec<Result<Event, Infallible>> = vec![
        Ok(Event::default().data(
            json!({"type": "presence", "users": presence_users}).to_string(),
        )),
    ];
    if let Some(by) = lock_by {
        init.push(Ok(Event::default().data(
            json!({"type": "lock", "by": by}).to_string(),
        )));
    }

    let guard = PresenceGuard {
        project_id: project_id.clone(),
        user_id:    user.sub.clone(),
        user_name:  user.name.clone(),
        channels:   Arc::clone(&state.channels),
        presence:   Arc::clone(&state.presence),
    };

    let broadcast_stream = BroadcastStream::new(rx).filter_map(|msg| {
        std::future::ready(
            msg.ok().map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
        )
    });

    let stream = tokio_stream::iter(init)
        .chain(GuardedStream { inner: broadcast_stream, _guard: guard });

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

// ── Project query (fire-and-forget, results via events SSE) ───────────────────

#[derive(serde::Deserialize)]
pub struct ProjectQueryBody {
    pub query: String,
    pub history: Option<Vec<HistoryMessage>>,
    pub attachments: Option<Vec<Attachment>>,
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

    // Acquire lock — reject if already locked.
    {
        let mut locks = state.locks.write().await;
        if locks.contains_key(&project_id) {
            return (StatusCode::CONFLICT, Json(json!({"error": "chat is locked"}))).into_response();
        }
        locks.insert(project_id.clone(), user.name.clone());
    }

    // Broadcast: lock + user's message.
    state.broadcast(&project_id, json!({
        "type": "lock",
        "by": user.name,
    }).to_string()).await;
    let att_for_broadcast: Vec<serde_json::Value> = body.attachments.as_ref()
        .map(|atts| atts.iter().map(|a| json!({
            "name": a.name,
            "mime_type": a.mime_type,
            "data": a.data,
            "preview_url": if a.mime_type.starts_with("image/") {
                format!("data:{};base64,{}", a.mime_type, a.data)
            } else {
                String::new()
            }
        })).collect())
        .unwrap_or_default();
    state.broadcast(&project_id, json!({
        "type": "user_message",
        "query": body.query,
        "username": user.name,
        "attachments": att_for_broadcast,
    }).to_string()).await;

    // Spawn agent task — all events go to the broadcast channel.
    let agent    = Arc::clone(&state.agent);
    let locks    = Arc::clone(&state.locks);
    let channels = Arc::clone(&state.channels);
    let pid      = project_id.clone();
    let query       = body.query.clone();
    let history     = body.history.unwrap_or_default();
    let attachments = body.attachments.unwrap_or_default();

    tokio::spawn(async move {
        let (agent_tx, mut agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let agent_clone = Arc::clone(&agent);
        tokio::spawn(async move { agent_clone.query_streaming(&query, &history, &attachments, agent_tx).await; });

        while let Some(event) = agent_rx.recv().await {
            if let Ok(data) = serde_json::to_string(&event) {
                let ch = channels.lock().await;
                if let Some(tx) = ch.get(&pid) { let _ = tx.send(data); }
            }
        }

        // Release lock and notify.
        locks.write().await.remove(&pid);
        let ch = channels.lock().await;
        if let Some(tx) = ch.get(&pid) {
            let _ = tx.send(json!({"type": "unlock"}).to_string());
        }
    });

    Json(json!({"ok": true})).into_response()
}

// ── Groups & projects ─────────────────────────────────────────────────────────

pub async fn list_my_groups(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (:User {id: $uid})-[:MEMBER_OF]->(g:Group)
         RETURN g.id AS id, g.name AS name, g.description AS description
         ORDER BY g.name",
        json!({ "uid": user.sub }),
    ).await
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

pub async fn list_projects(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.neo4j.query_read(
        "MATCH (:User {id: $uid})-[:MEMBER_OF]->(g:Group)-[:HAS_PROJECT]->(p:Project)
         RETURN p.id AS id, p.name AS name, p.description AS description,
                p.group_id AS group_id, g.name AS group_name,
                p.created_by AS created_by, p.created_at AS created_at
         ORDER BY p.created_at DESC",
        json!({ "uid": user.sub }),
    ).await
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(serde::Deserialize)]
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

#[derive(serde::Deserialize)]
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

// ── Conversations ─────────────────────────────────────────────────────────────

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

#[derive(serde::Deserialize)]
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

#[derive(serde::Deserialize)]
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

// ── Non-streaming project query (kept for internal use) ───────────────────────

pub async fn project_query(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectQueryBody>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }
    let history = body.history.as_deref().unwrap_or(&[]);
    let attachments = body.attachments.as_deref().unwrap_or(&[]);
    match state.agent.query(&body.query, history, attachments).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "project query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
