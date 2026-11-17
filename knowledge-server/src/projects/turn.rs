use crate::modes::Space;
use crate::toolsets::TurnContext;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use harvest_db::Db;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::handlers::ProjectState;
use super::live::ProjectLive;
use crate::agent::{
    Agent, AgentEvent, Attachment, HistoryMessage, PausedTurn, PendingConfirmCall, Source,
    ToolResumeResult,
};
use crate::conversations::store::{self, ConversationContext, Guard, Owner};
use crate::conversations::summary::refresh as refresh_summary;
use crate::conversations::title_generation::maybe_regenerate_title;
use crate::llm::types::{CompletedCall, Message, ProviderSelection, Usage, UsedProvider};
use crate::turns::metrics::{Stage, TURNS};
use tracing::Instrument as _;

const CANCEL_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ResolvedConfirmItem {
    pub content: String,
    pub is_error: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PausedConfirm {
    pub messages: Vec<Message>,
    pub iterations: usize,
    pub pending: Vec<PendingConfirmCall>,
    #[serde(default)]
    pub resolved: HashMap<String, ResolvedConfirmItem>,
    pub selection: Option<ProviderSelection>,
    pub elapsed_ms: u64,
}

pub(super) struct TurnLaunch {
    pub project_id: String,
    pub conv_id: String,
    pub turn_id: String,
    pub user_id: String,
    pub username: String,
    pub query: String,
    pub history: Vec<HistoryMessage>,
    pub message_count: usize,
    pub selection: Option<ProviderSelection>,
    pub work: TurnWork,
}

pub(super) enum TurnWork {
    Ask {
        attachments: Vec<Attachment>,
    },
    Resume {
        messages: Vec<Message>,
        iterations: usize,
        results: Vec<ToolResumeResult>,
        elapsed_ms: u64,
    },
}

impl TurnWork {
    fn persistence(&self) -> TurnPersist {
        match self {
            TurnWork::Ask { attachments } => TurnPersist::New {
                attachment_meta: attachments
                    .iter()
                    .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
                    .collect(),
            },
            TurnWork::Resume { .. } => TurnPersist::Continuation,
        }
    }

    async fn run(
        self,
        agent: &Agent,
        query: &str,
        history: &[HistoryMessage],
        selection: Option<&ProviderSelection>,
        events: mpsc::Sender<AgentEvent>,
    ) -> Option<PausedTurn> {
        match self {
            TurnWork::Ask { attachments } => {
                agent
                    .query_streaming(query, history, &attachments, selection, events)
                    .await
            }
            TurnWork::Resume {
                messages,
                iterations,
                results,
                elapsed_ms,
            } => {
                agent
                    .resume_after_confirm(
                        messages, iterations, results, selection, events, elapsed_ms,
                    )
                    .await
            }
        }
    }
}

struct CancelOnDrop(tokio::task::AbortHandle);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let handle = self.0.clone();
        tokio::spawn(async move {
            tokio::time::sleep(CANCEL_GRACE).await;
            handle.abort();
        });
    }
}

pub(super) async fn agent_for(
    state: &ProjectState,
    user_id: &str,
    project_id: &str,
    conv_id: &str,
) -> Arc<Agent> {
    let user_llm = crate::api::resolve_user_llm(
        &state.llm,
        &state.llm_configs,
        &state.user_key_store,
        user_id,
    )
    .await;
    let user_system_one = crate::api::resolve_user_system_one(
        &state.system_one_config,
        &state.llm_configs,
        &state.user_key_store,
        user_id,
    )
    .await;
    let builder = &state.agent_builder;
    let turn = TurnContext::load(
        &state.db,
        user_id,
        Space::deploy_project(project_id),
        conv_id,
        user_llm,
    )
    .await;
    builder.conversation_agent(
        &turn,
        user_system_one.or_else(|| builder.system_one.clone()),
    )
}

