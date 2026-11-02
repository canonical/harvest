use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use openraft::raft::{AppendEntriesRequest, InstallSnapshotRequest, VoteRequest};
use serde::Deserialize;

use crate::network::{APPEND_PATH, SNAPSHOT_PATH, VOTE_PATH};
use crate::node::{JoinRequest, Quorium};
use crate::state::Command;
use crate::types::{ServerId, TypeConfig};

type Shared = State<Arc<Quorium>>;

pub fn router(quorium: Arc<Quorium>) -> Router {
    Router::new()
        .route(APPEND_PATH, post(append))
        .route(VOTE_PATH, post(vote))
        .route(SNAPSHOT_PATH, post(snapshot))
        .route("/internal/quorium/keepalive", post(keepalive))
        .route("/internal/quorium/write", post(write))
        .route("/internal/quorium/members", get(members))
        .route("/internal/quorium/servers", post(admit))
        .route("/internal/quorium/servers/:id", delete(remove))
        .route(
            "/internal/quorium/tokens",
            post(issue_token).delete(revoke_tokens),
        )
        .route("/internal/quorium/status", get(status))
        .with_state(quorium)
}

fn failure(status: StatusCode, error: impl std::fmt::Display) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": error.to_string() })),
    )
        .into_response()
}

async fn append(State(q): Shared, Json(rpc): Json<AppendEntriesRequest<TypeConfig>>) -> Response {
    Json(q.raft().append_entries(rpc).await).into_response()
}

async fn vote(State(q): Shared, Json(rpc): Json<VoteRequest<ServerId>>) -> Response {
    Json(q.raft().vote(rpc).await).into_response()
}

async fn snapshot(
    State(q): Shared,
    Json(rpc): Json<InstallSnapshotRequest<TypeConfig>>,
) -> Response {
    Json(q.raft().install_snapshot(rpc).await).into_response()
}

async fn keepalive(State(q): Shared, Json(node_id): Json<String>) -> Response {
    if !q.is_leader() {
        return failure(StatusCode::MISDIRECTED_REQUEST, "not the leader");
    }
    Json(q.touch(&node_id)).into_response()
}

async fn write(State(q): Shared, Json(command): Json<Command>) -> Response {
    match q.write_locally(command).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn members(State(q): Shared) -> Response {
    if !q.is_leader() {
        return failure(StatusCode::MISDIRECTED_REQUEST, "not the leader");
    }
    match q.raft().ensure_linearizable().await {
        Ok(_) => Json(q.members()).into_response(),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn admit(State(q): Shared, Json(request): Json<JoinRequest>) -> Response {
    if !q.is_leader() {
        return match forward_admission(&q, &request).await {
            Ok(()) => Json(()).into_response(),
            Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
        };
    }
    match q.admit(request).await {
        Ok(true) => Json(()).into_response(),
        Ok(false) => failure(
            StatusCode::FORBIDDEN,
            "the join token is invalid, used or expired",
        ),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn forward_admission(q: &Quorium, request: &JoinRequest) -> anyhow::Result<()> {
    let leader = q
        .health()
        .leader
        .ok_or_else(|| anyhow::anyhow!("no quorium leader is available"))?;
    q.post::<_, ()>(&leader, "/internal/quorium/servers", request)
        .await
}

async fn remove(State(q): Shared, Path(id): Path<ServerId>) -> Response {
    match q.remove_server(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::CONFLICT, e),
    }
}

#[derive(Deserialize)]
struct TokenRequest {
    ttl_secs: u64,
}

async fn issue_token(State(q): Shared, Json(request): Json<TokenRequest>) -> Response {
    match q.issue_token(Duration::from_secs(request.ttl_secs)).await {
        Ok(token) => Json(token).into_response(),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn revoke_tokens(State(q): Shared) -> Response {
    match q.revoke_tokens().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn status(State(q): Shared) -> Response {
    Json(q.status()).into_response()
}
