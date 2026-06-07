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

use crate::agent::{Agent, AgentEvent, Attachment, HistoryMessage, Source};
use crate::api::ProjectAgentBuilder;
use crate::auth::jwt::Claims;
use crate::neo4j::Neo4jClient;

const CONVERSATION_TITLE_MAX_CHARS: usize = 60;
const CONVERSATION_TITLE_TRUNCATE_CHARS: usize = 57;
const PROJECT_NAME_MAX_CHARS: usize = 100;

#[derive(Clone)]
pub struct ProjectState {
    pub neo4j:         Arc<Neo4jClient>,
    pub agent:         Arc<Agent>,
    pub agent_builder: Arc<ProjectAgentBuilder>,
    pub locks:    Arc<RwLock<HashMap<String, String>>>,
    pub channels: Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
    pub presence: Arc<RwLock<HashMap<String, HashMap<String, UserPresence>>>>,
}

#[derive(Clone)]
pub struct UserPresence {
    pub name:    String,
    pub conv_id: Option<String>,
}

impl ProjectState {
    pub fn new(neo4j: Arc<Neo4jClient>, agent: Arc<Agent>, agent_builder: Arc<ProjectAgentBuilder>) -> Self {
        Self {
            neo4j,
            agent,
            agent_builder,
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
        if let Some(sender) = channels.get(project_id) {
            let _ = sender.send(msg);
        }
    }
}

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
                let mut presence_map = presence.write().await;
                if let Some(users) = presence_map.get_mut(&project_id) {
                    users.remove(&user_id);
                }
            }
            let leave = json!({
                "type": "user_leave",
                "user_id": user_id,
                "name": user_name,
            }).to_string();
            let channel_map = channels.lock().await;
            if let Some(sender) = channel_map.get(&project_id) {
                let _ = sender.send(leave);
            }
        });
    }
}

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub async fn require_project_access(
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

    let sender = state.get_or_create_channel(&project_id).await;
    let receiver = sender.subscribe();

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

    let presence_users: Vec<Value> = {
        let presence = state.presence.read().await;
        presence.get(&project_id)
            .map(|map| map.iter().map(|(id, up)| json!({
                "user_id": id,
                "name": up.name,
                "conv_id": up.conv_id,
            })).collect())
            .unwrap_or_default()
    };
    let lock_by = state.locks.read().await.get(&project_id).cloned();

    let _ = sender.send(json!({
        "type": "user_join",
        "user_id": user.sub,
        "name": user.name,
        "conv_id": conv_id,
    }).to_string());

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

    let broadcast_stream = BroadcastStream::new(receiver).filter_map(|msg| {
        std::future::ready(
            msg.ok().map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
        )
    });

    let stream = tokio_stream::iter(init)
        .chain(GuardedStream { inner: broadcast_stream, _guard: guard });

    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

async fn load_project_history(
    neo4j: &Neo4jClient,
    project_id: &str,
    conv_id: &str,
) -> Vec<HistoryMessage> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         RETURN c.messages AS messages",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.unwrap_or_default();

    rows.into_iter().next()
        .and_then(|r| r.get("messages").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .and_then(|s| serde_json::from_str::<Vec<HistoryMessage>>(&s).ok())
        .unwrap_or_default()
}

async fn save_project_turn(
    neo4j: &Neo4jClient,
    project_id: &str,
    conv_id: &str,
    user_text: &str,
    username: &str,
    attachments_meta: Vec<Value>,
    compacted_history: &[HistoryMessage],
    assistant_text: &str,
    sources: &[Source],
    tool_calls_made: usize,
    tool_calls: &[Value],
) {
    let now = chrono::Utc::now().to_rfc3339();
    let title = if user_text.len() > CONVERSATION_TITLE_MAX_CHARS {
        format!("{}…", &user_text[..CONVERSATION_TITLE_TRUNCATE_CHARS])
    } else {
        user_text.to_string()
    };

    let mut messages: Vec<Value> = compacted_history.iter().map(|entry| {
        json!({ "role": entry.role, "text": entry.text, "attachments": [] })
    }).collect();
    messages.push(json!({
        "role": "user",
        "text": user_text,
        "username": username,
        "attachments": attachments_meta,
    }));
    messages.push(json!({
        "role": "assistant",
        "text": assistant_text,
        "sources": sources,
        "tool_calls": tool_calls,
        "tool_calls_made": tool_calls_made,
    }));

    let messages_json = match serde_json::to_string(&messages) {
        Ok(s) => s,
        Err(e) => { tracing::error!(error=%e, "failed to serialize conversation"); return; }
    };
    let count = messages.len() as i64;

    let _ = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         SET c.title = $title, c.messages = $messages,
             c.message_count = $count, c.updated_at = $now
         RETURN c.id AS id",
        json!({
            "pid": project_id, "cid": conv_id,
            "title": title, "messages": messages_json,
            "count": count, "now": now,
        }),
    ).await;
}