impl TurnLaunch {
    pub(super) async fn spawn(self, state: &ProjectState) {
        let agent = agent_for(state, &self.user_id, &self.project_id, &self.conv_id).await;
        let live = Arc::clone(&state.live);
        let db = Arc::clone(&state.db);
        let registry = Arc::clone(&state.agent_builder.registry);
        let collocate_registry = Arc::clone(&state.collocate_registry);
        let pricing = Arc::clone(&state.pricing);
        let shadow = state
            .agent_builder
            .search
            .clone()
            .filter(|s| s.mode() == crate::config::SearchMode::Shadow);
        let conv_id = self.conv_id.clone();
        let span = tracing::info_span!(
            "turn",
            turn_id = %self.turn_id,
            conv_id = %self.conv_id,
            project_id = %self.project_id,
            node_id = %state.live.node().node_id(),
        );
        let task = tokio::spawn(
            async move {
                let _collocate_guard = crate::collocate::sessions::SessionGuard::new(
                    collocate_registry,
                    self.project_id.clone(),
                    self.conv_id.clone(),
                );
                let (events, agent_rx) = mpsc::channel::<AgentEvent>(64);
                let (paused_tx, paused_rx) = tokio::sync::oneshot::channel();
                let persist = self.work.persistence();
                let worker = {
                    let agent = Arc::clone(&agent);
                    let query = self.query.clone();
                    let history = self.history.clone();
                    let selection = self.selection.clone();
                    let work = self.work;
                    tokio::spawn(async move {
                        let paused = work
                            .run(&agent, &query, &history, selection.as_ref(), events)
                            .await;
                        let _ = paused_tx.send(paused);
                    })
                };
                let _worker = CancelOnDrop(worker.abort_handle());
                let agent_rx = match shadow {
                    Some(service) => crate::search::shadow::tap(
                        agent_rx,
                        service,
                        self.query.clone(),
                        self.turn_id.clone(),
                        self.conv_id.clone(),
                    ),
                    None => agent_rx,
                };
                drive_turn(
                    live,
                    Arc::clone(&db),
                    Arc::clone(agent.llm()),
                    registry,
                    self.project_id,
                    self.conv_id.clone(),
                    self.query,
                    self.username,
                    self.history,
                    self.message_count,
                    persist,
                    self.selection,
                    agent_rx,
                    paused_rx,
                    self.user_id,
                    pricing,
                    self.turn_id,
                )
                .await;
                refresh_summary(&db, &agent, &self.conv_id).await;
            }
            .instrument(span),
        );
        state
            .live
            .set_abort_handle(&conv_id, task.abort_handle())
            .await;
    }
}

pub(super) async fn load_project_context(
    db: &Db,
    project_id: &str,
    conv_id: &str,
) -> ConversationContext {
    store::load_context(db, Owner::Project(project_id), conv_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, conv_id, "could not load conversation context");
            ConversationContext::default()
        })
}

fn provider_json(provider_used: &UsedProvider) -> Value {
    json!({
        "provider_id": provider_used.provider_id,
        "kind": provider_used.kind,
        "model": provider_used.model,
    })
}

struct TurnOutcome<'a> {
    answer: &'a str,
    sources: &'a [Source],
    tool_calls_made: usize,
    chain: Vec<Value>,
    question: Option<Value>,
    provider_used: Option<&'a UsedProvider>,
    duration_ms: u64,
    usage: &'a Usage,
    llm_call_count: usize,
    calls: Vec<CompletedCall>,
}

