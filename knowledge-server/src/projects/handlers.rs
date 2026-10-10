use axum::{
    extract::{Extension, Path, Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
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
use tokio::sync::mpsc;
use tokio_stream::{wrappers::BroadcastStream, Stream};
use uuid::Uuid;

use crate::agent::{Agent, AgentEvent, Attachment, HistoryMessage, PausedTurn, PendingConfirmCall, Source, ToolResumeResult};
use super::live::{build_catchup_events, seq_filter, ProjectLive};
use crate::llm::types::{Message, ProviderSelection, Usage, UsedProvider};
use crate::conversations::summary::{effective_history, refresh as refresh_summary, StoredSummary};
use crate::conversations::title_generation::maybe_regenerate_title;
use crate::api::ProjectAgentBuilder;
use crate::auth::jwt::Claims;
use harvest_db::Db;

const PROJECT_NAME_MAX_CHARS: usize = 100;
const PAUSED_TURN_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ResolvedConfirmItem {
    content:  String,
    is_error: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PausedConfirm {
    messages:   Vec<Message>,
    iterations: usize,
    pending:    Vec<PendingConfirmCall>,
    #[serde(default)]
    resolved:   HashMap<String, ResolvedConfirmItem>,
    selection:  Option<ProviderSelection>,
    elapsed_ms: u64,
}

fn selection_from_parts(provider_id: &Option<String>, model: &Option<String>) -> Option<ProviderSelection> {
    provider_id.clone().map(|provider_id| ProviderSelection { provider_id, model: model.clone(), cache_breakpoint_index: None })
}

#[derive(Clone)]
pub struct ProjectState {
    pub db:         Arc<Db>,
    pub agent:         Arc<Agent>,
    pub agent_builder: Arc<ProjectAgentBuilder>,
    pub llm:           Arc<dyn crate::llm::LlmProvider>,
    pub llm_configs:   Arc<Vec<crate::config::LlmProviderConfig>>,
    pub system_one_config: Option<crate::config::SystemOneConfig>,
    pub user_key_store: Option<Arc<crate::auth::user_keys::UserKeyStore>>,
    pub pricing:       Arc<crate::cost::PricingTable>,
    pub collocate_registry: Arc<crate::collocate::sessions::SessionContainerRegistry>,
    pub live:          Arc<ProjectLive>,
}

impl ProjectState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Arc<Db>,
        agent: Arc<Agent>,
        agent_builder: Arc<ProjectAgentBuilder>,
        llm: Arc<dyn crate::llm::LlmProvider>,
        llm_configs: Arc<Vec<crate::config::LlmProviderConfig>>,
        system_one_config: Option<crate::config::SystemOneConfig>,
        user_key_store: Option<Arc<crate::auth::user_keys::UserKeyStore>>,
        pricing: Arc<crate::cost::PricingTable>,
        collocate_registry: Arc<crate::collocate::sessions::SessionContainerRegistry>,
    ) -> Self {
        let live = ProjectLive::standalone(Arc::clone(&db));
        Self::with_live(db, agent, agent_builder, llm, llm_configs, system_one_config, user_key_store, pricing, collocate_registry, live)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_live(
        db: Arc<Db>,
        agent: Arc<Agent>,
        agent_builder: Arc<ProjectAgentBuilder>,
        llm: Arc<dyn crate::llm::LlmProvider>,
        llm_configs: Arc<Vec<crate::config::LlmProviderConfig>>,
        system_one_config: Option<crate::config::SystemOneConfig>,
        user_key_store: Option<Arc<crate::auth::user_keys::UserKeyStore>>,
        pricing: Arc<crate::cost::PricingTable>,
        collocate_registry: Arc<crate::collocate::sessions::SessionContainerRegistry>,
        live: Arc<ProjectLive>,
    ) -> Self {
        Self {
            db,
            agent,
            agent_builder,
            llm,
            llm_configs,
            system_one_config,
            user_key_store,
            pricing,
            collocate_registry,
            live,
        }
    }

    async fn broadcast(&self, project_id: &str, msg: String) {
        self.live.broadcast(project_id, msg);
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
    connection_id: String,
    live:          Arc<ProjectLive>,
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        let connection_id = self.connection_id.clone();
        let live = Arc::clone(&self.live);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(e) = live.leave(&connection_id).await {
                    tracing::warn!(error = %e, "failed to record presence leave");
                }
            });
        }
    }
}


