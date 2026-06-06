use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use dashmap::DashMap;
use futures::StreamExt as _;
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

use crate::{
    agent::Agent,
    api::ProjectAgentBuilder,
    auth::jwt::Claims,
    llm::LlmProvider,
    neo4j::Neo4jClient,
    overview,
    projects::handlers::require_project_access,
};

const OVERVIEW_BROADCAST_BUFFER: usize = 256;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Clone)]
pub struct OverviewState {
    pub neo4j:         Arc<Neo4jClient>,
    pub llm:           Arc<dyn LlmProvider>,
    pub agent_builder: Arc<ProjectAgentBuilder>,
    pub agent:         Arc<Agent>,
    pub generating:    Arc<DashMap<String, broadcast::Sender<String>>>,
}

fn sse_stream(rx: broadcast::Receiver<String>) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        std::future::ready(
            msg.ok().map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
        )
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn get_overview(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<OverviewState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await?;

    let ov = overview::get(&state.neo4j, &project_id).await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let conv_count = overview::conversation_count(&state.neo4j, &project_id).await
        .unwrap_or(0);

    Ok(Json(json!({
        "current_status":            ov.current_status,
        "current_status_updated_at": ov.current_status_updated_at,
        "has_conversations":         conv_count > 0,
        "generating":                state.generating.contains_key(&project_id),
    })))
}

pub async fn overview_events(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<OverviewState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    if let Some(tx) = state.generating.get(&project_id) {
        let rx = tx.subscribe();
        drop(tx);
        return sse_stream(rx).into_response();
    }

    let (tx, rx) = broadcast::channel(1);
    let _ = tx.send(json!({"type": "overview_done"}).to_string());
    sse_stream(rx).into_response()
}

pub async fn regenerate_overview(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<OverviewState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_project_access(&state.neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    if let Some(tx) = state.generating.get(&project_id) {
        let rx = tx.subscribe();
        drop(tx);
        return sse_stream(rx).into_response();
    }

    let (tx, rx) = broadcast::channel::<String>(OVERVIEW_BROADCAST_BUFFER);
    state.generating.insert(project_id.clone(), tx.clone());

    let llm           = Arc::clone(&state.llm);
    let agent_builder = Arc::clone(&state.agent_builder);
    let neo4j         = Arc::clone(&state.neo4j);
    let generating    = Arc::clone(&state.generating);
    let project_id_owned = project_id.clone();

    tokio::spawn(async move {
        let result = overview::pipeline::run(
            llm, agent_builder, neo4j, &project_id_owned, tx.clone(),
        ).await;

        if let Err(e) = result {
            tracing::error!(project_id = project_id_owned, error = %e, "overview pipeline failed");
        }
        let _ = tx.send(json!({"type": "overview_done"}).to_string());
        generating.remove(&project_id_owned);
    });

    sse_stream(rx).into_response()
}
