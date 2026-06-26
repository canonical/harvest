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
use tokio::sync::{broadcast, mpsc, Mutex, RwLock};
use tokio_stream::{wrappers::{BroadcastStream, ReceiverStream}, Stream};
use uuid::Uuid;

use crate::agent::{Agent, AgentEvent, Attachment, HistoryMessage, Source};
use crate::api::ProjectAgentBuilder;
use crate::auth::jwt::Claims;
use crate::neo4j::Neo4jClient;
use crate::projects::task_graph::{TaskNode, collect_subgraph, compute_in_degrees, compute_dependents};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    TaskStart   { task_id: String, task_name: String },
    Thinking    { task_id: String, text: String },
    ToolCall    { task_id: String, name: String, input: Value },
    ToolResult  { task_id: String, name: String, preview: String },
    Done        { task_id: String, answer: String },
    Error       { task_id: String, message: String },
    TaskSkipped { task_id: String },
    RunDone,
}

const CONVERSATION_TITLE_MAX_CHARS: usize = 60;
const CONVERSATION_TITLE_TRUNCATE_CHARS: usize = 57;
const PROJECT_NAME_MAX_CHARS: usize = 100;

#[derive(Clone, Default)]
struct InFlightState {
    query:           String,
    username:        String,
    attachments:     Vec<Value>,
    text:            String,
    thinking_blocks: Vec<String>,
    current_thinking: String,
    tool_calls:      Vec<InFlightToolCall>,
}

#[derive(Clone)]
struct InFlightToolCall {
    name:        String,
    input:       Value,
    description: Option<String>,
    preview:     Option<String>,
    hostname:    Option<String>,
}

#[derive(Clone)]
pub struct ProjectState {
    pub neo4j:         Arc<Neo4jClient>,
    pub agent:         Arc<Agent>,
    pub agent_builder: Arc<ProjectAgentBuilder>,
    pub locks:     Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
    pub channels:  Arc<Mutex<HashMap<String, broadcast::Sender<String>>>>,
    pub presence:  Arc<RwLock<HashMap<String, HashMap<String, UserPresence>>>>,
    pub in_flight: Arc<RwLock<HashMap<String, HashMap<String, InFlightState>>>>,
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
            locks:     Arc::new(RwLock::new(HashMap::new())),
            channels:  Arc::new(Mutex::new(HashMap::new())),
            presence:  Arc::new(RwLock::new(HashMap::new())),
            in_flight: Arc::new(RwLock::new(HashMap::new())),
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