type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub async fn require_project_access(
    db: &Db,
    user_id: &str,
    user_role: &str,
    project_id: &str,
) -> Result<Value, ApiError> {
    let rows = db.query(
        "SELECT p.id, p.name, p.description, p.group_id, g.name AS group_name,
                p.created_by, p.created_at
         FROM projects p JOIN groups g ON g.id = p.group_id
         WHERE p.id = $pid
           AND ($role = 'admin' OR EXISTS (
                 SELECT 1 FROM user_groups ug WHERE ug.user_id = $uid AND ug.group_id = g.id))",
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
    if let Err(e) = require_project_access(&state.db, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    let conv_id = params.get("conv").cloned();
    let receiver = state.live.subscribe(&project_id);

    let (catchup, cutoff): (Vec<Result<Event, Infallible>>, u64) = match &conv_id {
        Some(cid) => match state.live.snapshot(cid).await {
            Some(snapshot) => {
                let s = &snapshot.state;
                let mut events = vec![Ok(Event::default().data(json!({
                    "type": "user_message",
                    "conv_id": cid,
                    "query": s.query,
                    "username": s.username,
                    "attachments": s.attachments,
                }).to_string()))];
                for v in build_catchup_events(cid, s) {
                    events.push(Ok(Event::default().data(v.to_string())));
                }
                (events, snapshot.seq)
            }
            None => (vec![], 0),
        },
        None => (vec![], 0),
    };

    let connection_id = Uuid::new_v4().to_string();
    if let Err(e) = state.live.join(&project_id, &connection_id, &user.sub, &user.name, conv_id.as_deref()).await {
        tracing::warn!(error = %e, "failed to record presence");
    }

    let presence_users = state.live.presence(&project_id).await.unwrap_or_default();
    let project_locks = state.live.locks(&project_id).await.unwrap_or_default();

    state.live.broadcast(&project_id, json!({
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
        connection_id,
        live: Arc::clone(&state.live),
    };

    let keep = seq_filter(conv_id.clone().unwrap_or_default(), cutoff);
    let broadcast_stream = BroadcastStream::new(receiver).filter_map(move |msg| {
        std::future::ready(
            msg.ok()
                .filter(|data| keep(data))
                .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
        )
    });

    let shutdown = state.live.shutdown_signal();
    let stream = tokio_stream::iter(init)
        .chain(GuardedStream { inner: broadcast_stream, _guard: guard })
        .take_until(shutdown);

    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

async fn load_project_messages_raw(
    db: &Db,
    project_id: &str,
    conv_id: &str,
) -> Vec<Value> {
    load_project_conversation(db, project_id, conv_id).await.0
}

async fn load_project_conversation(
    db: &Db,
    project_id: &str,
    conv_id: &str,
) -> (Vec<Value>, Option<StoredSummary>) {
    let rows = db.query(
        "SELECT messages, summary, summary_upto FROM conversations WHERE id = $cid AND project_id = $pid",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.unwrap_or_default();

    let Some(row) = rows.into_iter().next() else { return (vec![], None) };
    let messages = row.get("messages")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok())
        .unwrap_or_default();
    (messages, StoredSummary::from_row(&row))
}

fn history_messages_from_raw(raw: &[Value]) -> Vec<HistoryMessage> {
    raw.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect()
}

#[allow(clippy::too_many_arguments)]
async fn save_project_turn(
    db: &Db,
    project_id: &str,
    conv_id: &str,
    now: &str,
    user_text: &str,
    username: &str,
    attachments_meta: Vec<Value>,
    prior_messages: Vec<Value>,
    assistant_text: &str,
    sources: &[Source],
    tool_calls_made: usize,
    chain: Vec<Value>,
    question: Option<Value>,
    provider_used: Option<&UsedProvider>,
    duration_ms: u64,
    usage: &Usage,
    llm_call_count: usize,
    turn_id: &str,
    user_id: &str,
    pricing: &crate::cost::PricingTable,
    release_lock: bool,
) {
    let mut messages = prior_messages;
    messages.push(json!({
        "role": "user",
        "text": user_text,
        "username": username,
        "attachments": attachments_meta,
    }));
    let (chain, enumerations) = crate::agent::chain::split_enumerations(chain);
    let mut assistant_message = json!({
        "role": "assistant",
        "text": assistant_text,
        "sources": sources,
        "chain": chain,
        "tool_calls_made": tool_calls_made,
        "duration_ms": duration_ms,
        "usage": usage,
        "llm_call_count": llm_call_count,
        "cost_microusd": pricing.price_call(usage, provider_used.map(|p| p.kind.as_str()).unwrap_or(""), provider_used.map(|p| p.model.as_str()).unwrap_or("")),
    });
    if !enumerations.is_empty() {
        assistant_message["enumerations"] = json!(enumerations);
    }
    if let Some(question) = question {
        assistant_message["question"] = question;
    }
    if let Some(provider_used) = provider_used {
        assistant_message["provider"] = json!({
            "provider_id": provider_used.provider_id,
            "kind": provider_used.kind,
            "model": provider_used.model,
        });
    }
    messages.push(assistant_message);

    let messages_json = match serde_json::to_string(&messages) {
        Ok(s) => s,
        Err(e) => { tracing::error!(error=%e, "failed to serialize conversation"); return; }
    };
    let count = messages.len() as i64;

    let _ = db.query(
        "WITH released AS (
             DELETE FROM active_turns WHERE conv_id = $cid AND turn_id = $tid AND $release::bool
         )
         UPDATE conversations
         SET messages = $messages, message_count = $count, updated_at = $now
         WHERE id = $cid AND project_id = $pid
         RETURNING id",
        json!({
            "pid": project_id, "cid": conv_id,
            "messages": messages_json, "count": count, "now": now,
            "tid": turn_id, "release": release_lock,
        }),
    ).await;

    if let Some(record) = crate::cost::build_turn_record(
        crate::cost::CostScope::Chat, turn_id, user_id, provider_used, usage, llm_call_count, pricing,
        duration_ms, Some(project_id), Some(conv_id), None, None, None,
    ) {
        let _ = crate::cost::record_llm_call(db, &record).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn update_last_assistant_turn(
    db: &Db,
    project_id: &str,
    conv_id: &str,
    now: &str,
    assistant_text: &str,
    sources: &[Source],
    tool_calls_made: usize,
    new_chain: Vec<Value>,
    question: Option<Value>,
    provider_used: Option<&UsedProvider>,
    duration_ms: u64,
    usage: &Usage,
    llm_call_count: usize,
    turn_id: &str,
    user_id: &str,
    pricing: &crate::cost::PricingTable,
    release_lock: bool,
) {
    let mut messages = load_project_messages_raw(db, project_id, conv_id).await;
    let Some(last) = messages.last_mut() else { return; };

    let mut chain = last["chain"].as_array().cloned().unwrap_or_default();
    chain.extend(new_chain);

    last["text"] = json!(assistant_text);
    last["sources"] = json!(sources);
    last["chain"] = json!(chain);
    last["tool_calls_made"] = json!(tool_calls_made);
    last["duration_ms"] = json!(duration_ms);
    last["usage"] = json!(usage);
    last["llm_call_count"] = json!(llm_call_count);
    last["cost_microusd"] = json!(pricing.price_call(usage, provider_used.map(|p| p.kind.as_str()).unwrap_or(""), provider_used.map(|p| p.model.as_str()).unwrap_or("")));
    match question {
        Some(question) => last["question"] = question,
        None => { if let Some(obj) = last.as_object_mut() { obj.remove("question"); } }
    }
    if let Some(provider_used) = provider_used {
        last["provider"] = json!({
            "provider_id": provider_used.provider_id,
            "kind": provider_used.kind,
            "model": provider_used.model,
        });
    }

    let messages_json = match serde_json::to_string(&messages) {
        Ok(s) => s,
        Err(e) => { tracing::error!(error=%e, "failed to serialize conversation"); return; }
    };
    let count = messages.len() as i64;

    let _ = db.query(
        "WITH released AS (
             DELETE FROM active_turns WHERE conv_id = $cid AND turn_id = $tid AND $release::bool
         )
         UPDATE conversations
         SET messages = $messages, message_count = $count, updated_at = $now
         WHERE id = $cid AND project_id = $pid
         RETURNING id",
        json!({
            "pid": project_id, "cid": conv_id,
            "messages": messages_json, "count": count, "now": now,
            "tid": turn_id, "release": release_lock,
        }),
    ).await;

    if let Some(record) = crate::cost::build_turn_record(
        crate::cost::CostScope::Chat, turn_id, user_id, provider_used, usage, llm_call_count, pricing,
        duration_ms, Some(project_id), Some(conv_id), None, None, None,
    ) {
        let _ = crate::cost::record_llm_call(db, &record).await;
    }
}

async fn mark_confirm_action_statuses(
    db: &Db,
    project_id: &str,
    conv_id: &str,
    results: &[ResumeConfirmItem],
) {
    let mut messages = load_project_messages_raw(db, project_id, conv_id).await;
    let Some(last) = messages.last_mut() else { return; };
    let Some(chain) = last["chain"].as_array_mut() else { return; };
    for entry in chain.iter_mut() {
        if entry["type"] != "confirm_action" { continue; }
        let Some(id) = entry["id"].as_str() else { continue };
        if let Some(item) = results.iter().find(|r| r.tool_call_id == id) {
            entry["status"] = json!(item.status);
            entry["result_text"] = json!(item.result_text);
        }
    }

    let messages_json = match serde_json::to_string(&messages) {
        Ok(s) => s,
        Err(e) => { tracing::error!(error=%e, "failed to serialize conversation"); return; }
    };
    let count = messages.len() as i64;
    let now = harvest_db::now_rfc3339();

    let _ = db.query(
        "UPDATE conversations
         SET messages = $messages, message_count = $count, updated_at = $now
         WHERE id = $cid AND project_id = $pid
         RETURNING id",
        json!({
            "pid": project_id, "cid": conv_id,
            "messages": messages_json, "count": count, "now": now,
        }),
    ).await;
}

#[derive(serde::Deserialize)]
pub struct ProjectQueryBody {
    pub query: String,
    pub conversation_id: Option<String>,
    pub attachments: Option<Vec<Attachment>>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct ProjectQueryStreamBody {
    pub query: String,
    pub conversation_id: String,
    pub attachments: Option<Vec<Attachment>>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
}

enum TurnPersist {
    New { prior_messages: Vec<Value>, attachment_meta: Vec<Value> },
    Continuation,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn drive_turn(
    live:      Arc<ProjectLive>,
    db:        Arc<Db>,
    llm:       Arc<dyn crate::llm::LlmProvider>,
    registry:  Arc<crate::machines::MachineRegistry>,
    project_id: String,
    conv_id:    String,
    query:      String,
    username:   String,
    history:    Vec<HistoryMessage>,
    persist:    TurnPersist,
    selection:  Option<ProviderSelection>,
    mut agent_rx: mpsc::Receiver<AgentEvent>,
    paused_rx: tokio::sync::oneshot::Receiver<Option<PausedTurn>>,
    user_id:    String,
    pricing:    Arc<crate::cost::PricingTable>,
    turn_id:    String,
) {
    let mut chain_builder = crate::agent::chain::ChainBuilder::new();
    let mut pending_question: Option<Value> = None;
    let mut confirmation_requested = false;
    let mut released = false;

    while let Some(event) = agent_rx.recv().await {
        if matches!(event, AgentEvent::ConfirmAction { .. }) {
            confirmation_requested = true;
        }
        let (description, hostname) = if let AgentEvent::ToolCall { name, input } = &event {
            if name == "run_command" {
                let host = match input["agent_id"].as_str() {
                    Some(id) => registry.hostname_of(id).await,
                    None => None,
                };
                (None, host)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        match &event {
            AgentEvent::TextDelta { text } => chain_builder.text_delta(text),
            AgentEvent::ThinkingDelta { text } => chain_builder.thinking_delta(text),
            AgentEvent::Thinking { text } => chain_builder.thinking(text),
            AgentEvent::ToolCall { name, input } => {
                chain_builder.tool_call(name, input, description.as_deref(), hostname.as_deref());
            }
            AgentEvent::ToolResult { name, preview } => chain_builder.tool_result(name, preview),
            AgentEvent::Enumeration { tool, input, result } => chain_builder.enumeration(tool, input, result),
            AgentEvent::Question { question, choices } => {
                pending_question = Some(json!({ "question": question, "choices": choices }));
            }
            AgentEvent::ConfirmAction { id, name, input, description } => {
                chain_builder.confirm_action(id, name, input, description);
            }
            AgentEvent::ParallelResearchStarted { leads } => {
                chain_builder.parallel_research_started(leads);
            }
            AgentEvent::ParallelResearchLeadDone { index, iterations, preview, duration_ms } => {
                chain_builder.parallel_research_lead_done(*index, *iterations, preview, *duration_ms);
            }
            AgentEvent::ParallelResearchMergeStarted { duration_ms } => {
                chain_builder.parallel_research_merge_started(*duration_ms);
            }
            _ => {}
        }

        let seq = live.record(&conv_id, &event, description.clone(), hostname.clone()).await;

        if let Some(mut data) = turn_event_json(&event, &conv_id, description.as_deref(), hostname.as_deref(), &pricing) {
            data["seq"] = json!(seq);
            live.broadcast(&project_id, data.to_string());
        }
        if let AgentEvent::Done { answer, sources, tool_calls_made, provider_used, duration_ms, usage, llm_call_count, .. } = &event {
            let save_now = harvest_db::now_rfc3339();
            let chain = std::mem::take(&mut chain_builder).finish();
            match &persist {
                TurnPersist::New { prior_messages, attachment_meta } => {
                    save_project_turn(
                        &db, &project_id, &conv_id, &save_now,
                        &query, &username, attachment_meta.clone(),
                        prior_messages.clone(),
                        answer, sources, *tool_calls_made,
                        chain, pending_question.clone(), provider_used.as_ref(), *duration_ms,
                        usage, *llm_call_count, &turn_id, &user_id, &pricing,
                        !confirmation_requested,
                    ).await;
                }
                TurnPersist::Continuation => {
                    update_last_assistant_turn(
                        &db, &project_id, &conv_id, &save_now,
                        answer, sources, *tool_calls_made,
                        chain, pending_question.clone(), provider_used.as_ref(), *duration_ms,
                        usage, *llm_call_count, &turn_id, &user_id, &pricing,
                        !confirmation_requested,
                    ).await;
                }
            }
            live.broadcast(&project_id, json!({
                "type": "conversation_updated",
                "conv_id": conv_id,
                "updated_at": save_now,
            }).to_string());

            if !confirmation_requested && !released {
                if let Err(e) = live.finish(&conv_id, &turn_id).await {
                    tracing::warn!(error = %e, conv_id, "failed to release conversation lock");
                }
                live.broadcast(&project_id, json!({"type": "unlock", "conv_id": conv_id}).to_string());
                released = true;
            }

            let llm_t      = Arc::clone(&llm);
            let db_t    = Arc::clone(&db);
            let live_t     = Arc::clone(&live);
            let pid_t      = project_id.clone();
            let cid_t      = conv_id.clone();
            let query_t    = query.clone();
            let answer_t   = answer.clone();
            let prior_t    = history.clone();
            let count_t    = match &persist {
                TurnPersist::New { prior_messages, .. } => prior_messages.len() + 2,
                TurnPersist::Continuation => history.len() + 2,
            };
            tokio::spawn(async move {
                if let Some(new_title) = maybe_regenerate_title(
                    &db_t, &*llm_t, &cid_t, &prior_t, &query_t, &answer_t, count_t,
                ).await {
                    let data = json!({
                        "type": "title_updated",
                        "conv_id": cid_t,
                        "title": new_title,
                    }).to_string();
                    live_t.broadcast(&pid_t, data);
                }
            });
        }
    }

    if let Ok(Some(paused_turn)) = paused_rx.await {
        let paused = PausedConfirm {
            messages:   paused_turn.messages,
            iterations: paused_turn.iterations,
            pending:    paused_turn.pending,
            resolved:   HashMap::new(),
            selection,
            elapsed_ms: paused_turn.elapsed_ms,
        };
        match serde_json::to_value(&paused) {
            Ok(payload) => {
                if let Err(e) = live.save_paused(&project_id, &conv_id, &payload).await {
                    tracing::error!(error = %e, conv_id, "failed to persist paused confirmation");
                }
            }
            Err(e) => tracing::error!(error = %e, conv_id, "failed to serialize paused confirmation"),
        }
    }

    if !released {
        if let Err(e) = live.finish(&conv_id, &turn_id).await {
            tracing::warn!(error = %e, conv_id, "failed to release conversation lock");
        }
        live.broadcast(&project_id, json!({"type": "unlock", "conv_id": conv_id}).to_string());
    }
}

fn turn_event_json(
    event: &AgentEvent,
    conv_id: &str,
    description: Option<&str>,
    hostname: Option<&str>,
    pricing: &crate::cost::PricingTable,
) -> Option<Value> {
    match event {
        AgentEvent::TextDelta { text } => Some(json!({
            "type": "text_delta", "conv_id": conv_id, "text": text,
        })),
        AgentEvent::ThinkingDelta { text } => Some(json!({
            "type": "thinking_delta", "conv_id": conv_id, "text": text,
        })),
        AgentEvent::Thinking { text } => Some(json!({
            "type": "thinking", "conv_id": conv_id, "text": text,
        })),
        AgentEvent::ToolCall { name, input } => {
            let mut v = json!({
                "type": "tool_call", "conv_id": conv_id,
                "name": name, "input": input,
            });
            if let Some(d) = description { v["description"] = json!(d); }
            if let Some(h) = hostname { v["hostname"] = json!(h); }
            Some(v)
        }
        AgentEvent::ToolResult { name, preview } => Some(json!({
            "type": "tool_result", "conv_id": conv_id,
            "name": name, "preview": preview,
        })),
        AgentEvent::Phase { label } => Some(json!({
            "type": "phase", "conv_id": conv_id, "label": label,
        })),
        AgentEvent::Intent { mode } => Some(json!({
            "type": "intent", "conv_id": conv_id,
            "mode": format!("{:?}", mode).to_lowercase(),
        })),
        AgentEvent::ParallelResearchStarted { leads } => Some(json!({
            "type": "parallel_research_started", "conv_id": conv_id, "leads": leads,
        })),
        AgentEvent::ParallelResearchLeadDone { index, iterations, preview, duration_ms } => Some(json!({
            "type": "parallel_research_lead_done", "conv_id": conv_id,
            "index": index, "iterations": iterations, "preview": preview, "duration_ms": duration_ms,
        })),
        AgentEvent::ParallelResearchMergeStarted { duration_ms } => Some(json!({
            "type": "parallel_research_merge_started", "conv_id": conv_id, "duration_ms": duration_ms,
        })),
        AgentEvent::Done { answer, sources, tool_calls_made, provider_used, duration_ms, usage, llm_call_count, .. } => {
            let cost_microusd = pricing.price_call(usage, provider_used.as_ref().map(|p| p.kind.as_str()).unwrap_or(""), provider_used.as_ref().map(|p| p.model.as_str()).unwrap_or(""));
            Some(json!({
                "type": "done", "conv_id": conv_id,
                "answer": answer, "sources": sources,
                "tool_calls_made": tool_calls_made,
                "provider_used": provider_used,
                "duration_ms": duration_ms,
                "usage": usage,
                "llm_call_count": llm_call_count,
                "cost_microusd": cost_microusd,
            }))
        }
        AgentEvent::Question { question, choices } => Some(json!({
            "type": "question", "conv_id": conv_id,
            "question": question, "choices": choices,
        })),
        AgentEvent::ConfirmAction { id, name, input, description } => Some(json!({
            "type": "confirm_action", "conv_id": conv_id,
            "id": id, "name": name, "input": input, "description": description,
        })),
        AgentEvent::Error { message } => Some(json!({
            "type": "error", "conv_id": conv_id, "message": message,
        })),
        AgentEvent::TitleUpdated { title } => Some(json!({
            "type": "title_updated", "conv_id": conv_id, "title": title,
        })),
        AgentEvent::Enumeration { .. } => None,
    }
}

pub async fn project_query_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectQueryStreamBody>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.db, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    if state.live.node().is_draining() {
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "server is shutting down, retry"}))).into_response();
    }

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
    let turn_id = uuid::Uuid::new_v4().to_string();

    match state.live.try_lock(&project_id, &body.conversation_id, &turn_id, &user.name, &user.name, &body.query, &attachments_for_broadcast).await {
        Ok(true) => {}
        Ok(false) => return (StatusCode::CONFLICT, Json(json!({"error": "chat is locked"}))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to acquire conversation lock");
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "could not lock the conversation"}))).into_response();
        }
    }

    let user_llm = crate::api::resolve_user_llm(
        &state.llm, &state.llm_configs, &state.user_key_store, &user.sub,
    ).await;
    let user_system_one = crate::api::resolve_user_system_one(
        &state.system_one_config, &state.llm_configs, &state.user_key_store, &user.sub,
    ).await;
    let agent = if std::sync::Arc::ptr_eq(&user_llm, &state.agent_builder.llm) && user_system_one.is_none() {
        state.agent_builder.build_for_conversation(project_id.clone(), body.conversation_id.clone())
    } else {
        state.agent_builder.build_for_conversation_with_llm_and_system_one(
            project_id.clone(),
            body.conversation_id.clone(),
            user_llm.clone(),
            user_system_one.or_else(|| state.agent_builder.system_one.clone()),
        )
    };
    let (raw_messages, stored_summary) = load_project_conversation(&state.db, &project_id, &body.conversation_id).await;
    let raw_history = history_messages_from_raw(&raw_messages);
    let history = effective_history(&raw_history, stored_summary.as_ref());

    state.broadcast(&project_id, json!({
        "type": "lock",
        "by": user.name,
        "conv_id": body.conversation_id,
    }).to_string()).await;
    state.broadcast(&project_id, json!({
        "type": "user_message",
        "conv_id": body.conversation_id,
        "query": body.query,
        "username": user.name,
        "attachments": attachments_for_broadcast,
    }).to_string()).await;

    let live      = Arc::clone(&state.live);
    let db        = Arc::clone(&state.db);
    let llm       = Arc::clone(agent.llm());
    let registry  = Arc::clone(&state.agent_builder.registry);
    let collocate_registry = Arc::clone(&state.collocate_registry);
    let project_id_owned = project_id.clone();
    let query            = body.query.clone();
    let conv_id          = body.conversation_id.clone();
    let username         = user.name.clone();
    let selection        = selection_from_parts(&body.provider_id, &body.model);
    let attachments      = body.attachments.unwrap_or_default();
    let attachment_meta: Vec<Value> = attachments.iter()
        .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
        .collect();
    let prior_messages_for_save = raw_messages;
    let pricing = Arc::clone(&state.pricing);
    let user_id = user.sub.clone();

    let task = tokio::spawn(async move {
        let _collocate_guard = crate::collocate::sessions::SessionGuard::new(
            Arc::clone(&collocate_registry),
            project_id_owned.clone(),
            conv_id.clone(),
        );
        let (agent_event_sender, agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let (paused_tx, paused_rx) = tokio::sync::oneshot::channel();
        let agent_clone = Arc::clone(&agent);
        let history_for_agent = history.clone();
        let query_for_agent   = query.clone();
        let selection_for_agent = selection.clone();
        let _agent_task = AbortOnDrop(tokio::spawn(async move {
            let paused = agent_clone.query_streaming(&query_for_agent, &history_for_agent, &attachments, selection_for_agent.as_ref(), agent_event_sender).await;
            let _ = paused_tx.send(paused);
        }).abort_handle());

        let db_for_summary = Arc::clone(&db);
        let conv_for_summary = conv_id.clone();
        drive_turn(
            live, db, llm, registry,
            project_id_owned, conv_id, query, username, history,
            TurnPersist::New { prior_messages: prior_messages_for_save, attachment_meta },
            selection,
            agent_rx, paused_rx,
            user_id, pricing, turn_id,
        ).await;
        refresh_summary(&db_for_summary, &agent, &conv_for_summary).await;
    });
    state.live.set_abort_handle(&body.conversation_id, task.abort_handle()).await;

    Json(json!({"ok": true})).into_response()
}

struct AbortOnDrop(tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub async fn list_my_groups(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
) -> Result<impl IntoResponse, ApiError> {
    let rows = state.db.query(
        "SELECT g.id, g.name, g.description
         FROM user_groups ug JOIN groups g ON g.id = ug.group_id
         WHERE ug.user_id = $uid
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
    let rows = state.db.query(
        "SELECT p.id, p.name, p.description, p.group_id, g.name AS group_name,
                p.created_by, p.created_at
         FROM user_groups ug
         JOIN groups g   ON g.id = ug.group_id
         JOIN projects p ON p.group_id = g.id
         WHERE ug.user_id = $uid
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

    let group_rows = state.db.query(
        "SELECT id FROM groups WHERE id = $gid",
        json!({ "gid": body.group_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if group_rows.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "group not found"));
    }

    if user.role != "admin" {
        let member = state.db.query(
            "SELECT 1 AS ok FROM user_groups WHERE user_id = $uid AND group_id = $gid",
            json!({ "uid": user.sub, "gid": body.group_id }),
        ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
        if member.is_empty() {
            return Err(err(StatusCode::FORBIDDEN, "not a member of this group"));
        }
    }

    let id            = Uuid::new_v4().to_string();
    let install_token = Uuid::new_v4().to_string();
    let deployment_id = Uuid::new_v4().to_string();
    let now           = harvest_db::now_rfc3339();
    let rows = state.db.query(
        "WITH p AS (
             INSERT INTO projects (id, name, description, group_id, created_by, created_at, install_token)
             VALUES ($id, $name, $description, $gid, $uid, $now, $install_token)
             RETURNING *
         ), d AS (
             INSERT INTO deployments (id, project_id, name, environment_description, infra_state,
                                      created_by, created_at, updated_at)
             SELECT $deployment_id, p.id, p.name, '', 'none', p.created_by, p.created_at, p.created_at
             FROM p
         )
         SELECT p.id, p.name, p.description, p.group_id, g.name AS group_name, p.created_by, p.created_at
         FROM p JOIN groups g ON g.id = p.group_id",
        json!({
            "gid": body.group_id, "id": id, "name": name,
            "description": body.description, "uid": user.sub, "now": now,
            "install_token": install_token,
            "deployment_id": deployment_id,
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
    let project = require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
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
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
    }
    state.db.query(
        "UPDATE projects SET
             name        = COALESCE($name::text, name),
             description = COALESCE($description::text, description)
         WHERE id = $pid
         RETURNING id",
        json!({
            "pid": project_id,
            "name": body.name.as_deref().map(str::trim),
            "description": body.description,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    state.db.query(
        "DELETE FROM projects WHERE id = $pid",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_conversations(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let rows = state.db.query(
        "SELECT c.id, c.title, c.created_by, u.name AS created_by_name, c.message_count,
                 c.created_at, c.updated_at
          FROM conversations c
          LEFT JOIN users u ON u.id = c.created_by
          WHERE c.project_id = $pid
          ORDER BY c.updated_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|e| {
        tracing::error!(error = %e, "list_conversations: db query failed");
        err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
    })?;
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
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let id    = Uuid::new_v4().to_string();
    let now   = harvest_db::now_rfc3339();
    let title = body.title.unwrap_or_else(|| "New conversation".to_string());
    state.db.query(
        "INSERT INTO conversations (id, project_id, title, messages, message_count,
                                    created_by, created_at, updated_at)
         VALUES ($id, $pid, $title, '[]', 0, $uid, $now, $now)
         RETURNING id",
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
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let rows = state.db.query(
        "SELECT id, title, messages, created_by, created_at, updated_at
         FROM conversations WHERE id = $cid AND project_id = $pid",
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
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let exists = state.db.query(
        "SELECT 1 AS ok FROM conversations WHERE id = $cid AND project_id = $pid",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    let now           = harvest_db::now_rfc3339();
    let message_count = body.messages.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let messages_json = body.messages.to_string();
    state.db.query(
        "UPDATE conversations
         SET title = $title, messages = $messages, message_count = $count, updated_at = $now,
             summary = NULL, summary_upto = 0
         WHERE id = $cid AND project_id = $pid
         RETURNING id",
        json!({
            "pid": project_id, "cid": conv_id,
            "title": body.title, "messages": messages_json,
            "count": message_count, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(serde::Deserialize)]
pub struct ResumeConfirmItem {
    pub tool_call_id: String,
    pub status: String,
    #[serde(default)]
    pub result_text: String,
}

#[derive(serde::Deserialize)]
pub struct ResumeConfirmBody {
    pub results: Vec<ResumeConfirmItem>,
}

pub async fn resume_confirm_action(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, conv_id)): Path<(String, String)>,
    Json(body): Json<ResumeConfirmBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;

    if body.results.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "results must not be empty"));
    }

    let mut submitted: Vec<(String, ResolvedConfirmItem)> = Vec::new();
    for item in &body.results {
        submitted.push((item.tool_call_id.clone(), ResolvedConfirmItem {
            content: if !item.result_text.is_empty() {
                item.result_text.clone()
            } else if item.status == "denied" {
                "The user declined to run this action.".to_string()
            } else if item.status == "error" {
                "The action failed.".to_string()
            } else {
                "Done.".to_string()
            },
            is_error: item.status != "done",
        }));
    }

    let deadline = tokio::time::Instant::now() + PAUSED_TURN_WAIT;
    loop {
        if state.live.has_paused(&project_id, &conv_id).await
            || !state.live.is_locked(&project_id, &conv_id).await.unwrap_or(false)
            || tokio::time::Instant::now() >= deadline
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let updated = state.live.update_paused(&project_id, &conv_id, |payload| {
        let Ok(mut paused) = serde_json::from_value::<PausedConfirm>(payload.clone()) else { return false };
        for (id, resolved) in &submitted {
            if !paused.pending.iter().any(|p| &p.id == id) { continue; }
            paused.resolved.entry(id.clone()).or_insert_with(|| resolved.clone());
        }
        let ready = paused.resolved.len() >= paused.pending.len();
        if let Ok(value) = serde_json::to_value(&paused) {
            *payload = value;
        }
        ready
    }).await.map_err(|e| {
        tracing::error!(error = %e, "failed to update paused confirmation");
        err(StatusCode::INTERNAL_SERVER_ERROR, "server error")
    })?;

    let Some((payload, ready)) = updated else {
        return Err(err(StatusCode::NOT_FOUND, "no pending confirmation for this conversation"));
    };

    mark_confirm_action_statuses(&state.db, &project_id, &conv_id, &body.results).await;

    if !ready {
        return Ok(Json(json!({ "ok": true, "resumed": false })));
    }

    let Ok(paused) = serde_json::from_value::<PausedConfirm>(payload.clone()) else {
        return Err(err(StatusCode::INTERNAL_SERVER_ERROR, "corrupt paused confirmation"));
    };

    let (raw_messages, stored_summary) = load_project_conversation(&state.db, &project_id, &conv_id).await;
    let split = raw_messages.len().saturating_sub(2);
    let (prior_raw, tail_raw) = raw_messages.split_at(split);
    let prior_history = effective_history(&history_messages_from_raw(prior_raw), stored_summary.as_ref());
    let (query, username) = tail_raw.iter()
        .find(|m| m["role"] == "user")
        .map(|m| (
            m["text"].as_str().unwrap_or("").to_string(),
            m["username"].as_str().unwrap_or("").to_string(),
        ))
        .unwrap_or_default();

    let turn_id = uuid::Uuid::new_v4().to_string();
    let locked = state.live.try_lock(&project_id, &conv_id, &turn_id, &user.name, &username, &query, &[]).await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "could not lock the conversation"))?;
    if !locked {
        let _ = state.live.save_paused(&project_id, &conv_id, &payload).await;
        return Err(err(StatusCode::CONFLICT, "chat is locked"));
    }
    state.broadcast(&project_id, json!({
        "type": "lock", "by": user.name, "conv_id": conv_id,
    }).to_string()).await;

    let results: Vec<ToolResumeResult> = paused.pending.iter().filter_map(|p| {
        paused.resolved.get(&p.id).map(|r| ToolResumeResult {
            tool_call_id: p.tool_use_id.clone(),
            content:      r.content.clone(),
            is_error:     r.is_error,
        })
    }).collect();

    let user_llm = crate::api::resolve_user_llm(
        &state.llm, &state.llm_configs, &state.user_key_store, &user.sub,
    ).await;
    let user_system_one = crate::api::resolve_user_system_one(
        &state.system_one_config, &state.llm_configs, &state.user_key_store, &user.sub,
    ).await;
    let agent = if std::sync::Arc::ptr_eq(&user_llm, &state.agent_builder.llm) && user_system_one.is_none() {
        state.agent_builder.build_for_conversation(project_id.clone(), conv_id.clone())
    } else {
        state.agent_builder.build_for_conversation_with_llm_and_system_one(
            project_id.clone(),
            conv_id.clone(),
            user_llm.clone(),
            user_system_one.or_else(|| state.agent_builder.system_one.clone()),
        )
    };

    let live      = Arc::clone(&state.live);
    let db        = Arc::clone(&state.db);
    let llm       = Arc::clone(agent.llm());
    let registry  = Arc::clone(&state.agent_builder.registry);
    let collocate_registry = Arc::clone(&state.collocate_registry);
    let project_id_owned = project_id.clone();
    let conv_id_owned    = conv_id.clone();
    let pricing = Arc::clone(&state.pricing);
    let user_id = user.sub.clone();
    let selection = paused.selection.clone();

    let task = tokio::spawn(async move {
        let _collocate_guard = crate::collocate::sessions::SessionGuard::new(
            Arc::clone(&collocate_registry),
            project_id_owned.clone(),
            conv_id_owned.clone(),
        );
        let (agent_event_sender, agent_rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
        let (paused_tx, paused_rx) = tokio::sync::oneshot::channel();
        let agent_clone = Arc::clone(&agent);
        let selection_for_agent = selection.clone();
        let _agent_task = AbortOnDrop(tokio::spawn(async move {
            let out = agent_clone.resume_after_confirm(
                paused.messages, paused.iterations, results, selection_for_agent.as_ref(), agent_event_sender,
                paused.elapsed_ms,
            ).await;
            let _ = paused_tx.send(out);
        }).abort_handle());

        let db_for_summary = Arc::clone(&db);
        let conv_for_summary = conv_id_owned.clone();
        drive_turn(
            live, db, llm, registry,
            project_id_owned, conv_id_owned, query, username, prior_history,
            TurnPersist::Continuation,
            selection,
            agent_rx, paused_rx,
            user_id, pricing, turn_id,
        ).await;
        refresh_summary(&db_for_summary, &agent, &conv_for_summary).await;
    });
    state.live.set_abort_handle(&conv_id, task.abort_handle()).await;

    Ok(Json(json!({ "ok": true, "resumed": true })))
}

#[cfg(test)]
mod provider_selection_tests {
    use super::*;

    #[test]
    fn selection_from_parts_builds_selection_when_provider_id_present() {
        let selection = selection_from_parts(&Some("anthropic-main".into()), &Some("claude-sonnet-5".into()));
        let selection = selection.expect("expected Some");
        assert_eq!(selection.provider_id, "anthropic-main");
        assert_eq!(selection.model.as_deref(), Some("claude-sonnet-5"));
    }

    #[test]
    fn selection_from_parts_allows_model_to_be_absent() {
        let selection = selection_from_parts(&Some("anthropic-main".into()), &None).expect("expected Some");
        assert_eq!(selection.provider_id, "anthropic-main");
        assert_eq!(selection.model, None);
    }

    #[test]
    fn selection_from_parts_is_none_without_provider_id() {
        assert!(selection_from_parts(&None, &Some("claude-sonnet-5".into())).is_none());
        assert!(selection_from_parts(&None, &None).is_none());
    }
}

#[cfg(test)]
mod in_flight_tests {
    use super::*;
    use crate::projects::live::{record_in_flight, InFlightState};

    fn empty_state() -> InFlightState {
        InFlightState::default()
    }

    fn event_types(events: &[Value]) -> Vec<&str> {
        events.iter().map(|e| e["type"].as_str().unwrap()).collect()
    }

    #[test]
    fn catchup_preserves_interleaved_order_of_thinking_and_tool_calls() {
        let mut state = empty_state();
        record_in_flight(&mut state, &AgentEvent::Thinking { text: "a".into() }, None, None);
        record_in_flight(&mut state, &AgentEvent::ToolCall { name: "t1".into(), input: json!({}) }, None, None);
        record_in_flight(&mut state, &AgentEvent::ToolResult { name: "t1".into(), preview: "r1".into() }, None, None);
        record_in_flight(&mut state, &AgentEvent::Thinking { text: "b".into() }, None, None);
        record_in_flight(&mut state, &AgentEvent::ToolCall { name: "t2".into(), input: json!({}) }, None, None);
        record_in_flight(&mut state, &AgentEvent::ToolResult { name: "t2".into(), preview: "r2".into() }, None, None);

        let events = build_catchup_events("c1", &state);
        assert_eq!(
            event_types(&events),
            vec!["thinking", "tool_call", "tool_result", "thinking", "tool_call", "tool_result"]
        );
        assert_eq!(events[0]["text"], "a");
        assert_eq!(events[3]["text"], "b");
        assert_eq!(events[1]["name"], "t1");
        assert_eq!(events[4]["name"], "t2");
        assert_eq!(events[2]["preview"], "r1");
        assert_eq!(events[5]["preview"], "r2");
    }

    #[test]
    fn catchup_coalesces_consecutive_thinking_deltas() {
        let mut state = empty_state();
        record_in_flight(&mut state, &AgentEvent::ThinkingDelta { text: "Hello ".into() }, None, None);
        record_in_flight(&mut state, &AgentEvent::ThinkingDelta { text: "world".into() }, None, None);

        let events = build_catchup_events("c1", &state);
        assert_eq!(event_types(&events), vec!["thinking"]);
        assert_eq!(events[0]["text"], "Hello world");
    }

    #[test]
    fn catchup_coalesces_consecutive_text_deltas() {
        let mut state = empty_state();
        record_in_flight(&mut state, &AgentEvent::TextDelta { text: "foo".into() }, None, None);
        record_in_flight(&mut state, &AgentEvent::TextDelta { text: "bar".into() }, None, None);

        let events = build_catchup_events("c1", &state);
        assert_eq!(event_types(&events), vec!["text_delta"]);
        assert_eq!(events[0]["text"], "foobar");
    }

    #[test]
    fn catchup_separates_thinking_blocks_split_by_a_tool_call() {
        let mut state = empty_state();
        record_in_flight(&mut state, &AgentEvent::ThinkingDelta { text: "d1".into() }, None, None);
        record_in_flight(&mut state, &AgentEvent::ToolCall { name: "t".into(), input: json!({}) }, None, None);
        record_in_flight(&mut state, &AgentEvent::Thinking { text: "block".into() }, None, None);

        let events = build_catchup_events("c1", &state);
        assert_eq!(event_types(&events), vec!["thinking", "tool_call", "thinking"]);
        assert_eq!(events[0]["text"], "d1");
        assert_eq!(events[2]["text"], "block");
    }

    #[test]
    fn catchup_includes_description_and_hostname_on_tool_call() {
        let mut state = empty_state();
        record_in_flight(
            &mut state,
            &AgentEvent::ToolCall { name: "run_command".into(), input: json!({"agent_id": "a1"}) },
            Some("Running ls".into()),
            Some("build-box".into()),
        );

        let events = build_catchup_events("c1", &state);
        assert_eq!(events[0]["description"], "Running ls");
        assert_eq!(events[0]["hostname"], "build-box");
    }

    #[test]
    fn catchup_omits_description_and_hostname_when_absent() {
        let mut state = empty_state();
        record_in_flight(
            &mut state,
            &AgentEvent::ToolCall { name: "search".into(), input: json!({}) },
            None, None,
        );

        let events = build_catchup_events("c1", &state);
        assert!(events[0].get("description").is_none());
        assert!(events[0].get("hostname").is_none());
    }

    #[test]
    fn catchup_emits_empty_for_default_state() {
        let state = empty_state();
        let events = build_catchup_events("c1", &state);
        assert!(events.is_empty());
    }
}

#[cfg(test)]
mod drive_turn_broadcast_tests {
    use crate::machines::MachineRegistry;
    use crate::cost::PricingTable;
    use super::*;
    use async_trait::async_trait;
    use crate::llm::types::{LlmResponse, Message, ModelInfo, ToolDefinition};
    use harvest_db::test_support::TestDb;

    struct NoopProvider;
    #[async_trait]
    impl crate::llm::LlmProvider for NoopProvider {
        fn id(&self) -> &str { "noop" }
        fn kind(&self) -> &str { "noop" }
        fn default_model(&self) -> &str { "noop-model" }
        async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> { Ok(vec![]) }
        async fn chat_with(
            &self, _model: Option<&str>, _messages: &[Message], _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> { unimplemented!("not used") }
    }

    fn count(messages: &[Value], ty: &str) -> usize {
        messages.iter().filter(|m| m["type"].as_str() == Some(ty)).count()
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn broadcasts_each_chain_event_exactly_once_and_releases_the_lock() {
        let t = TestDb::new().await;
        let db = Arc::new(t.db.clone());
        let live = ProjectLive::standalone(Arc::clone(&db));
        live.node().heartbeat().await.unwrap();
        assert!(live.try_lock("proj-1", "conv-1", "turn-1", "tester", "tester", "query", &[]).await.unwrap());
        let mut rx = live.subscribe("proj-1");

        let llm: Arc<dyn crate::llm::LlmProvider> = Arc::new(NoopProvider);
        let registry = MachineRegistry::new();
        let pricing = Arc::new(PricingTable::default());
        let (agent_tx, agent_rx) = mpsc::channel::<AgentEvent>(64);
        let (paused_tx, paused_rx) = tokio::sync::oneshot::channel::<Option<PausedTurn>>();

        agent_tx.send(AgentEvent::Thinking { text: "a".into() }).await.unwrap();
        agent_tx.send(AgentEvent::ToolCall { name: "t1".into(), input: json!({}) }).await.unwrap();
        agent_tx.send(AgentEvent::ToolResult { name: "t1".into(), preview: "r1".into() }).await.unwrap();
        agent_tx.send(AgentEvent::Thinking { text: "b".into() }).await.unwrap();
        drop(agent_tx);
        drop(paused_tx);

        drive_turn(
            Arc::clone(&live), Arc::clone(&db), llm, registry,
            "proj-1".into(), "conv-1".into(), "query".into(), "tester".into(),
            vec![], TurnPersist::Continuation, None,
            agent_rx, paused_rx,
            "user-1".into(), pricing, "turn-1".into(),
        ).await;

        let mut received = Vec::new();
        while let Ok(m) = rx.try_recv() { received.push(serde_json::from_str::<Value>(&m).unwrap()); }

        assert_eq!(count(&received, "thinking"), 2, "{:?}", received);
        assert_eq!(count(&received, "tool_call"), 1, "{:?}", received);
        assert_eq!(count(&received, "tool_result"), 1, "{:?}", received);
        assert_eq!(count(&received, "unlock"), 1, "{:?}", received);
        assert_eq!(count(&received, "done"), 0);
        let seqs: Vec<u64> = received.iter().filter_map(|m| m["seq"].as_u64()).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4]);
        assert!(!live.is_locked("proj-1", "conv-1").await.unwrap());
        assert!(live.local_snapshot("conv-1").await.is_none());
    }
}

pub async fn delete_conversation(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, conv_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let rows = state.db.query(
        "SELECT created_by FROM conversations WHERE id = $cid AND project_id = $pid",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    let creator = row["created_by"].as_str().unwrap_or("");
    if user.role != "admin" && creator != user.sub {
        return Err(err(StatusCode::FORBIDDEN, "only the creator can delete this conversation"));
    }
    state.db.query(
        "DELETE FROM conversations WHERE id = $cid AND project_id = $pid",
        json!({ "pid": project_id, "cid": conv_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn list_artifacts(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let rows = state.db.query(
        "SELECT id, title, kind, created_at, updated_at, created_by
         FROM artifacts WHERE project_id = $pid
         ORDER BY created_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

#[derive(serde::Deserialize)]
pub struct CreateArtifactBody {
    pub title:   String,
    pub kind:    String,
    pub content: String,
}

pub async fn create_artifact_route(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateArtifactBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let kind = crate::artifacts::handlers::ArtifactKind::parse(&body.kind)
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "kind must be 'markdown', 'pdf', 'terraform', or 'terragrunt'"))?;
    if body.title.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "title is required"));
    }
    crate::artifacts::handlers::validate_content_for_kind(kind, &body.content)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    let result = crate::artifacts::handlers::create_artifact(
        &state.db, &project_id, kind, &body.title, &body.content, &user.sub,
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((StatusCode::CREATED, Json(result)))
}

pub async fn list_project_skills(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let rows = state.db.query(
        "SELECT id, name, description, created_at, updated_at, created_by
         FROM skills WHERE project_id = $pid
         ORDER BY created_at DESC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(rows))
}

async fn project_skill_name_taken(
    db: &Db,
    project_id: &str,
    name: &str,
    exclude_id: &str,
) -> Result<bool, ApiError> {
    let rows = db.query(
        "SELECT id FROM skills
         WHERE name = $name AND (project_id IS NULL OR project_id = $pid) AND id <> $exclude_id
         LIMIT 1",
        json!({ "pid": project_id, "name": name, "exclude_id": exclude_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(!rows.is_empty())
}

#[derive(serde::Deserialize)]
pub struct CreateSkillBody {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

pub async fn create_project_skill(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<CreateSkillBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "name is required"));
    }
    if project_skill_name_taken(&state.db, &project_id, &name, "").await? {
        return Err(err(StatusCode::CONFLICT, "a skill with this name already exists"));
    }
    let id  = Uuid::new_v4().to_string();
    let now = harvest_db::now_rfc3339();
    state.db.query(
        "INSERT INTO skills (id, project_id, name, description, content, created_by, created_at, updated_at)
         VALUES ($id, $pid, $name, $description, $content, $uid, $now, $now)
         RETURNING id",
        json!({
            "pid": project_id, "id": id, "name": name, "description": body.description,
            "content": body.content, "uid": user.sub, "now": now,
        }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok((StatusCode::CREATED, Json(json!({ "id": id, "name": name, "created_at": now }))))
}

pub async fn get_project_skill(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, skill_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    let rows = state.db.query(
        "SELECT id, name, description, content, created_by, created_at, updated_at
         FROM skills WHERE id = $sid AND project_id = $pid",
        json!({ "pid": project_id, "sid": skill_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    let row = rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))?;
    Ok(Json(row))
}

#[derive(serde::Deserialize)]
pub struct UpdateSkillBody {
    pub name:        Option<String>,
    pub description: Option<String>,
    pub content:     Option<String>,
}

pub async fn update_project_skill(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, skill_id)): Path<(String, String)>,
    Json(body): Json<UpdateSkillBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    if let Some(ref name) = body.name {
        if name.trim().is_empty() {
            return Err(err(StatusCode::BAD_REQUEST, "name cannot be empty"));
        }
        if project_skill_name_taken(&state.db, &project_id, name.trim(), &skill_id).await? {
            return Err(err(StatusCode::CONFLICT, "a skill with this name already exists"));
        }
    }
    let exists = state.db.query(
        "SELECT 1 AS ok FROM skills WHERE id = $sid AND project_id = $pid",
        json!({ "pid": project_id, "sid": skill_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    if exists.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "not found"));
    }
    let now = harvest_db::now_rfc3339();
    let params = json!({
        "pid": project_id, "sid": skill_id, "now": now,
        "name": body.name.as_deref().map(str::trim),
        "description": body.description,
        "content": body.content,
    });
    let sql = "UPDATE skills SET
                   name        = COALESCE($name::text, name),
                   description = COALESCE($description::text, description),
                   content     = COALESCE($content::text, content),
                   updated_at  = $now
               WHERE id = $sid AND project_id = $pid
               RETURNING id";
    state.db.query(sql, params)
        .await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete_project_skill(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path((project_id, skill_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.db, &user.sub, &user.role, &project_id).await?;
    state.db.query(
        "DELETE FROM skills WHERE id = $sid AND project_id = $pid",
        json!({ "pid": project_id, "sid": skill_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn project_query(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ProjectState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ProjectQueryBody>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.db, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }
    let user_llm = crate::api::resolve_user_llm(
        &state.llm, &state.llm_configs, &state.user_key_store, &user.sub,
    ).await;
    let agent = if std::sync::Arc::ptr_eq(&user_llm, &state.agent_builder.llm) {
        state.agent_builder.build_for_conversation(project_id.clone(), body.conversation_id.clone().unwrap_or_default())
    } else {
        state.agent_builder.build_for_conversation_with_llm(project_id.clone(), body.conversation_id.clone().unwrap_or_default(), user_llm.clone())
    };
    let history = match &body.conversation_id {
        Some(conv_id) => {
            let (raw, stored_summary) = load_project_conversation(&state.db, &project_id, conv_id).await;
            effective_history(&history_messages_from_raw(&raw), stored_summary.as_ref())
        }
        None => vec![],
    };
    let attachments = body.attachments.as_deref().unwrap_or(&[]);
    let selection = selection_from_parts(&body.provider_id, &body.model);
    match agent.query(&body.query, &history, attachments, selection.as_ref()).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "project query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