impl TurnOutcome<'_> {
    fn cost_records(
        &self,
        project_id: &str,
        conv_id: &str,
        turn_id: &str,
        user_id: &str,
        pricing: &crate::cost::PricingTable,
    ) -> Vec<crate::cost::LlmCallRecord> {
        crate::cost::build_turn_records(
            crate::cost::CostScope::Chat,
            &crate::cost::RecordIds {
                turn_id,
                user_id,
                project_id: Some(project_id),
                conversation_id: Some(conv_id),
                deployment_id: None,
                proposal_id: None,
                artifact_id: None,
            },
            &crate::cost::TurnUsage {
                provider_used: self.provider_used,
                usage: self.usage,
                llm_call_count: self.llm_call_count,
                duration_ms: self.duration_ms,
                calls: &self.calls,
            },
            pricing,
        )
    }
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
    turn: TurnOutcome<'_>,
    turn_id: &str,
    user_id: &str,
    pricing: &crate::cost::PricingTable,
) {
    let records = turn.cost_records(project_id, conv_id, turn_id, user_id, pricing);
    let cost_microusd: i64 = records.iter().map(|r| r.cost_microusd).sum();
    let (chain, enumerations) = crate::agent::chain::split_enumerations(turn.chain.clone());
    let mut assistant_message = json!({
        "role": "assistant",
        "text": turn.answer,
        "sources": turn.sources,
        "chain": chain,
        "tool_calls_made": turn.tool_calls_made,
        "duration_ms": turn.duration_ms,
        "usage": turn.usage,
        "llm_call_count": turn.llm_call_count,
        "cost_microusd": cost_microusd,
    });
    if !enumerations.is_empty() {
        assistant_message["enumerations"] = json!(enumerations);
    }
    if let Some(question) = &turn.question {
        assistant_message["question"] = question.clone();
    }
    if let Some(provider_used) = turn.provider_used {
        assistant_message["provider"] = provider_json(provider_used);
    }
    let messages = [
        json!({
            "role": "user",
            "text": user_text,
            "username": username,
            "attachments": attachments_meta,
        }),
        assistant_message,
    ];
    if let Err(e) = store::append(
        db,
        Owner::Project(project_id),
        Guard::HoldingTurn(turn_id),
        conv_id,
        &messages,
        now,
    )
    .await
    {
        tracing::error!(error = %e, conv_id, "failed to save the turn");
    }
    let _ = crate::cost::record_llm_calls(db, &records).await;
}

#[allow(clippy::too_many_arguments)]
async fn update_last_assistant_turn(
    db: &Db,
    project_id: &str,
    conv_id: &str,
    now: &str,
    turn: TurnOutcome<'_>,
    turn_id: &str,
    user_id: &str,
    pricing: &crate::cost::PricingTable,
) {
    let records = turn.cost_records(project_id, conv_id, turn_id, user_id, pricing);
    let cost_microusd: i64 = records.iter().map(|r| r.cost_microusd).sum();
    let edited = store::update_last(
        db,
        Owner::Project(project_id),
        Guard::HoldingTurn(turn_id),
        conv_id,
        now,
        |last| {
            let mut chain = last["chain"].as_array().cloned().unwrap_or_default();
            chain.extend(turn.chain.iter().cloned());
            last["text"] = json!(turn.answer);
            last["sources"] = json!(turn.sources);
            last["chain"] = json!(chain);
            last["tool_calls_made"] = json!(turn.tool_calls_made);
            last["duration_ms"] = json!(turn.duration_ms);
            last["usage"] = json!(turn.usage);
            last["llm_call_count"] = json!(turn.llm_call_count);
            last["cost_microusd"] = json!(cost_microusd);
            match &turn.question {
                Some(question) => last["question"] = question.clone(),
                None => {
                    if let Some(obj) = last.as_object_mut() {
                        obj.remove("question");
                    }
                }
            }
            if let Some(provider_used) = turn.provider_used {
                last["provider"] = provider_json(provider_used);
            }
        },
    )
    .await;
    if let Err(e) = edited {
        tracing::error!(error = %e, conv_id, "failed to update the continued turn");
    }
    let _ = crate::cost::record_llm_calls(db, &records).await;
}