    let catchup: Vec<Result<Event, Infallible>> = if let Some(cid) = &conv_id {
        let map = state.in_flight.read().await;
        if let Some(s) = map.get(&project_id).and_then(|m| m.get(cid)) {
            let mut events = vec![];
            events.push(Ok(Event::default().data(json!({
                "type": "user_message",
                "conv_id": cid,
                "query": s.query,
                "username": s.username,
                "attachments": s.attachments,
            }).to_string())));
            for t in &s.thinking_blocks {
                events.push(Ok(Event::default().data(json!({
                    "type": "thinking", "conv_id": cid, "text": t,
                }).to_string())));
            }
            if !s.current_thinking.is_empty() {
                events.push(Ok(Event::default().data(json!({
                    "type": "thinking", "conv_id": cid, "text": s.current_thinking,
                }).to_string())));
            }
            for tc in &s.tool_calls {
                events.push(Ok(Event::default().data(json!({
                    "type": "tool_call", "conv_id": cid,
                    "name": tc.name, "input": tc.input,
                    "description": tc.description, "hostname": tc.hostname,
                }).to_string())));
                if let Some(preview) = &tc.preview {
                    events.push(Ok(Event::default().data(json!({
                        "type": "tool_result", "conv_id": cid,
                        "name": tc.name, "preview": preview,
                    }).to_string())));
                }
            }
            if !s.text.is_empty() {
                events.push(Ok(Event::default().data(json!({
                    "type": "text_delta", "conv_id": cid, "text": s.text,
                }).to_string())));
            }
            events
        } else {
            vec![]
        }
    } else {
        vec![]
    };

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
    let project_locks: Vec<(String, String)> = state.locks.read().await
        .get(&project_id)
        .map(|m| m.iter().map(|(cid, by)| (cid.clone(), by.clone())).collect())
        .unwrap_or_default();

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
    for (cid, by) in &project_locks {
        init.push(Ok(Event::default().data(
            json!({"type": "lock", "by": by, "conv_id": cid}).to_string(),
        )));
    }
    init.extend(catchup);

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
    now: &str,
    user_text: &str,
    username: &str,
    attachments_meta: Vec<Value>,
    compacted_history: &[HistoryMessage],
    assistant_text: &str,
    sources: &[Source],
    tool_calls_made: usize,
    tool_calls: &[Value],
) {
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
             c.message_count = $count, c.updated_at = $now, c.suggestions = null
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

#[derive(serde::Deserialize)]
pub struct ProjectQueryStreamBody {
    pub query: String,
    pub conversation_id: String,
    pub attachments: Option<Vec<Attachment>>,
}

pub async fn project_query_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectQueryStreamBody>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    {
        let mut locks = state.locks.write().await;
        if locks.get(&project_id).map_or(false, |m| m.contains_key(&body.conversation_id)) {
            return (StatusCode::CONFLICT, Json(json!({"error": "chat is locked"}))).into_response();
        }
        locks.entry(project_id.clone()).or_default().insert(body.conversation_id.clone(), user.name.clone());
    }

    let agent = state.agent_builder.build(project_id.clone());
    let raw_history = load_project_history(&state.neo4j, &project_id, &body.conversation_id).await;
    let history = agent.compact_history(&raw_history).await;

    state.broadcast(&project_id, json!({
        "type": "lock",
        "by": user.name,
        "conv_id": body.conversation_id,
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
        "conv_id": body.conversation_id,
        "query": body.query,
        "username": user.name,
        "attachments": attachments_for_broadcast,
    }).to_string()).await;

    let locks     = Arc::clone(&state.locks);
    let channels  = Arc::clone(&state.channels);
    let neo4j     = Arc::clone(&state.neo4j);
    let llm       = Arc::clone(&state.agent_builder.llm);
    let registry  = Arc::clone(&state.agent_builder.registry);
    let in_flight = Arc::clone(&state.in_flight);
    let project_id_owned = project_id.clone();
    let query            = body.query.clone();
    let conv_id          = body.conversation_id.clone();
    let username         = user.name.clone();
    let attachments      = body.attachments.unwrap_or_default();
    let attachment_meta: Vec<Value> = attachments.iter()
        .map(|a| json!({ "name": a.name, "mime_type": a.mime_type }))
        .collect();

    in_flight.write().await
        .entry(project_id.clone())
        .or_default()
        .insert(conv_id.clone(), InFlightState {
            query:       body.query.clone(),
            username:    user.name.clone(),
            attachments: attachments_for_broadcast,
            ..Default::default()
        });

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

            let (description, hostname) = if let AgentEvent::ToolCall { name, input } = &event {
                if name == "run_command" || name == "run_cypher" {
                    let desc = agent.describe_tool_call(name, input).await;
                    let host = if name == "run_command" {
                        input["agent_id"].as_str()
                            .and_then(|id| registry.agents.get(id).map(|a| a.hostname.clone()))
                    } else {
                        None
                    };
                    (Some(desc), host)
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

            {
                let mut map = in_flight.write().await;
                if let Some(entry) = map.get_mut(&project_id_owned).and_then(|m| m.get_mut(&conv_id)) {
                    match &event {
                        AgentEvent::ThinkingDelta { text } => entry.current_thinking.push_str(text),
                        AgentEvent::Thinking { text } => {
                            entry.thinking_blocks.push(text.clone());
                            entry.current_thinking.clear();
                        }
                        AgentEvent::TextDelta { text } => entry.text.push_str(text),
                        AgentEvent::ToolCall { name, input } => entry.tool_calls.push(InFlightToolCall {
                            name: name.clone(),
                            input: input.clone(),
                            description: description.clone(),
                            preview: None,
                            hostname: hostname.clone(),
                        }),
                        AgentEvent::ToolResult { name, preview } => {
                            if let Some(tc) = entry.tool_calls.iter_mut().rev()
                                .find(|tc| tc.name == *name && tc.preview.is_none())
                            {
                                tc.preview = Some(preview.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }

            if let AgentEvent::Done { answer, sources, tool_calls_made } = &event {
                let save_now = chrono::Utc::now().to_rfc3339();
                save_project_turn(
                    &neo4j, &project_id_owned, &conv_id, &save_now,
                    &query, &username, attachment_meta.clone(),
                    &history,
                    answer, sources, *tool_calls_made,
                    &tool_calls_log,
                ).await;
                {
                    let ch = channels.lock().await;
                    if let Some(sender) = ch.get(&project_id_owned) {
                        let _ = sender.send(json!({
                            "type": "conversation_updated",
                            "conv_id": conv_id,
                            "updated_at": save_now,
                        }).to_string());
                    }
                }

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

                let llm_s      = Arc::clone(&llm);
                let neo4j_s    = Arc::clone(&neo4j);
                let channels_s = Arc::clone(&channels);
                let pid_s      = project_id_owned.clone();
                let cid_s      = conv_id.clone();
                let query_s    = query.clone();
                let answer_s   = answer.clone();
                tokio::spawn(async move {
                    if let Some(choices) = super::suggestions::generate_suggestions(
                        &*llm_s, &query_s, &answer_s,
                    ).await {
                        let _ = neo4j_s.query_read(
                            "MATCH (:Project {id:$pid})-[:HAS_CONVERSATION]->(c:Conversation {id:$cid})
                             SET c.suggestions = $suggestions",
                            json!({ "pid": pid_s, "cid": cid_s, "suggestions": choices }),
                        ).await;
                        let data = json!({
                            "type": "suggestions",
                            "conv_id": cid_s,
                            "choices": choices,
                        }).to_string();
                        let ch = channels_s.lock().await;
                        if let Some(sender) = ch.get(&pid_s) {
                            let _ = sender.send(data);
                        }
                    }
                });
            }

            let mut v = if let AgentEvent::ToolCall { .. } = &event {
                let mut v = serde_json::to_value(&event).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    if let Some(d) = &description { obj.insert("description".to_string(), json!(d)); }
                    if let Some(h) = &hostname    { obj.insert("hostname".to_string(),    json!(h)); }
                }
                v
            } else {
                serde_json::to_value(&event).unwrap_or(Value::Null)
            };
            if let Some(obj) = v.as_object_mut() {
                obj.insert("conv_id".to_string(), json!(conv_id));
            }
            if let Ok(data) = serde_json::to_string(&v) {
                let channel_map = channels.lock().await;
                if let Some(sender) = channel_map.get(&project_id_owned) {
                    let _ = sender.send(data);
                }
            }
        }

        if let Some(m) = in_flight.write().await.get_mut(&project_id_owned) {
            m.remove(&conv_id);
        }
        if let Some(m) = locks.write().await.get_mut(&project_id_owned) {
            m.remove(&conv_id);
        }
        let channel_map = channels.lock().await;
        if let Some(sender) = channel_map.get(&project_id_owned) {
            let _ = sender.send(json!({"type": "unlock", "conv_id": conv_id}).to_string());
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
    state.broadcast(&project_id, json!({
        "type": "conversation_created",
        "conversation": {
            "id": id, "title": title,
            "created_by": user.sub, "created_by_name": user.name,
            "message_count": 0,
            "created_at": now, "updated_at": now,
        },
    }).to_string()).await;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "title": title, "created_at": now, "updated_at": now }))))
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
                c.suggestions AS suggestions, c.created_by AS created_by,
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

pub async fn list_tasks(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task)
         RETURN t.id AS id, t.name AS name, t.prompt AS prompt,
                t.status AS status, t.created_at AS created_at,
                COALESCE(t.depends_on, []) AS depends_on
         ORDER BY t.created_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(serde::Deserialize)]
pub struct CreateTaskBody {
    pub name:        String,
    pub prompt:      String,
    pub depends_on:  Option<Vec<String>>,
}

pub async fn create_task(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateTaskBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let name       = body.name.trim().to_string();
    let prompt     = body.prompt.trim().to_string();
    let depends_on = body.depends_on.unwrap_or_default();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    if prompt.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "prompt is required"));
    }

    if !depends_on.is_empty() {
        let dep_vals: Vec<Value> = depends_on.iter().map(|id| Value::String(id.clone())).collect();
        let found = state.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task)
             WHERE t.id IN $ids RETURN t.id AS id",
            json!({ "pid": project_id, "ids": dep_vals }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        if found.len() != depends_on.len() {
            return Err(err(StatusCode::BAD_REQUEST, "one or more dependency IDs not found in this project"));
        }
    }

    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let dep_vals: Vec<Value> = depends_on.iter().map(|id| Value::String(id.clone())).collect();
    state.neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         CREATE (t:Task {
             id: $id, name: $name, prompt: $prompt,
             status: 'idle', created_at: $now,
             depends_on: $depends_on
         })
         CREATE (p)-[:HAS_TASK]->(t)
         RETURN t.id AS id",
        json!({ "pid": project_id, "id": id, "name": name, "prompt": prompt,
                "now": now, "depends_on": dep_vals }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let _ = user;
    Ok((StatusCode::CREATED, Json(json!({
        "id":         id,
        "name":       name,
        "prompt":     prompt,
        "status":     "idle",
        "depends_on": depends_on,
        "created_at": now,
    }))))
}

