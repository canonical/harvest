use axum::{
    extract::{Extension, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::mpsc;

use crate::agent::{chain::ChainBuilder, Agent, AgentEvent, Attachment};
use crate::api::{resolve_user_llm, QueryState};
use crate::auth::jwt::Claims;
use crate::conversations::handlers::{append_user_turn, load_conversation_context};
use crate::conversations::title_generation::maybe_regenerate_title;
use crate::llm::types::ProviderSelection;

#[derive(Deserialize)]
pub struct QueryRequest {
    pub query: String,
    pub conversation_id: Option<String>,
    pub attachments: Option<Vec<Attachment>>,
    pub repositories: Option<Vec<String>>,
    pub versions: Option<Vec<String>>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
}

fn selection_from(req: &QueryRequest) -> Option<ProviderSelection> {
    req.provider_id.clone().map(|provider_id| ProviderSelection { provider_id, model: req.model.clone() })
}

pub async fn handle_query(
    Extension(user): Extension<Claims>,
    State(qs): State<Arc<QueryState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let attachments = req.attachments.as_deref().unwrap_or(&[]);
    let (raw_messages, history) = load_context_if_needed(&qs, &user.sub, req.conversation_id.as_deref()).await;
    let selection = selection_from(&req);
    let user_llm = resolve_user_llm(&qs.llm, &qs.llm_configs, &qs.user_key_store, &user.sub).await;
    let agent = if Arc::ptr_eq(&user_llm, &qs.llm) {
        Arc::clone(&qs.agent)
    } else {
        let db = qs.db.clone().unwrap_or_else(|| {
            panic!("db must be available when user key providers are configured")
        });
        Arc::new(
            Agent::new(user_llm, crate::agent::graph_tools::all_tools(db), qs.max_iterations)
                .with_compaction(qs.compaction_threshold_chars, qs.compaction_keep_last)
                .with_parallel_research(true),
        )
    };
    let compacted = agent.compact_history(&history).await;
    match agent.query(&req.query, &compacted, attachments, selection.as_ref()).await {
        Ok(response) => {
            if let (Some(db), Some(cid)) = (&qs.db, &req.conversation_id) {
                let att_meta: Vec<_> = attachments.iter()
                    .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
                    .collect();
                let turn_id = uuid::Uuid::new_v4().to_string();
                let _ = append_user_turn(
                    db, &user.sub, cid,
                    &req.query, &user.name, &att_meta, raw_messages,
                    &response.answer, &response.sources, response.tool_calls_made,
                    vec![], None, None, response.provider_used.as_ref(), response.duration_ms,
                    &response.usage, response.llm_call_count, &turn_id, &qs.pricing,
                ).await;

                let msg_count = compacted.len() + 2;
                let db_t   = Arc::clone(db);
                let llm_t     = Arc::clone(agent.llm());
                let cid_t     = cid.clone();
                let prior_t   = compacted.clone();
                let query_t   = req.query.clone();
                let answer_t  = response.answer.clone();
                tokio::spawn(async move {
                    maybe_regenerate_title(
                        &db_t, &*llm_t, &cid_t, &prior_t, &query_t, &answer_t, msg_count,
                    ).await;
                });
            }
            let mut response = response;
            response.cost_microusd = qs.pricing.price_call(
                &response.usage,
                response.provider_used.as_ref().map(|p| p.kind.as_str()).unwrap_or(""),
                response.provider_used.as_ref().map(|p| p.model.as_str()).unwrap_or(""),
            );
            Json(response).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn handle_query_stream(
    Extension(user): Extension<Claims>,
    State(qs): State<Arc<QueryState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let selection = selection_from(&req);
    let attachments = req.attachments.unwrap_or_default();

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    let user_llm = resolve_user_llm(&qs.llm, &qs.llm_configs, &qs.user_key_store, &user.sub).await;
    let agent = if Arc::ptr_eq(&user_llm, &qs.llm) {
        Arc::clone(&qs.agent)
    } else {
        let db = qs.db.clone().unwrap_or_else(|| {
            panic!("db must be available when user key providers are configured")
        });
        Arc::new(
            Agent::new(user_llm.clone(), crate::agent::graph_tools::all_tools(db), qs.max_iterations)
                .with_compaction(qs.compaction_threshold_chars, qs.compaction_keep_last)
                .with_parallel_research(true),
        )
    };
    let llm      = Arc::clone(agent.llm());
    let qs_ctx   = Arc::clone(&qs);
    let db    = qs.db.clone();
    let user_id  = user.sub.clone();
    let username = user.name.clone();
    let query    = req.query.clone();
    let conv_id  = req.conversation_id.clone();
    let att_meta: Vec<_> = attachments.iter()
        .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
        .collect();

    tokio::spawn(async move {
        let (raw_messages, history) =
            load_context_if_needed(&qs_ctx, &user_id, conv_id.as_deref()).await;
        let compacted = agent.compact_history(&history).await;

        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
        let query_for_agent     = query.clone();
        let compacted_for_agent = compacted.clone();
        let selection_for_agent = selection.clone();
        let agent_for_query     = Arc::clone(&agent);
        tokio::spawn(async move {
            agent_for_query.query_streaming(&query_for_agent, &compacted_for_agent, &attachments, selection_for_agent.as_ref(), agent_tx).await;
        });

        let mut chain_builder = ChainBuilder::new();
        let mut pending_question: Option<Value> = None;
        let mut pending_confirm_action: Option<Value> = None;

        while let Some(event) = agent_rx.recv().await {
            match &event {
                AgentEvent::TextDelta { text } => chain_builder.text_delta(text),
                AgentEvent::ThinkingDelta { text } => chain_builder.thinking_delta(text),
                AgentEvent::Thinking { text } => chain_builder.thinking(text),
                AgentEvent::ToolCall { name, input } => chain_builder.tool_call(name, input, None, None),
                AgentEvent::ToolResult { name, preview } => chain_builder.tool_result(name, preview),
                AgentEvent::Question { question, choices } => {
                    pending_question = Some(json!({ "question": question, "choices": choices }));
                }
                AgentEvent::ConfirmAction { name, input, description, .. } => {
                    pending_confirm_action = Some(json!({
                        "name": name, "input": input, "description": description,
                        "status": "pending", "steps": [], "result_text": "",
                    }));
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

            if let (
                AgentEvent::Done { answer, sources, tool_calls_made, provider_used, duration_ms, usage, llm_call_count, .. },
                Some(cid),
                Some(db),
            ) = (&event, &conv_id, &db) {
                let chain = std::mem::take(&mut chain_builder).finish();
                let turn_id = uuid::Uuid::new_v4().to_string();
                let _ = append_user_turn(
                    db, &user_id, cid,
                    &query, &username, &att_meta, raw_messages.clone(),
                    answer, sources, *tool_calls_made,
                    chain, pending_question.clone(), pending_confirm_action.clone(),
                    provider_used.as_ref(), *duration_ms,
                    usage, *llm_call_count, &turn_id, &qs_ctx.pricing,
                ).await;

                let msg_count = compacted.len() + 2;
                let db_t  = Arc::clone(db);
                let llm_t    = Arc::clone(&llm);
                let cid_t    = cid.clone();
                let prior_t  = compacted.clone();
                let query_t  = query.clone();
                let answer_t = answer.clone();
                let tx_t     = tx.clone();
                tokio::spawn(async move {
                    if let Some(title) = maybe_regenerate_title(
                        &db_t, &*llm_t, &cid_t, &prior_t, &query_t, &answer_t, msg_count,
                    ).await {
                        let _ = tx_t.send(AgentEvent::TitleUpdated { title }).await;
                    }
                });
            }

            let _ = tx.send(event).await;
        }
    });

    let pricing_for_stream = Arc::clone(&qs.pricing);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(move |event| {
        let mut value = serde_json::to_value(&event).unwrap_or_default();
        if value.get("type").and_then(|v| v.as_str()) == Some("done") {
            if let Some(usage) = value.get("usage").cloned() {
                let kind = value.get("provider_used").and_then(|p| p.get("kind")).and_then(|v| v.as_str()).unwrap_or("");
                let model = value.get("provider_used").and_then(|p| p.get("model")).and_then(|v| v.as_str()).unwrap_or("");
                let cost = pricing_for_stream.price_call(
                    &serde_json::from_value::<crate::llm::types::Usage>(usage).unwrap_or_default(),
                    kind, model,
                );
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("cost_microusd".to_string(), json!(cost));
                }
            }
        }
        let data = serde_json::to_string(&value).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

async fn load_context_if_needed(
    qs: &QueryState,
    user_id: &str,
    conv_id: Option<&str>,
) -> (Vec<Value>, Vec<crate::agent::HistoryMessage>) {
    match (conv_id, &qs.db) {
        (Some(cid), Some(db)) => {
            load_conversation_context(db, user_id, cid).await.unwrap_or_default()
        }
        _ => (vec![], vec![]),
    }
}
