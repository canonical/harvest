use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use quorium::Quorium;
use serde::Deserialize;
use serde_json::json;

use crate::cluster::node::ClusterNode;

const DEFAULT_TOKEN_TTL_SECS: u64 = 3_600;

pub struct ClusterAdmin {
    pub node: Arc<ClusterNode>,
    pub quorium: Option<Arc<Quorium>>,
    pub shared_secret: String,
}

pub fn router(admin: Arc<ClusterAdmin>) -> Router {
    Router::new()
        .route("/internal/cluster/status", get(status))
        .route(
            "/internal/cluster/tokens",
            post(issue_token).delete(revoke_tokens),
        )
        .route("/internal/cluster/servers/:id", delete(remove_server))
        .route("/internal/cluster/evacuate", post(evacuate))
        .route("/internal/cluster/restore", post(restore))
        .with_state(admin)
}

fn failure(status: StatusCode, error: impl std::fmt::Display) -> Response {
    (status, Json(json!({ "error": error.to_string() }))).into_response()
}

fn require_quorium(admin: &ClusterAdmin) -> Result<&Arc<Quorium>, Response> {
    admin.quorium.as_ref().ok_or_else(|| {
        failure(
            StatusCode::CONFLICT,
            "this operation needs cluster.backend = \"quorium\"",
        )
    })
}

async fn status(State(admin): State<Arc<ClusterAdmin>>) -> Response {
    let members = admin.node.alive_nodes().await.unwrap_or_default();
    Json(json!({
        "node_id": admin.node.node_id(),
        "standing": format!("{:?}", admin.node.standing()).to_lowercase(),
        "draining": admin.node.is_draining(),
        "membership": admin.node.membership_health(),
        "members": members,
        "quorium": admin.quorium.as_ref().map(|q| q.status()),
    }))
    .into_response()
}

#[derive(Deserialize, Default)]
struct TokenRequest {
    ttl_secs: Option<u64>,
}

async fn issue_token(
    State(admin): State<Arc<ClusterAdmin>>,
    body: Option<Json<TokenRequest>>,
) -> Response {
    let quorium = match require_quorium(&admin) {
        Ok(q) => q,
        Err(response) => return response,
    };
    let ttl = body
        .and_then(|Json(b)| b.ttl_secs)
        .unwrap_or(DEFAULT_TOKEN_TTL_SECS);
    match quorium.issue_token(Duration::from_secs(ttl)).await {
        Ok(mut token) => {
            token.shared_secret = admin.shared_secret.clone();
            Json(json!({ "token": token.encode(), "expires_in_secs": ttl })).into_response()
        }
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn revoke_tokens(State(admin): State<Arc<ClusterAdmin>>) -> Response {
    let quorium = match require_quorium(&admin) {
        Ok(q) => q,
        Err(response) => return response,
    };
    match quorium.revoke_tokens().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn evacuate(State(admin): State<Arc<ClusterAdmin>>) -> Response {
    match admin.node.set_draining().await {
        Ok(()) => {
            Json(json!({ "node_id": admin.node.node_id(), "draining": true })).into_response()
        }
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn restore(State(admin): State<Arc<ClusterAdmin>>) -> Response {
    match admin.node.restore().await {
        Ok(()) => {
            Json(json!({ "node_id": admin.node.node_id(), "draining": false })).into_response()
        }
        Err(e) => failure(StatusCode::SERVICE_UNAVAILABLE, e),
    }
}

async fn remove_server(State(admin): State<Arc<ClusterAdmin>>, Path(id): Path<String>) -> Response {
    let quorium = match require_quorium(&admin) {
        Ok(q) => q,
        Err(response) => return response,
    };
    let server_id = match resolve_server(quorium, &id) {
        Some(server_id) => server_id,
        None => return failure(StatusCode::NOT_FOUND, format!("no server matches {id}")),
    };
    match quorium.remove_server(server_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(StatusCode::CONFLICT, e),
    }
}

fn resolve_server(quorium: &Quorium, id: &str) -> Option<quorium::types::ServerId> {
    let servers = quorium.servers();
    if let Some(server) = id
        .parse::<u64>()
        .ok()
        .and_then(|n| servers.iter().find(|s| s.id == n))
    {
        return Some(server.id);
    }
    servers
        .iter()
        .find(|s| s.address.trim_end_matches('/') == id.trim_end_matches('/'))
        .map(|s| s.id)
}
