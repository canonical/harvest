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

use crate::agent::{chain::ChainBuilder, AgentEvent, Attachment};
use crate::api::QueryState;
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
    let compacted = qs.agent.compact_history(&history).await;
    let selection = selection_from(&req);
    match qs.agent.query(&req.query, &compacted, attachments, selection.as_ref()).await {
        Ok(response) => {
            if let (Some(neo4j), Some(cid)) = (&qs.neo4j, &req.conversation_id) {
                let att_meta: Vec<_> = attachments.iter()
                    .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
                    .collect();
                let _ = append_user_turn(
                    neo4j, &user.sub, cid,
                    &req.query, &user.name, &att_meta, raw_messages,
                    &response.answer, &response.sources, response.tool_calls_made,
                    vec![], None, None, response.provider_used.as_ref(),
                ).await;

                let msg_count = compacted.len() + 2;
                let neo4j_t   = Arc::clone(neo4j);
                let llm_t     = Arc::clone(qs.agent.llm());
                let cid_t     = cid.clone();
                let prior_t   = compacted.clone();
                let query_t   = req.query.clone();
                let answer_t  = response.answer.clone();
                tokio::spawn(async move {
                    maybe_regenerate_title(
                        &neo4j_t, &*llm_t, &cid_t, &prior_t, &query_t, &answer_t, msg_count,
                    ).await;
                });
            }
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
    let (raw_messages, history) = load_context_if_needed(&qs, &user.sub, req.conversation_id.as_deref()).await;
    let compacted = qs.agent.compact_history(&history).await;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    let llm      = Arc::clone(qs.agent.llm());
    let agent    = Arc::clone(&qs.agent);
    let neo4j    = qs.neo4j.clone();
    let user_id  = user.sub.clone();
    let username = user.name.clone();
    let query    = req.query.clone();
    let conv_id  = req.conversation_id.clone();
    let att_meta: Vec<_> = attachments.iter()
        .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
        .collect();

    tokio::spawn(async move {
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
        let query_for_agent     = query.clone();
        let compacted_for_agent = compacted.clone();
        let selection_for_agent = selection.clone();
        tokio::spawn(async move {
            agent.query_streaming(&query_for_agent, &compacted_for_agent, &attachments, selection_for_agent.as_ref(), agent_tx).await;
        });

        let mut chain_builder = ChainBuilder::new();
        let mut pending_question: Option<Value> = None;
        let mut pending_confirm_action: Option<Value> = None;

        while let Some(event) = agent_rx.recv().await {
            match &event {
                AgentEvent::TextDelta { text } => chain_builder.text_delta(text),
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
                _ => {}
            }

            let new_title = if let (
                AgentEvent::Done { answer, sources, tool_calls_made, provider_used },
                Some(cid),
                Some(neo4j),
            ) = (&event, &conv_id, &neo4j) {
                let chain = std::mem::take(&mut chain_builder).finish();
                let _ = append_user_turn(
                    neo4j, &user_id, cid,
                    &query, &username, &att_meta, raw_messages.clone(),
                    answer, sources, *tool_calls_made,
                    chain, pending_question.clone(), pending_confirm_action.clone(),
                    provider_used.as_ref(),
                ).await;
                let msg_count = compacted.len() + 2;
                maybe_regenerate_title(neo4j, &*llm, cid, &compacted, &query, answer, msg_count).await
            } else {
                None
            };

            let _ = tx.send(event).await;
            if let Some(title) = new_title {
                let _ = tx.send(AgentEvent::TitleUpdated { title }).await;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
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
    match (conv_id, &qs.neo4j) {
        (Some(cid), Some(neo4j)) => {
            load_conversation_context(neo4j, user_id, cid).await.unwrap_or_default()
        }
        _ => (vec![], vec![]),
    }
}