#[derive(serde::Deserialize)]
pub struct ProjectQueryBody {
    pub query: String,
    pub conversation_id: Option<String>,
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

    {
        let mut locks = state.locks.write().await;
        if locks.contains_key(&project_id) {
            return (StatusCode::CONFLICT, Json(json!({"error": "chat is locked"}))).into_response();
        }
        locks.insert(project_id.clone(), user.name.clone());
    }

    let agent = state.agent_builder.build(project_id.clone());
    let raw_history = match &body.conversation_id {
        Some(conv_id) => load_project_history(&state.neo4j, &project_id, conv_id).await,
        None => vec![],
    };
    let history = agent.compact_history(&raw_history).await;

    state.broadcast(&project_id, json!({
        "type": "lock",
        "by": user.name,
    }).to_string()).await;
    let attachments_for_broadcast: Vec<serde_json::Value> = body.attachments.as_ref()
        .map(|attachments| attachments.iter().map(|a| json!({
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
        "attachments": attachments_for_broadcast,
    }).to_string()).await;

    let locks    = Arc::clone(&state.locks);
    let channels = Arc::clone(&state.channels);
    let neo4j    = Arc::clone(&state.neo4j);
    let llm      = Arc::clone(&state.agent_builder.llm);
    let registry = Arc::clone(&state.agent_builder.registry);
    let project_id_owned = project_id.clone();
    let query            = body.query.clone();
    let conv_id          = body.conversation_id.clone();
    let username         = user.name.clone();
    let attachments      = body.attachments.unwrap_or_default();
    let attachment_meta: Vec<Value> = attachments.iter()
        .map(|a| json!({ "name": a.name, "mime_type": a.mime_type }))
        .collect();

    tokio::spawn(async move {
        let (agent_event_sender, mut agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let agent_clone = Arc::clone(&agent);
        let history_for_agent = history.clone();
        let query_for_agent   = query.clone();
        tokio::spawn(async move {
            agent_clone.query_streaming(&query_for_agent, &history_for_agent, &attachments, agent_event_sender).await;
        });

        let mut tool_calls_log: Vec<Value> = Vec::new();

        while let Some(event) = agent_rx.recv().await {
            match &event {
                AgentEvent::ToolCall { name, input } => {
                    tool_calls_log.push(json!({
                        "name": name, "input": input,
                        "status": "done", "preview": null,
                    }));
                }
                AgentEvent::ToolResult { name, preview } => {
                    if let Some(tc) = tool_calls_log.iter_mut().rev()
                        .find(|tc| tc["name"] == *name && tc["preview"].is_null())
                    {
                        tc["preview"] = json!(preview);
                    }
                }
                _ => {}
            }

            if let (AgentEvent::Done { answer, sources, tool_calls_made }, Some(cid)) =
                (&event, &conv_id)
            {
                save_project_turn(
                    &neo4j, &project_id_owned, cid,
                    &query, &username, attachment_meta.clone(),
                    &history,
                    answer, sources, *tool_calls_made,
                    &tool_calls_log,
                ).await;

                let neo4j_m  = Arc::clone(&neo4j);
                let llm_m    = Arc::clone(&llm);
                let pid_m    = project_id_owned.clone();
                let query_m  = query.clone();
                let answer_m = answer.clone();
                tokio::spawn(async move {
                    super::memory_gen::maybe_generate_memory(
                        &neo4j_m, &*llm_m, &pid_m, &query_m, &answer_m,
                    ).await;
                });
            }
            let data = if let AgentEvent::ToolCall { name, input } = &event {
                if name == "run_command" || name == "run_cypher" {
                    let description = agent.describe_tool_call(name, input).await;
                    let mut v = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("description".to_string(), json!(description));
                        if name == "run_command" {
                            if let Some(h) = input["agent_id"].as_str()
                                .and_then(|id| registry.agents.get(id).map(|a| a.hostname.clone()))
                            {
                                obj.insert("hostname".to_string(), json!(h));
                            }
                        }
                    }
                    serde_json::to_string(&v).ok()
                } else {
                    serde_json::to_string(&event).ok()
                }
            } else {
                serde_json::to_string(&event).ok()
            };
            if let Some(data) = data {
                let channel_map = channels.lock().await;
                if let Some(sender) = channel_map.get(&project_id_owned) {
                    let _ = sender.send(data);
                }
            }
        }

        locks.write().await.remove(&project_id_owned);
        let channel_map = channels.lock().await;
        if let Some(sender) = channel_map.get(&project_id_owned) {
            let _ = sender.send(json!({"type": "unlock"}).to_string());
        }
    });

    Json(json!({"ok": true})).into_response()
}

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
    if name.len() > PROJECT_NAME_MAX_CHARS {
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

    let id            = Uuid::new_v4().to_string();
    let install_token = Uuid::new_v4().to_string();
    let now           = chrono::Utc::now().to_rfc3339();
    let rows = state.neo4j.query_read(
        "MATCH (g:Group {id: $gid})
         CREATE (p:Project {
             id: $id, name: $name, description: $description,
             group_id: $gid, created_by: $uid, created_at: $now,
             install_token: $install_token
         })
         CREATE (g)-[:HAS_PROJECT]->(p)
         RETURN p.id AS id, p.name AS name, p.description AS description,
                p.group_id AS group_id, g.name AS group_name,
                p.created_by AS created_by, p.created_at AS created_at",
        json!({
            "gid": body.group_id, "id": id, "name": name,
            "description": body.description, "uid": user.sub, "now": now,
            "install_token": install_token,
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
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
    }
    let mut set_clauses: Vec<&str> = Vec::new();
    if body.name.is_some()        { set_clauses.push("p.name = $name"); }
    if body.description.is_some() { set_clauses.push("p.description = $description"); }
    if !set_clauses.is_empty() {
        let cypher = format!(
            "MATCH (p:Project {{id: $pid}}) SET {} RETURN p.id AS id",
            set_clauses.join(", ")
        );
        let mut params = json!({ "pid": project_id });
        if let Some(name) = &body.name        { params["name"]        = json!(name.trim()); }
        if let Some(desc) = &body.description { params["description"] = json!(desc); }
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
    let now           = chrono::Utc::now().to_rfc3339();
    let message_count = body.messages.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let messages_json = body.messages.to_string();
    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation {id: $cid})
         SET c.title = $title, c.messages = $messages,
             c.message_count = $count, c.updated_at = $now
         RETURN c.id AS id",
        json!({
            "pid": project_id, "cid": conv_id,
            "title": body.title, "messages": messages_json,
            "count": message_count, "now": now,
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

pub async fn list_secrets(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_SECRET]->(s:ProjectSecret)
         RETURN s.name AS name, s.created_at AS created_at
         ORDER BY s.name",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(serde::Deserialize)]
pub struct UpsertSecretBody {
    pub name:  String,
    pub value: String,
}

pub async fn upsert_secret(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<UpsertSecretBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let name = body.name.trim().to_uppercase();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
    }
    if body.value.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "value cannot be empty"));
    }
    let now = chrono::Utc::now().to_rfc3339();
    state.neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         MERGE (p)-[:HAS_SECRET]->(s:ProjectSecret {name: $name})
         ON CREATE SET s.value = $value, s.created_at = $now
         ON MATCH SET s.value = $value
         RETURN s.name",
        json!({ "pid": project_id, "name": name, "value": body.value, "now": now }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((StatusCode::CREATED, Json(json!({ "ok": true }))))
}

pub async fn delete_secret(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, secret_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_SECRET]->(s:ProjectSecret {name: $name})
         DETACH DELETE s",
        json!({ "pid": project_id, "name": secret_name }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_memories(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_MEMORY]->(m:Memory)
         RETURN m.id AS id, m.title AS title,
                m.created_at AS created_at, m.updated_at AS updated_at,
                m.created_by AS created_by
         ORDER BY m.created_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(serde::Deserialize)]
pub struct CreateMemoryBody {
    pub title:   String,
    pub content: String,
}

pub async fn create_memory(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateMemoryBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let title = body.title.trim().to_string();
    if title.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "title is required"));
    }
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    state.neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         CREATE (m:Memory {
             id: $id, title: $title, content: $content,
             created_by: $uid, created_at: $now, updated_at: $now
         })
         CREATE (p)-[:HAS_MEMORY]->(m)
         RETURN m.id AS id",
        json!({
            "pid": project_id, "id": id, "title": title,
            "content": body.content, "uid": user.sub, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "title": title, "created_at": now }))))
}