pub(super) enum TurnPersist {
    New { attachment_meta: Vec<Value> },
    Continuation,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
pub(super) async fn drive_turn(
    live: Arc<ProjectLive>,
    db: Arc<Db>,
    llm: Arc<dyn crate::llm::LlmProvider>,
    registry: Arc<crate::machines::MachineRegistry>,
    project_id: String,
    conv_id: String,
    query: String,
    username: String,
    history: Vec<HistoryMessage>,
    message_count: usize,
    persist: TurnPersist,
    selection: Option<ProviderSelection>,
    mut agent_rx: mpsc::Receiver<AgentEvent>,
    paused_rx: tokio::sync::oneshot::Receiver<Option<PausedTurn>>,
    user_id: String,
    pricing: Arc<crate::cost::PricingTable>,
    turn_id: String,
) {
    let started_at = std::time::Instant::now();
    let mut chain_builder = crate::agent::chain::ChainBuilder::new();
    let mut pending_question: Option<Value> = None;
    let mut confirmation_requested = false;
    let mut released = false;
    let mut answered = false;
    let mut calls: Vec<CompletedCall> = Vec::new();

    while let Some(event) = agent_rx.recv().await {
        if let AgentEvent::LlmCall(call) = event {
            calls.push(call);
            continue;
        }
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
            AgentEvent::TextDelta { text } => {
                if !answered {
                    answered = true;
                    TURNS.stage(Stage::FirstToken, started_at.elapsed());
                }
                chain_builder.text_delta(text)
            }
            AgentEvent::ThinkingDelta { text } => chain_builder.thinking_delta(text),
            AgentEvent::Thinking { text } => chain_builder.thinking(text),
            AgentEvent::ToolCall { name, input } => {
                chain_builder.tool_call(name, input, description.as_deref(), hostname.as_deref());
            }
            AgentEvent::ToolResult {
                name,
                preview,
                view,
            } => chain_builder.tool_result(name, preview, view.as_ref()),
            AgentEvent::Enumeration {
                tool,
                input,
                result,
            } => chain_builder.enumeration(tool, input, result),
            AgentEvent::Question { question, choices } => {
                pending_question = Some(json!({ "question": question, "choices": choices }));
            }
            AgentEvent::ConfirmAction {
                id,
                name,
                input,
                description,
            } => {
                chain_builder.confirm_action(id, name, input, description);
            }
            AgentEvent::ParallelResearchStarted { leads } => {
                chain_builder.parallel_research_started(leads);
            }
            AgentEvent::ParallelResearchLeadDone {
                index,
                iterations,
                preview,
                duration_ms,
            } => {
                chain_builder.parallel_research_lead_done(
                    *index,
                    *iterations,
                    preview,
                    *duration_ms,
                );
            }
            AgentEvent::ParallelResearchMergeStarted { duration_ms } => {
                chain_builder.parallel_research_merge_started(*duration_ms);
            }
            _ => {}
        }

        let seq = live
            .record(&conv_id, &event, description.clone(), hostname.clone())
            .await;

        if let Some(mut data) = turn_event_json(
            &event,
            &conv_id,
            description.as_deref(),
            hostname.as_deref(),
            &pricing,
        ) {
            data["seq"] = json!(seq);
            live.broadcast(&project_id, data.to_string());
        }
        if let AgentEvent::Done {
            answer,
            sources,
            tool_calls_made,
            provider_used,
            duration_ms,
            usage,
            llm_call_count,
            ..
        } = &event
        {
            TURNS.completed(started_at.elapsed());
            let save_now = harvest_db::now_rfc3339();
            let outcome = TurnOutcome {
                answer,
                sources,
                tool_calls_made: *tool_calls_made,
                chain: std::mem::take(&mut chain_builder).finish(),
                question: pending_question.clone(),
                provider_used: provider_used.as_ref(),
                duration_ms: *duration_ms,
                usage,
                llm_call_count: *llm_call_count,
                calls: std::mem::take(&mut calls),
            };
            let persisting = std::time::Instant::now();
            match &persist {
                TurnPersist::New { attachment_meta } => {
                    save_project_turn(
                        &db,
                        &project_id,
                        &conv_id,
                        &save_now,
                        &query,
                        &username,
                        attachment_meta.clone(),
                        outcome,
                        &turn_id,
                        &user_id,
                        &pricing,
                    )
                    .await;
                }
                TurnPersist::Continuation => {
                    update_last_assistant_turn(
                        &db,
                        &project_id,
                        &conv_id,
                        &save_now,
                        outcome,
                        &turn_id,
                        &user_id,
                        &pricing,
                    )
                    .await;
                }
            }
            TURNS.stage(Stage::Persist, persisting.elapsed());
            live.broadcast(
                &project_id,
                json!({
                    "type": "conversation_updated",
                    "conv_id": conv_id,
                    "updated_at": save_now,
                })
                .to_string(),
            );

            if !confirmation_requested && !released {
                if let Err(e) = live.finish(&conv_id, &turn_id).await {
                    tracing::warn!(error = %e, conv_id, "failed to release conversation lock");
                }
                live.broadcast(
                    &project_id,
                    json!({"type": "unlock", "conv_id": conv_id}).to_string(),
                );
                released = true;
            }

            let llm_t = Arc::clone(&llm);
            let db_t = Arc::clone(&db);
            let live_t = Arc::clone(&live);
            let pid_t = project_id.clone();
            let cid_t = conv_id.clone();
            let query_t = query.clone();
            let answer_t = answer.clone();
            let prior_t = history.clone();
            let count_t = match &persist {
                TurnPersist::New { .. } => message_count + 2,
                TurnPersist::Continuation => message_count,
            };
            tokio::spawn(async move {
                if let Some(new_title) = maybe_regenerate_title(
                    &db_t, &*llm_t, &cid_t, &prior_t, &query_t, &answer_t, count_t,
                )
                .await
                {
                    let data = json!({
                        "type": "title_updated",
                        "conv_id": cid_t,
                        "title": new_title,
                    })
                    .to_string();
                    live_t.broadcast(&pid_t, data);
                }
            });
        }
    }