#[derive(serde::Deserialize)]
pub struct UpdateTaskBody {
    pub name:       Option<String>,
    pub prompt:     Option<String>,
    pub depends_on: Option<Vec<String>>,
}

pub async fn update_task(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, task_id)): Path<(String, String)>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let _ = user;

    if let Some(ref deps) = body.depends_on {
        if !deps.is_empty() {
            let dep_vals: Vec<Value> = deps.iter().map(|id| Value::String(id.clone())).collect();
            let found = state.neo4j.query_read(
                "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task)
                 WHERE t.id IN $ids RETURN t.id AS id",
                json!({ "pid": project_id, "ids": dep_vals }),
            ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
            if found.len() != deps.len() {
                return Err(err(StatusCode::BAD_REQUEST, "one or more dependency IDs not found in this project"));
            }
        }
    }

    let mut sets: Vec<&str> = vec![];
    if body.name.is_some()       { sets.push("t.name = $name"); }
    if body.prompt.is_some()     { sets.push("t.prompt = $prompt"); }
    if body.depends_on.is_some() { sets.push("t.depends_on = $depends_on"); }

    if sets.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "nothing to update"));
    }

    let cypher = format!(
        "MATCH (:Project {{id: $pid}})-[:HAS_TASK]->(t:Task {{id: $tid}}) SET {} \
         RETURN t.id AS id, t.name AS name, t.prompt AS prompt, t.status AS status, \
                t.created_at AS created_at, COALESCE(t.depends_on, []) AS depends_on",
        sets.join(", ")
    );

    let dep_vals: Option<Vec<Value>> = body.depends_on.as_ref().map(|d| {
        d.iter().map(|id| Value::String(id.clone())).collect()
    });

    let rows = state.neo4j.query_read(
        &cypher,
        json!({
            "pid":        project_id,
            "tid":        task_id,
            "name":       body.name.as_deref().unwrap_or(""),
            "prompt":     body.prompt.as_deref().unwrap_or(""),
            "depends_on": dep_vals.unwrap_or_default(),
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "task not found"))
        .map(|row| Json(row))
}

