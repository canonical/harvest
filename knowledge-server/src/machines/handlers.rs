use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post},
    Extension, Json, Router,
};
use futures::StreamExt as _;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::{wrappers::ReceiverStream, Stream};
use uuid::Uuid;

use crate::{auth::jwt::Claims, neo4j::Neo4jClient};
use super::{ConnectedAgent, MachineRegistry, ResultBody, ServerToAgent, hash_token};

const SSE_KEEPALIVE_INTERVAL_SECS: u64 = 25;
const DEFAULT_EXECUTE_TIMEOUT_SECS: u64 = 30;
const MAX_EXECUTE_TIMEOUT_SECS: u64 = 300;

pub struct MachineState {
    pub registry:    Arc<MachineRegistry>,
    pub neo4j:       Option<Arc<Neo4jClient>>,
    pub binary_path: Option<PathBuf>,
    pub server_url:  String,
}

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

fn neo4j_or_err(state: &MachineState) -> Result<&Arc<Neo4jClient>, ApiError> {
    state.neo4j.as_ref().ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable"))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

pub fn generate_install_script(server_url: &str, install_token: &str) -> String {
    format!(
        r#"#!/usr/bin/env bash
# Harvest Agent Installer
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "error: this installer must be run as root (try: curl ... | sudo bash)" >&2
  exit 1
fi

SERVER_URL="{server_url}"
INSTALL_TOKEN="{install_token}"
BINARY_URL="${{SERVER_URL}}/agents/binary/harvest-agent"
SERVICE_NAME="harvest-agent"

echo "Installing Harvest Agent..."

# Download binary
curl -fsSL "${{BINARY_URL}}" -o /usr/local/bin/harvest-agent
chmod +x /usr/local/bin/harvest-agent

# Write configuration (no project_id — server derives project from token)
mkdir -p /etc/harvest-agent
chmod 700 /etc/harvest-agent
cat > /etc/harvest-agent/config.toml << EOF
server_url  = "${{SERVER_URL}}"
agent_token = "${{INSTALL_TOKEN}}"
EOF
chmod 600 /etc/harvest-agent/config.toml

# Install systemd service
cat > /etc/systemd/system/harvest-agent.service << 'SERVICE'
[Unit]
Description=Harvest Agent
After=network.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/harvest-agent
Restart=always
RestartSec=5
User=root

[Install]
WantedBy=multi-user.target
SERVICE

# Install uninstall script
cat > /usr/local/bin/uninstall-harvest-agent << 'UNINSTALL'
#!/usr/bin/env bash
set -e
systemctl stop harvest-agent  2>/dev/null || true
systemctl disable harvest-agent 2>/dev/null || true
rm -f /etc/systemd/system/harvest-agent.service
systemctl daemon-reload
rm -f /usr/local/bin/harvest-agent
rm -rf /etc/harvest-agent
rm -f /usr/local/bin/uninstall-harvest-agent
echo "Harvest agent successfully uninstalled."
UNINSTALL
chmod +x /usr/local/bin/uninstall-harvest-agent

systemctl daemon-reload
systemctl enable harvest-agent
systemctl restart harvest-agent

echo "Harvest agent installed and running."
echo "Run 'uninstall-harvest-agent' to remove."
"#
    )
}

pub fn machines_router(state: Arc<MachineState>) -> Router {
    Router::new()
        .route("/agents/:project_id/install.sh", get(install_script_handler))
        .route("/agents/binary/harvest-agent",   get(binary_handler))
        .route("/agent/events",                  get(agent_events_handler))
        .route("/agent/results",                 post(agent_results_handler))
        .route("/agent/ping",                    post(agent_ping_handler))
        .with_state(state)
}

pub fn machines_protected_router(state: Arc<MachineState>) -> Router {
    Router::new()
        .route("/projects/:pid/agents",                      get(list_agents))
        .route("/projects/:pid/agents/:aid",                 delete(delete_agent))
        .route("/projects/:pid/agents/:aid/execute",         post(execute_command))
        .route("/projects/:pid/agents/rotate-install-token", post(rotate_install_token))
        .with_state(state)
}

async fn install_script_handler(
    State(state): State<Arc<MachineState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let neo4j = match neo4j_or_err(&state) {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };

    let rows = match neo4j.query_read(
        "MATCH (p:Project {id: $pid}) RETURN p.install_token AS install_token",
        json!({ "pid": project_id }),
    ).await {
        Ok(r) => r,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response(),
    };

    let first = match rows.into_iter().next() {
        Some(r) => r,
        None    => return err(StatusCode::NOT_FOUND, "project not found").into_response(),
    };

    let install_token = match first["install_token"].as_str().map(|s| s.to_string()) {
        Some(t) => t,
        None => {
            let tok = Uuid::new_v4().to_string();
            if neo4j.query_read(
                "MATCH (p:Project {id: $pid}) SET p.install_token = $tok",
                json!({ "pid": project_id, "tok": tok }),
            ).await.is_err() {
                return err(StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response();
            }
            tok
        }
    };

    let script = generate_install_script(&state.server_url, &install_token);
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/x-shellscript")], script).into_response()
}

async fn binary_handler(State(state): State<Arc<MachineState>>) -> impl IntoResponse {
    let path = match &state.binary_path {
        Some(p) => p.clone(),
        None    => return err(StatusCode::NOT_FOUND, "binary not configured on this server").into_response(),
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        ).into_response(),
        Err(_) => err(StatusCode::NOT_FOUND, "binary not found").into_response(),
    }
}

struct AgentGuard {
    agent_id:   String,
    index_hash: String,
    registry:   Arc<MachineRegistry>,
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        self.registry.agents.remove(&self.agent_id);
        self.registry.token_index.remove(&self.index_hash);
        tracing::info!(agent_id = %self.agent_id, "agent disconnected");
    }
}