pub async fn get_memory(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, memory_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_MEMORY]->(m:Memory {id: $mid})
         RETURN m.id AS id, m.title AS title, m.content AS content,
                m.created_by AS created_by,
                m.created_at AS created_at, m.updated_at AS updated_at",
        json!({ "pid": project_id, "mid": memory_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(row))
}

#[derive(serde::Deserialize)]
pub struct UpdateMemoryBody {
    pub title:   Option<String>,
    pub content: Option<String>,
}

pub async fn update_memory(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, memory_id)): Path<(String, String)>,
    Json(body): Json<UpdateMemoryBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    if let Some(ref title) = body.title {
        if title.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "title cannot be empty"));
        }
    }
    let exists = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_MEMORY]->(m:Memory {id: $mid}) RETURN 1",
        json!({ "pid": project_id, "mid": memory_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut set_clauses = vec!["m.updated_at = $now"];
    if body.title.is_some()   { set_clauses.push("m.title = $title"); }
    if body.content.is_some() { set_clauses.push("m.content = $content"); }
    let cypher = format!(
        "MATCH (:Project {{id: $pid}})-[:HAS_MEMORY]->(m:Memory {{id: $mid}}) SET {} RETURN m.id",
        set_clauses.join(", ")
    );
    let mut params = json!({ "pid": project_id, "mid": memory_id, "now": now });
    if let Some(title)   = &body.title   { params["title"]   = json!(title.trim()); }
    if let Some(content) = &body.content { params["content"] = json!(content); }
    state.neo4j.query_read(&cypher, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_memory(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, memory_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_MEMORY]->(m:Memory {id: $mid}) DETACH DELETE m",
        json!({ "pid": project_id, "mid": memory_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
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
    let agent = state.agent_builder.build(project_id.clone());
    let raw_history = match &body.conversation_id {
        Some(conv_id) => load_project_history(&state.neo4j, &project_id, conv_id).await,
        None => vec![],
    };
    let history = agent.compact_history(&raw_history).await;
    let attachments = body.attachments.as_deref().unwrap_or(&[]);
    match agent.query(&body.query, &history, attachments).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "project query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