pub async fn delete_task(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, task_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let _ = user;
    state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task {id: $tid}) DETACH DELETE t",
        json!({ "pid": project_id, "tid": task_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_task(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, task_id)): Path<(String, String)>,
) -> axum::response::Response {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }
    let _ = user;

    let all_rows = match state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task)
         RETURN t.id AS id, t.name AS name, t.prompt AS prompt,
                COALESCE(t.depends_on, []) AS depends_on",
        json!({ "pid": project_id }),
    ).await {
        Ok(r) => r,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response(),
    };

    if !all_rows.iter().any(|r| r["id"].as_str() == Some(&task_id)) {
        return err(StatusCode::NOT_FOUND, "task not found").into_response();
    }

    let all_tasks: Vec<TaskNode> = all_rows.iter().map(|r| TaskNode {
        id:         r["id"].as_str().unwrap_or("").to_string(),
        name:       r["name"].as_str().unwrap_or("").to_string(),
        prompt:     r["prompt"].as_str().unwrap_or("").to_string(),
        depends_on: r["depends_on"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
    }).collect();

    let subgraph = collect_subgraph(&task_id, all_tasks);

    let (sse_tx, sse_rx) = mpsc::channel::<String>(256);
    let builder = Arc::clone(&state.agent_builder);
    let neo4j   = Arc::clone(&state.neo4j);
    let pid     = project_id.clone();

    tokio::spawn(async move {
        execute_dag(subgraph, builder, neo4j, pid, sse_tx).await;
    });

    let stream = ReceiverStream::new(sse_rx)
        .map(|data| Ok::<Event, Infallible>(Event::default().data(data)));
    Sse::new(stream).keep_alive(KeepAlive::default()).into_response()
}

