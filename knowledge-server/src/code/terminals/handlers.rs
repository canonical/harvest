use std::sync::Arc;

use axum::{
    extract::{ws::WebSocketUpgrade, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::connection::{node_leaving, serve};
use super::model::{TerminalId, TerminalSize};
use super::service::{self, TerminalError};
use super::store;
use crate::api::origin::is_same_origin;
use crate::auth::jwt::Claims;
use crate::code::handlers::{env, failure, internal, session_for, CodeState, Reply};

impl IntoResponse for TerminalError {
    fn into_response(self) -> Response {
        match self {
            Self::NotFound => failure(StatusCode::NOT_FOUND, "not found"),
            Self::SessionInactive => failure(StatusCode::CONFLICT, "the session is not active"),
            Self::LimitReached => failure(
                StatusCode::TOO_MANY_REQUESTS,
                format!("a session can have at most {} terminals", store::MAX_PER_SESSION),
            ),
            Self::Container(error) => {
                tracing::warn!(error = %format!("{error:#}"), "terminal container request failed");
                failure(StatusCode::BAD_GATEWAY, "the session's container did not respond")
            }
        }
    }
}

fn terminal_id(raw: &str) -> Result<TerminalId, Response> {
    TerminalId::parse(raw).ok_or_else(|| TerminalError::NotFound.into_response())
}

pub async fn list(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let terminals = store::list(&env.db, &session_id).await.map_err(internal)?;
    Ok(Json(json!({ "terminals": terminals })).into_response())
}

pub async fn create(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let terminal = service::create(env, &session_id).await.map_err(IntoResponse::into_response)?;
    tracing::info!(session = session_id, terminal = terminal.id, user = user.sub, "terminal created");
    Ok((StatusCode::CREATED, Json(json!({ "terminal": terminal }))).into_response())
}

#[derive(Deserialize)]
pub struct RenameBody {
    title: String,
}

pub async fn rename(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path((session_id, terminal)): Path<(String, String)>,
    Json(body): Json<RenameBody>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let id = terminal_id(&terminal)?;
    let title = body.title.trim();
    if title.is_empty() || title.chars().count() > 80 {
        return Err(failure(StatusCode::BAD_REQUEST, "a title needs between 1 and 80 characters"));
    }
    let renamed = store::rename(&env.db, &session_id, &id, title)
        .await
        .map_err(internal)?
        .ok_or_else(|| TerminalError::NotFound.into_response())?;
    Ok(Json(json!({ "terminal": renamed })).into_response())
}

pub async fn end(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path((session_id, terminal)): Path<(String, String)>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let id = terminal_id(&terminal)?;
    service::end(env, &session_id, &id).await.map_err(IntoResponse::into_response)?;
    tracing::info!(session = session_id, terminal = id.as_str(), user = user.sub, "terminal ended");
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub struct SocketQuery {
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    rows: u16,
}

pub async fn socket(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path((session_id, terminal)): Path<(String, String)>,
    Query(query): Query<SocketQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Reply {
    if !is_same_origin(&headers, &state.public_origin) {
        return Err(failure(StatusCode::FORBIDDEN, "terminals open only from Harvest itself"));
    }
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let id = terminal_id(&terminal)?;
    let size = TerminalSize::new(query.cols, query.rows).unwrap_or(TerminalSize::DEFAULT);
    let console = service::attach(env, &session_id, &id, size).await.map_err(IntoResponse::into_response)?;
    tracing::info!(session = session_id, terminal = id.as_str(), user = user.sub, "terminal attached");
    let env = env.clone();
    let leaving = node_leaving(&state.node);
    Ok(upgrade.on_upgrade(move |socket| serve(env, session_id, console, socket, leaving)))
}