    if let Ok(Some(paused_turn)) = paused_rx.await {
        let paused = PausedConfirm {
            messages: paused_turn.messages,
            iterations: paused_turn.iterations,
            pending: paused_turn.pending,
            resolved: HashMap::new(),
            selection,
            elapsed_ms: paused_turn.elapsed_ms,
        };
        match serde_json::to_value(&paused) {
            Ok(payload) => {
                if let Err(e) = live.save_paused(&project_id, &conv_id, &payload).await {
                    tracing::error!(error = %e, conv_id, "failed to persist paused confirmation");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, conv_id, "failed to serialize paused confirmation")
            }
        }
    }

    if !released {
        if let Err(e) = live.finish(&conv_id, &turn_id).await {
            tracing::warn!(error = %e, conv_id, "failed to release conversation lock");
        }
        live.broadcast(
            &project_id,
            json!({"type": "unlock", "conv_id": conv_id}).to_string(),
        );
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
            if let Some(d) = description {
                v["description"] = json!(d);
            }
            if let Some(h) = hostname {
                v["hostname"] = json!(h);
            }
            Some(v)
        }
        AgentEvent::ToolResult {
            name,
            preview,
            view,
        } => Some(json!({
            "type": "tool_result", "conv_id": conv_id,
            "name": name, "preview": preview, "view": view,
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
        AgentEvent::ParallelResearchLeadDone {
            index,
            iterations,
            preview,
            duration_ms,
        } => Some(json!({
            "type": "parallel_research_lead_done", "conv_id": conv_id,
            "index": index, "iterations": iterations, "preview": preview, "duration_ms": duration_ms,
        })),
        AgentEvent::ParallelResearchMergeStarted { duration_ms } => Some(json!({
            "type": "parallel_research_merge_started", "conv_id": conv_id, "duration_ms": duration_ms,
        })),
        AgentEvent::Done {
            answer,
            sources,
            tool_calls_made,
            provider_used,
            duration_ms,
            usage,
            llm_call_count,
            ..
        } => {
            let cost_microusd = pricing.price_call(
                usage,
                provider_used
                    .as_ref()
                    .map(|p| p.kind.as_str())
                    .unwrap_or(""),
                provider_used
                    .as_ref()
                    .map(|p| p.model.as_str())
                    .unwrap_or(""),
            );
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
        AgentEvent::ConfirmAction {
            id,
            name,
            input,
            description,
        } => Some(json!({
            "type": "confirm_action", "conv_id": conv_id,
            "id": id, "name": name, "input": input, "description": description,
        })),
        AgentEvent::Error { message } => Some(json!({
            "type": "error", "conv_id": conv_id, "message": message,
        })),
        AgentEvent::TitleUpdated { title } => Some(json!({
            "type": "title_updated", "conv_id": conv_id, "title": title,
        })),
        AgentEvent::Enumeration { .. } | AgentEvent::LlmCall(_) => None,
    }
}