async fn execute_dag(
    subgraph:   Vec<TaskNode>,
    builder:    Arc<ProjectAgentBuilder>,
    neo4j:      Arc<Neo4jClient>,
    project_id: String,
    sse_tx:     mpsc::Sender<String>,
) {
    let in_deg_init = compute_in_degrees(&subgraph);
    let dependents  = compute_dependents(&subgraph);

    let by_id: std::collections::HashMap<String, TaskNode> =
        subgraph.into_iter().map(|t| (t.id.clone(), t)).collect();

    let mut in_degree = in_deg_init;
    let (done_tx, mut done_rx) = mpsc::channel::<(String, bool)>(64);
    let mut running = 0usize;

    for (id, task) in &by_id {
        if in_degree[id] == 0 {
            running += 1;
            let t       = task.clone();
            let b       = Arc::clone(&builder);
            let n       = Arc::clone(&neo4j);
            let pid     = project_id.clone();
            let sse     = sse_tx.clone();
            let done    = done_tx.clone();
            tokio::spawn(async move {
                run_single_task(t, b.build(pid.clone()), n, pid, sse, done).await;
            });
        }
    }

    while running > 0 {
        let Some((done_id, success)) = done_rx.recv().await else { break };
        running -= 1;
        if success {
            if let Some(deps_list) = dependents.get(&done_id) {
                for dep_id in deps_list {
                    if let Some(deg) = in_degree.get_mut(dep_id) {
                        *deg -= 1;
                        if *deg == 0 {
                            if let Some(task) = by_id.get(dep_id) {
                                running += 1;
                                let t    = task.clone();
                                let b    = Arc::clone(&builder);
                                let n    = Arc::clone(&neo4j);
                                let pid  = project_id.clone();
                                let sse  = sse_tx.clone();
                                let done = done_tx.clone();
                                tokio::spawn(async move {
                                    run_single_task(t, b.build(pid.clone()), n, pid, sse, done).await;
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    for (tid, deg) in &in_degree {
        if *deg > 0 {
            let event = RunEvent::TaskSkipped { task_id: tid.clone() };
            if let Ok(data) = serde_json::to_string(&event) {
                let _ = sse_tx.send(data).await;
            }
        }
    }

    if let Ok(data) = serde_json::to_string(&RunEvent::RunDone) {
        let _ = sse_tx.send(data).await;
    }
}

async fn run_single_task(
    task:       TaskNode,
    agent:      Arc<Agent>,
    neo4j:      Arc<Neo4jClient>,
    project_id: String,
    sse_tx:     mpsc::Sender<String>,
    done_tx:    mpsc::Sender<(String, bool)>,
) {
    let tid = task.id.clone();

    let start = RunEvent::TaskStart { task_id: tid.clone(), task_name: task.name.clone() };
    if let Ok(data) = serde_json::to_string(&start) { let _ = sse_tx.send(data).await; }

    let _ = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task {id: $tid}) SET t.status = 'running'",
        json!({ "pid": project_id, "tid": tid }),
    ).await;

    let (event_tx, mut event_rx) = mpsc::channel::<AgentEvent>(64);
    let agent_ref    = Arc::clone(&agent);
    let prompt_clone = task.prompt.clone();
    tokio::spawn(async move {
        agent_ref.query_streaming(&prompt_clone, &[], &[], event_tx).await;
    });

    let mut output     = String::new();
    let mut had_error  = false;
    let mut tool_calls: Vec<Value>            = Vec::new();
    let mut pending:    Option<(String, Value)> = None;
    let mut preamble_buf = String::new();

    while let Some(event) = event_rx.recv().await {
        let run_event = match event {
            AgentEvent::ThinkingDelta { text } => {
                preamble_buf.push_str(&text);
                continue;
            }
            AgentEvent::TextDelta { text } => {
                preamble_buf.push_str(&text);
                continue;
            }
            AgentEvent::ToolCall { name, input } => {
                if !preamble_buf.is_empty() {
                    let text = std::mem::take(&mut preamble_buf);
                    if let Ok(data) = serde_json::to_string(&RunEvent::Thinking {
                        task_id: tid.clone(), text,
                    }) {
                        let _ = sse_tx.send(data).await;
                    }
                }
                pending = Some((name.clone(), input.clone()));
                RunEvent::ToolCall { task_id: tid.clone(), name, input }
            }
            AgentEvent::ToolResult { name, preview } => {
                let input = if let Some((pname, pinput)) = pending.take() {
                    if pname == name { pinput } else { Value::Null }
                } else { Value::Null };
                tool_calls.push(json!({ "name": name, "input": input, "preview": preview }));
                RunEvent::ToolResult { task_id: tid.clone(), name, preview }
            }
            AgentEvent::Done { ref answer, .. } => {
                preamble_buf.clear();
                output = answer.clone();
                RunEvent::Done { task_id: tid.clone(), answer: answer.clone() }
            }
            AgentEvent::Error { ref message } => {
                had_error = true;
                RunEvent::Error { task_id: tid.clone(), message: message.clone() }
            }
            AgentEvent::Thinking { text } => {
                RunEvent::Thinking { task_id: tid.clone(), text }
            }
            AgentEvent::Question { .. } => continue,
        };
        if let Ok(data) = serde_json::to_string(&run_event) {
            let _ = sse_tx.send(data).await;
        }
    }

    let status          = if had_error { "error" } else { "done" };
    let tool_calls_json = serde_json::to_string(&tool_calls).unwrap_or_else(|_| "[]".to_string());
    let _ = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task {id: $tid})
         SET t.status = $status, t.output = $output, t.tool_calls = $tool_calls",
        json!({ "pid": project_id, "tid": tid, "status": status,
                "output": output, "tool_calls": tool_calls_json }),
    ).await;

    let _ = done_tx.send((tid, !had_error)).await;
}

pub async fn get_task_logs(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, task_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;
    let _ = user;
    let rows = state.neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_TASK]->(t:Task {id: $tid})
         RETURN t.output AS output, t.status AS status,
                COALESCE(t.tool_calls, '[]') AS tool_calls",
        json!({ "pid": project_id, "tid": task_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "task not found"))?;
    Ok(Json(json!({
        "status":     row["status"],
        "output":     row.get("output").cloned().unwrap_or(Value::Null),
        "tool_calls": row.get("tool_calls").cloned().unwrap_or(Value::String("[]".into())),
    })))
}
