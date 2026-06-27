use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::StreamExt as _;
use serde::Deserialize;
use serde_json::json;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::mpsc;

use crate::agent::{AgentEvent, Attachment};
use crate::api::QueryState;
use crate::auth::jwt::Claims;
use crate::conversations::handlers::{append_user_turn, load_user_history};

#[derive(Deserialize)]
pub struct QueryRequest {
    pub query: String,
    pub conversation_id: Option<String>,
    pub attachments: Option<Vec<Attachment>>,
    pub repositories: Option<Vec<String>>,
    pub versions: Option<Vec<String>>,
}

pub async fn handle_query(
    Extension(user): Extension<Claims>,
    State(qs): State<Arc<QueryState>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let attachments = req.attachments.as_deref().unwrap_or(&[]);
    let history = load_history_if_needed(&qs, &user.sub, req.conversation_id.as_deref()).await;
    let compacted = qs.agent.compact_history(&history).await;
    match qs.agent.query(&req.query, &compacted, attachments).await {
        Ok(response) => {
            if let (Some(neo4j), Some(cid)) = (&qs.neo4j, &req.conversation_id) {
                let att_meta: Vec<_> = attachments.iter()
                    .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
                    .collect();
                let _ = append_user_turn(
                    neo4j, &user.sub, cid,
                    &req.query, &user.name, &att_meta, &compacted,
                    &response.answer, &response.sources, response.tool_calls_made,
                ).await;
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
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let attachments = req.attachments.unwrap_or_default();
    let history = load_history_if_needed(&qs, &user.sub, req.conversation_id.as_deref()).await;
    let compacted = qs.agent.compact_history(&history).await;

    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    let agent = Arc::clone(&qs.agent);
    let neo4j = qs.neo4j.clone();
    let user_id  = user.sub.clone();
    let username = user.name.clone();
    let query    = req.query.clone();
    let conv_id  = req.conversation_id.clone();
    let att_meta: Vec<_> = attachments.iter()
        .map(|a| json!({ "name": a.name, "mime_type": a.mime_type, "data": a.data }))
        .collect();

    tokio::spawn(async move {
        let (agent_tx, mut agent_rx) = mpsc::channel::<AgentEvent>(64);
        let query_for_agent          = query.clone();
        let compacted_for_agent      = compacted.clone();
        tokio::spawn(async move {
            agent.query_streaming(&query_for_agent, &compacted_for_agent, &attachments, agent_tx).await;
        });

        while let Some(event) = agent_rx.recv().await {
            if let (AgentEvent::Done { answer, sources, tool_calls_made }, Some(cid), Some(neo4j)) =
                (&event, &conv_id, &neo4j)
            {
                let _ = append_user_turn(
                    neo4j, &user_id, cid,
                    &query, &username, &att_meta, &compacted,
                    answer, sources, *tool_calls_made,
                ).await;
            }
            let _ = tx.send(event).await;
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn load_history_if_needed(
    qs: &QueryState,
    user_id: &str,
    conv_id: Option<&str>,
) -> Vec<crate::agent::HistoryMessage> {
    match (conv_id, &qs.neo4j) {
        (Some(cid), Some(neo4j)) => {
            load_user_history(neo4j, user_id, cid).await.unwrap_or_default()
        }
        _ => vec![],
    }
}