struct GuardedStream<S: Unpin> {
    inner:  S,
    _guard: AgentGuard,
}

impl<S: Stream + Unpin> Stream for GuardedStream<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

async fn agent_events_handler(
    State(state):  State<Arc<MachineState>>,
    headers:       HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let neo4j = match &state.neo4j {
        Some(n) => Arc::clone(n),
        None    => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let hostname = params.get("hostname").cloned().unwrap_or_else(|| "unknown".into());

    let token_hash = hash_token(&token);

    let machine = neo4j.query_read(
        "MATCH (m:Machine) WHERE m.agent_token_hash = $h
         RETURN m.id AS id, m.project_id AS project_id",
        json!({ "h": token_hash }),
    ).await.ok().and_then(|r| r.into_iter().next());

    let (agent_id, project_id, index_hash, first_event) = if let Some(row) = machine {
        let id  = row["id"].as_str().unwrap_or("").to_string();
        let pid = row["project_id"].as_str().unwrap_or("").to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let _ = neo4j.query_read(
            "MATCH (m:Machine {id: $id}) SET m.hostname = $h, m.last_seen = $now",
            json!({ "id": id, "h": hostname, "now": now }),
        ).await;
        (id, pid, token_hash, ServerToAgent::HelloAck)
    } else {
        let project = neo4j.query_read(
            "MATCH (p:Project {install_token: $tok}) RETURN p.id AS id",
            json!({ "tok": token }),
        ).await.ok().and_then(|r| r.into_iter().next());

        let project_id = match project {
            Some(r) => r["id"].as_str().unwrap_or("").to_string(),
            None    => {
                tracing::warn!(hostname, "agent connected with unrecognised token");
                return StatusCode::UNAUTHORIZED.into_response();
            }
        };

        let aid        = Uuid::new_v4().to_string();
        let perm_token = Uuid::new_v4().to_string();
        let perm_hash  = hash_token(&perm_token);
        let now        = chrono::Utc::now().to_rfc3339();

        if neo4j.query_read(
            "CREATE (m:Machine {
                 id: $id, project_id: $pid, hostname: $h,
                 agent_token_hash: $hash, created_at: $now, last_seen: $now
             })",
            json!({
                "id": aid, "pid": project_id, "h": hostname,
                "hash": perm_hash, "now": now,
            }),
        ).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        (aid, project_id, perm_hash, ServerToAgent::Registered { agent_token: perm_token })
    };

    let (command_sender, command_receiver) = mpsc::channel::<ServerToAgent>(64);

    state.registry.agents.insert(agent_id.clone(), ConnectedAgent {
        id:           agent_id.clone(),
        project_id,
        hostname:     hostname.clone(),
        connected_at: chrono::Utc::now(),
        sender:       command_sender,
    });
    state.registry.token_index.insert(index_hash.clone(), agent_id.clone());

    tracing::info!(agent_id, hostname, "agent connected via SSE");

    let first_data  = serde_json::to_string(&first_event).unwrap_or_default();
    let init_stream = tokio_stream::iter(vec![
        Ok::<Event, Infallible>(Event::default().data(first_data)),
    ]);

    let guard = AgentGuard { agent_id, index_hash, registry: Arc::clone(&state.registry) };
    let command_stream = GuardedStream {
        inner: ReceiverStream::new(command_receiver).map(|msg| {
            let data = serde_json::to_string(&msg).unwrap_or_default();
            Ok::<Event, Infallible>(Event::default().data(data))
        }),
        _guard: guard,
    };

    let mut response = Sse::new(init_stream.chain(command_stream))
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(SSE_KEEPALIVE_INTERVAL_SECS))
                .text("keep-alive"),
        )
        .into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        header::HeaderValue::from_static("no"),
    );
    response
}

