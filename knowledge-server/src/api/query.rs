use axum::{
    extract::State,
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures::StreamExt as _;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::mpsc;

use crate::agent::{Agent, AgentEvent, Attachment, HistoryMessage};

#[derive(Deserialize)]
pub struct QueryRequest {
    pub query: String,
    pub history: Option<Vec<HistoryMessage>>,
    pub attachments: Option<Vec<Attachment>>,
    pub repositories: Option<Vec<String>>,
    pub versions: Option<Vec<String>>,
}

pub async fn handle_query(
    State(agent): State<Arc<Agent>>,
    Json(req): Json<QueryRequest>,
) -> impl IntoResponse {
    let history = req.history.as_deref().unwrap_or(&[]);
    let attachments = req.attachments.as_deref().unwrap_or(&[]);
    match agent.query(&req.query, history, attachments).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "query failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn handle_query_stream(
    State(agent): State<Arc<Agent>>,
    Json(req): Json<QueryRequest>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let (tx, rx) = mpsc::channel::<AgentEvent>(64);
    let history = req.history.unwrap_or_default();
    let attachments = req.attachments.unwrap_or_default();

    tokio::spawn(async move {
        agent.query_streaming(&req.query, &history, &attachments, tx).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