async fn agent_results_handler(
    State(state): State<Arc<MachineState>>,
    headers:      HeaderMap,
    Json(body):   Json<ResultBody>,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    if state.registry.token_index.get(&hash_token(&token)).is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    if let Some((_, pending)) = state.registry.pending.remove(&body.request_id) {
        let _ = pending.tx.send(Ok(super::CommandResult {
            stdout:    body.stdout,
            stderr:    body.stderr,
            exit_code: body.exit_code,
        }));
    }

    StatusCode::OK.into_response()
}

async fn agent_ping_handler(
    State(state): State<Arc<MachineState>>,
    headers:      HeaderMap,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let token_hash = hash_token(&token);
    let agent_id = match state.registry.token_index.get(&token_hash) {
        Some(r) => r.value().clone(),
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    if let Some(neo4j) = &state.neo4j {
        let neo4j = Arc::clone(neo4j);
        tokio::spawn(async move {
            let now = chrono::Utc::now().to_rfc3339();
            let _ = neo4j.query_read(
                "MATCH (m:Machine {id: $id}) SET m.last_seen = $now",
                json!({ "id": agent_id, "now": now }),
            ).await;
        });
    }

    StatusCode::OK.into_response()
}

pub async fn list_agents(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let db_machines = neo4j.query_read(
        "MATCH (m:Machine {project_id: $pid})
         RETURN m.id AS id, m.hostname AS hostname,
                m.last_seen AS last_seen, m.created_at AS created_at
         ORDER BY m.created_at ASC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let result: Vec<Value> = db_machines.into_iter().map(|m| {
        let id     = m["id"].as_str().unwrap_or("").to_string();
        let online = state.registry.agents.contains_key(&id);
        json!({
            "id":         id,
            "hostname":   m["hostname"],
            "online":     online,
            "last_seen":  m["last_seen"],
            "created_at": m["created_at"],
        })
    }).collect();

    Ok(Json(result))
}

#[derive(serde::Deserialize)]
pub struct ExecuteBody {
    pub command:      String,
    #[serde(default = "default_execute_timeout")]
    pub timeout_secs: u64,
}

fn default_execute_timeout() -> u64 { DEFAULT_EXECUTE_TIMEOUT_SECS }

pub async fn execute_command(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
    Json(body):       Json<ExecuteBody>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let timeout = body.timeout_secs.min(MAX_EXECUTE_TIMEOUT_SECS);
    match state.registry.execute(&agent_id, body.command, timeout).await {
        Ok(r) => Ok(Json(json!({
            "stdout":    r.stdout,
            "stderr":    r.stderr,
            "exit_code": r.exit_code,
        }))),
        Err(e) => Err(err(StatusCode::BAD_GATEWAY, &e)),
    }
}

pub async fn delete_agent(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let rows = neo4j.query_read(
        "MATCH (m:Machine {id: $aid, project_id: $pid}) RETURN m.id AS id",
        json!({ "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    if rows.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "agent not found"));
    }

    let sender = state.registry.agents.get(&agent_id).map(|a| a.sender.clone());
    state.registry.agents.remove(&agent_id);
    if let Some(sender) = sender {
        let _ = sender.send(super::ServerToAgent::Uninstall).await;
    }

    neo4j.query_read(
        "MATCH (m:Machine {id: $aid, project_id: $pid}) DELETE m",
        json!({ "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}

pub async fn rotate_install_token(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let new_token = Uuid::new_v4().to_string();
    neo4j.query_read(
        "MATCH (p:Project {id: $pid}) SET p.install_token = $tok",
        json!({ "pid": project_id, "tok": new_token }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    Ok(Json(json!({ "ok": true })))
}

async fn require_project_access(
    neo4j:      &Neo4jClient,
    user_id:    &str,
    user_role:  &str,
    project_id: &str,
) -> Result<(), ApiError> {
    let rows = neo4j.query_read(
        "MATCH (g:Group)-[:HAS_PROJECT]->(p:Project {id: $pid})
         WHERE $role = 'admin'
            OR EXISTS { MATCH (:User {id: $uid})-[:MEMBER_OF]->(g) }
         RETURN p.id AS id",
        json!({ "pid": project_id, "uid": user_id, "role": user_role }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    if rows.is_empty() {
        return Err(err(StatusCode::NOT_FOUND, "project not found"));
    }
    Ok(())
}
