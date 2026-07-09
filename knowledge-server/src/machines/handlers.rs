use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, post, put},
    Extension, Json, Router,
};
use futures::{SinkExt as _, StreamExt as _};
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

use crate::{auth::jwt::Claims, lxd::{Flavor, LxdClient}, neo4j::Neo4jClient};
use super::{lxd_provision, port_forwards, ConnectedAgent, MachineRegistry, ResultBody, ServerToAgent, hash_token};

const SSE_KEEPALIVE_INTERVAL_SECS: u64 = 25;
const DEFAULT_EXECUTE_TIMEOUT_SECS: u64 = 30;
const MAX_EXECUTE_TIMEOUT_SECS: u64 = 300;
const CONSOLE_CLAIM_TIMEOUT_SECS: u64 = 15;
const CONSOLE_MAX_MESSAGE_SIZE: usize = 64 * 1024;
const DEFAULT_CONSOLE_COLS: u16 = 80;
const DEFAULT_CONSOLE_ROWS: u16 = 24;

pub struct MachineState {
    pub registry:    Arc<MachineRegistry>,
    pub neo4j:       Option<Arc<Neo4jClient>>,
    pub binary_path: Option<PathBuf>,
    pub server_url:  String,
    pub lxd:         Option<Arc<LxdClient>>,
}

pub(super) type ApiError = (StatusCode, Json<Value>);

pub(super) fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

pub(super) fn neo4j_or_err(state: &MachineState) -> Result<&Arc<Neo4jClient>, ApiError> {
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
        .route("/agent/console/:session_id",     get(agent_console_claim_handler))
        .route("/agent/tunnel/:session_id",      get(agent_tunnel_claim_handler))
        .with_state(state)
}

pub fn machines_protected_router(state: Arc<MachineState>) -> Router {
    Router::new()
        .route("/projects/:pid/agents",                      get(list_agents))
        .route("/projects/:pid/agents/:aid",                 delete(delete_agent))
        .route("/projects/:pid/agents/:aid/execute",         post(execute_command))
        .route("/projects/:pid/agents/:aid/console",         get(agent_console_open_handler))
        .route("/projects/:pid/agents/:aid/start",           post(start_agent))
        .route("/projects/:pid/agents/:aid/stop",            post(stop_agent))
        .route("/projects/:pid/agents/:aid/restart",         post(restart_agent))
        .route("/projects/:pid/agents/rotate-install-token", post(rotate_install_token))
        .route("/projects/:pid/agents/flavors",              get(list_flavors))
        .route("/projects/:pid/agents/lxd",                  post(create_lxd_agent_handler))
        .route("/projects/:pid/agents/:aid/port-forwards",
               get(list_port_forwards).post(create_port_forward))
        .route("/projects/:pid/agents/:aid/port-forwards/:fid",
               put(update_port_forward).delete(delete_port_forward))
        .route("/agents/:agent_id/:route_name",
               axum::routing::any(super::proxy::port_forward_proxy_handler))
        .route("/agents/:agent_id/:route_name/",
               axum::routing::any(super::proxy::port_forward_proxy_handler))
        .route("/agents/:agent_id/:route_name/*subpath",
               axum::routing::any(super::proxy::port_forward_proxy_handler_subpath))
        .with_state(state)
}

pub(crate) async fn get_or_create_install_token(
    neo4j: &Neo4jClient,
    project_id: &str,
) -> anyhow::Result<Option<String>> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid}) RETURN p.install_token AS install_token",
        json!({ "pid": project_id }),
    ).await?;

    let Some(first) = rows.into_iter().next() else {
        return Ok(None);
    };

    if let Some(tok) = first["install_token"].as_str() {
        return Ok(Some(tok.to_string()));
    }

    let tok = Uuid::new_v4().to_string();
    neo4j.query_read(
        "MATCH (p:Project {id: $pid}) SET p.install_token = $tok",
        json!({ "pid": project_id, "tok": tok }),
    ).await?;
    Ok(Some(tok))
}

async fn install_script_handler(
    State(state): State<Arc<MachineState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let neo4j = match neo4j_or_err(&state) {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };

    let install_token = match get_or_create_install_token(neo4j, &project_id).await {
        Ok(Some(tok)) => tok,
        Ok(None)      => return err(StatusCode::NOT_FOUND, "project not found").into_response(),
        Err(_)        => return err(StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response(),
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
    sender:     mpsc::Sender<ServerToAgent>,
    registry:   Arc<MachineRegistry>,
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        if !self.registry.disconnect_if_current(&self.agent_id, &self.sender) {
            tracing::info!(agent_id = %self.agent_id, "stale connection dropped, agent already reconnected");
            return;
        }

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

        let _ = neo4j.query_read(
            "MATCH (li:LxdInstance {project_id: $pid, hostname: $h})
             WITH li LIMIT 1
             MATCH (m:Machine {id: $mid})
             SET m.provider = 'lxd', m.lxd_instance = li.hostname, m.description = li.description
             DELETE li",
            json!({ "pid": project_id, "h": hostname, "mid": aid }),
        ).await;

        (aid, project_id, perm_hash, ServerToAgent::Registered { agent_token: perm_token })
    };

    let (command_sender, command_receiver) = mpsc::channel::<ServerToAgent>(64);

    state.registry.agents.insert(agent_id.clone(), ConnectedAgent {
        id:           agent_id.clone(),
        project_id,
        hostname:     hostname.clone(),
        connected_at: chrono::Utc::now(),
        sender:       command_sender.clone(),
    });
    state.registry.token_index.insert(index_hash.clone(), agent_id.clone());

    tracing::info!(agent_id, hostname, "agent connected via SSE");

    let first_data  = serde_json::to_string(&first_event).unwrap_or_default();
    let init_stream = tokio_stream::iter(vec![
        Ok::<Event, Infallible>(Event::default().data(first_data)),
    ]);

    let guard = AgentGuard { agent_id, index_hash, sender: command_sender, registry: Arc::clone(&state.registry) };
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
                m.last_seen AS last_seen, m.created_at AS created_at,
                m.provider AS provider, m.description AS description
         ORDER BY m.created_at ASC",
        json!({ "pid": project_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    let result: Vec<Value> = db_machines.into_iter().map(|m| {
        let id       = m["id"].as_str().unwrap_or("").to_string();
        let online   = state.registry.agents.contains_key(&id);
        let provider = m["provider"].as_str().unwrap_or("manual").to_string();
        json!({
            "id":          id,
            "hostname":    m["hostname"],
            "online":      online,
            "last_seen":   m["last_seen"],
            "created_at":  m["created_at"],
            "provider":    provider,
            "description": m["description"],
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

#[derive(serde::Deserialize)]
pub struct ConsoleQuery {
    #[serde(default = "default_console_cols")]
    pub cols: u16,
    #[serde(default = "default_console_rows")]
    pub rows: u16,
}

fn default_console_cols() -> u16 { DEFAULT_CONSOLE_COLS }
fn default_console_rows() -> u16 { DEFAULT_CONSOLE_ROWS }

async fn bridge_socket(
    socket: WebSocket,
    mut in_rx: mpsc::Receiver<Message>,
    out_tx: mpsc::Sender<Message>,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    let reader = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            if out_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = in_rx.recv().await {
        if ws_tx.send(msg).await.is_err() {
            break;
        }
    }

    reader.abort();
}

async fn pump_browser_console_socket(
    socket:         WebSocket,
    session_id:     String,
    registry:       Arc<MachineRegistry>,
    mut to_browser_rx: mpsc::Receiver<Message>,
    to_agent_tx:    mpsc::Sender<Message>,
) {
    let first = tokio::select! {
        msg = to_browser_rx.recv() => msg,
        _ = tokio::time::sleep(Duration::from_secs(CONSOLE_CLAIM_TIMEOUT_SECS)) => None,
    };

    let Some(first) = first else {
        registry.expire_console_session(&session_id);
        let (mut ws_tx, _) = socket.split();
        let _ = ws_tx.send(Message::Text(
            r#"{"type":"error","message":"agent did not respond"}"#.to_string(),
        )).await;
        return;
    };

    let (mut ws_tx, ws_rx) = socket.split();
    if ws_tx.send(first).await.is_err() {
        return;
    }

    let reader = tokio::spawn(async move {
        let mut ws_rx = ws_rx;
        while let Some(Ok(msg)) = ws_rx.next().await {
            if to_agent_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = to_browser_rx.recv().await {
        if ws_tx.send(msg).await.is_err() {
            break;
        }
    }

    reader.abort();
}

pub async fn agent_console_open_handler(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
    Query(q):         Query<ConsoleQuery>,
    ws:               WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    if !state.registry.agents.contains_key(&agent_id) {
        return Err(err(StatusCode::BAD_GATEWAY, "agent not connected"));
    }

    let (session_id, to_browser_rx, to_agent_tx) = state.registry
        .open_console_session(&agent_id, q.cols, q.rows)
        .await
        .map_err(|e| err(StatusCode::BAD_GATEWAY, &e))?;

    tracing::info!(user = %user.email, project_id, agent_id, session_id, "console session opened");

    let registry = Arc::clone(&state.registry);
    Ok(ws
        .max_message_size(CONSOLE_MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| async move {
            pump_browser_console_socket(socket, session_id, registry, to_browser_rx, to_agent_tx).await;
        }))
}

pub async fn agent_console_claim_handler(
    State(state):  State<Arc<MachineState>>,
    headers:       HeaderMap,
    Path(session_id): Path<String>,
    ws:            WebSocketUpgrade,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let agent_id = match state.registry.token_index.get(&hash_token(&token)) {
        Some(r) => r.value().clone(),
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let Some((to_browser_tx, to_agent_rx)) = state.registry.claim_console_session(&session_id, &agent_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    tracing::info!(agent_id, session_id, "console session claimed by agent");

    ws.max_message_size(CONSOLE_MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| async move {
            bridge_socket(socket, to_agent_rx, to_browser_tx).await;
        })
        .into_response()
}

pub async fn agent_tunnel_claim_handler(
    State(state):  State<Arc<MachineState>>,
    headers:       HeaderMap,
    Path(session_id): Path<String>,
    ws:            WebSocketUpgrade,
) -> impl IntoResponse {
    let token = match extract_bearer_token(&headers) {
        Some(t) => t,
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let agent_id = match state.registry.token_index.get(&hash_token(&token)) {
        Some(r) => r.value().clone(),
        None    => return StatusCode::UNAUTHORIZED.into_response(),
    };

    let Some((to_caller_tx, to_agent_rx)) = state.registry.claim_tunnel_session(&session_id, &agent_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    tracing::info!(agent_id, session_id, "tunnel session claimed by agent");

    ws.max_message_size(CONSOLE_MAX_MESSAGE_SIZE)
        .on_upgrade(move |socket| async move {
            bridge_socket(socket, to_agent_rx, to_caller_tx).await;
        })
        .into_response()
}

#[derive(Debug)]
pub enum DeleteAgentError {
    NotFound,
    LxdUnavailable,
    LxdFailed(String),
    Db,
}

impl std::fmt::Display for DeleteAgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeleteAgentError::NotFound       => write!(f, "agent not found"),
            DeleteAgentError::LxdUnavailable => write!(f, "LXD is not configured on this server"),
            DeleteAgentError::LxdFailed(e)   => write!(f, "failed to delete LXD container: {e}"),
            DeleteAgentError::Db             => write!(f, "server error"),
        }
    }
}

impl std::error::Error for DeleteAgentError {}

struct MachineLxdInfo {
    provider:     String,
    lxd_instance: String,
}

async fn lookup_machine_lxd_info(
    neo4j:      &Neo4jClient,
    project_id: &str,
    agent_id:   &str,
) -> Result<Option<MachineLxdInfo>, ()> {
    let rows = neo4j.query_read(
        "MATCH (m:Machine {id: $aid, project_id: $pid})
         RETURN m.id AS id, m.provider AS provider, m.lxd_instance AS lxd_instance",
        json!({ "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| ())?;

    Ok(rows.into_iter().next().map(|m| MachineLxdInfo {
        provider:     m["provider"].as_str().unwrap_or("manual").to_string(),
        lxd_instance: m["lxd_instance"].as_str().unwrap_or_default().to_string(),
    }))
}

async fn lxd_instance_for_agent(
    neo4j:      &Neo4jClient,
    lxd:        Option<&Arc<LxdClient>>,
    project_id: &str,
    agent_id:   &str,
) -> Result<(Arc<LxdClient>, String), ApiError> {
    let info = lookup_machine_lxd_info(neo4j, project_id, agent_id).await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "agent not found"))?;

    if info.provider != "lxd" {
        return Err(err(StatusCode::BAD_REQUEST, "agent is not LXD-managed"));
    }
    let lxd = lxd.ok_or_else(|| err(StatusCode::SERVICE_UNAVAILABLE, "LXD is not configured on this server"))?;
    Ok((Arc::clone(lxd), info.lxd_instance))
}

pub async fn delete_agent_core(
    neo4j:      &Neo4jClient,
    lxd:        Option<&Arc<LxdClient>>,
    registry:   &MachineRegistry,
    project_id: &str,
    agent_id:   &str,
) -> Result<(), DeleteAgentError> {
    let machine = lookup_machine_lxd_info(neo4j, project_id, agent_id).await
        .map_err(|_| DeleteAgentError::Db)?
        .ok_or(DeleteAgentError::NotFound)?;

    if machine.provider == "lxd" {
        let lxd = lxd.ok_or(DeleteAgentError::LxdUnavailable)?;
        lxd.delete_instance(&machine.lxd_instance).await
            .map_err(|e| DeleteAgentError::LxdFailed(e.to_string()))?;
    }

    let sender = registry.agents.get(agent_id).map(|a| a.sender.clone());
    registry.agents.remove(agent_id);
    if let Some(sender) = sender {
        let _ = sender.send(super::ServerToAgent::Uninstall).await;
    }

    neo4j.query_read(
        "MATCH (m:Machine {id: $aid, project_id: $pid}) DELETE m",
        json!({ "aid": agent_id, "pid": project_id }),
    ).await.map_err(|_| DeleteAgentError::Db)?;

    Ok(())
}

pub async fn delete_agent(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    delete_agent_core(neo4j, state.lxd.as_ref(), &state.registry, &project_id, &agent_id)
        .await
        .map_err(|e| match e {
            DeleteAgentError::NotFound       => err(StatusCode::NOT_FOUND, &e.to_string()),
            DeleteAgentError::LxdUnavailable => err(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
            DeleteAgentError::LxdFailed(_)   => err(StatusCode::BAD_GATEWAY, &e.to_string()),
            DeleteAgentError::Db             => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        })?;

    Ok(Json(json!({ "ok": true })))
}

async fn set_lxd_agent_state(
    user:       &Claims,
    state:      &MachineState,
    project_id: &str,
    agent_id:   &str,
    action:     &str,
) -> Result<(), ApiError> {
    let neo4j = neo4j_or_err(state)?;
    require_project_access(neo4j, &user.sub, &user.role, project_id).await?;

    let (lxd, lxd_instance) = lxd_instance_for_agent(neo4j, state.lxd.as_ref(), project_id, agent_id).await?;

    let result = match action {
        "start"   => lxd.start_instance(&lxd_instance).await,
        "stop"    => lxd.stop_instance(&lxd_instance).await,
        "restart" => lxd.restart_instance(&lxd_instance).await,
        _         => unreachable!("unknown lxd agent action: {action}"),
    };

    result.map_err(|e| err(StatusCode::BAD_GATEWAY, &e.to_string()))?;

    if action == "stop" || action == "restart" {
        state.registry.agents.remove(agent_id);
    }

    tracing::info!(user = %user.email, project_id, agent_id, action, "agent lxd state change");
    Ok(())
}

pub async fn start_agent(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    set_lxd_agent_state(&user, &state, &project_id, &agent_id, "start").await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn stop_agent(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    set_lxd_agent_state(&user, &state, &project_id, &agent_id, "stop").await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn restart_agent(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    set_lxd_agent_state(&user, &state, &project_id, &agent_id, "restart").await?;
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

pub async fn list_flavors(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path(project_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    if state.lxd.is_none() {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "LXD is not configured on this server"));
    }

    let flavors: Vec<Value> = Flavor::all().iter().map(Flavor::to_json).collect();
    Ok(Json(flavors))
}

#[derive(serde::Deserialize)]
pub struct CreateLxdAgentBody {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub flavor: String,
}

pub async fn create_lxd_agent_handler(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path(project_id): Path<String>,
    Json(body):       Json<CreateLxdAgentBody>,
) -> axum::response::Response {
    let neo4j = match neo4j_or_err(&state) {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };
    if let Err(e) = require_project_access(neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    let lxd = match state.lxd.as_ref() {
        Some(l) => l,
        None => return err(StatusCode::SERVICE_UNAVAILABLE, "LXD is not configured on this server").into_response(),
    };

    if body.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name is required").into_response();
    }
    let flavor = match Flavor::from_id(&body.flavor) {
        Some(f) => f,
        None => return err(StatusCode::BAD_REQUEST, "unknown flavor").into_response(),
    };

    let (tx, rx) = mpsc::channel::<String>(64);
    let neo4j = Arc::clone(neo4j);
    let lxd = Arc::clone(lxd);
    let server_url = state.server_url.clone();
    let project_id_task = project_id.clone();
    let name = body.name.clone();
    let description = body.description.clone();

    tokio::spawn(async move {
        if let Err(e) = lxd_provision::create_lxd_agent(
            &neo4j, &lxd, &server_url, &project_id_task, &name, &description, flavor, tx,
        ).await {
            tracing::error!(project_id = project_id_task, name, error = ?e, "failed to create LXD-managed agent");
        }
    });

    let stream = ReceiverStream::new(rx).map(|data| Ok::<Event, Infallible>(Event::default().data(data)));
    let mut response = Sse::new(stream).keep_alive(KeepAlive::default()).into_response();
    response.headers_mut().insert(
        header::HeaderName::from_static("x-accel-buffering"),
        header::HeaderValue::from_static("no"),
    );
    response
}

fn port_forward_error_to_api(e: port_forwards::PortForwardError) -> ApiError {
    use port_forwards::PortForwardError::*;
    match e {
        Validation(msg)    => err(StatusCode::BAD_REQUEST, &msg),
        DuplicateRouteName => err(StatusCode::CONFLICT, &e.to_string()),
        NotFound           => err(StatusCode::NOT_FOUND, &e.to_string()),
        Db                 => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

fn port_forward_json(state: &MachineState, f: &port_forwards::PortForward) -> Value {
    json!({
        "id":         f.id,
        "agent_id":   f.agent_id,
        "port":       f.port,
        "route_name": f.route_name,
        "created_at": f.created_at,
        "updated_at": f.updated_at,
        "url":        format!("{}/agents/{}/{}", state.server_url, f.agent_id, f.route_name),
    })
}

pub async fn list_port_forwards(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let forwards = port_forwards::list_for_agent(neo4j, &project_id, &agent_id)
        .await
        .map_err(port_forward_error_to_api)?;

    let result: Vec<Value> = forwards.iter().map(|f| port_forward_json(&state, f)).collect();
    Ok(Json(result))
}

#[derive(serde::Deserialize)]
pub struct CreatePortForwardBody {
    pub port:       u64,
    pub route_name: String,
}

pub async fn create_port_forward(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id)): Path<(String, String)>,
    Json(body):       Json<CreatePortForwardBody>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let port = port_forwards::validate_port(body.port)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;

    let forward = port_forwards::create(neo4j, &project_id, &agent_id, port, &body.route_name)
        .await
        .map_err(port_forward_error_to_api)?;

    Ok(Json(port_forward_json(&state, &forward)))
}

#[derive(serde::Deserialize)]
pub struct UpdatePortForwardBody {
    pub port:       Option<u64>,
    pub route_name: Option<String>,
}

pub async fn update_port_forward(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id, forward_id)): Path<(String, String, String)>,
    Json(body):       Json<UpdatePortForwardBody>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    let port = match body.port {
        Some(p) => Some(port_forwards::validate_port(p).map_err(|e| err(StatusCode::BAD_REQUEST, &e))?),
        None    => None,
    };

    let forward = port_forwards::update(neo4j, &project_id, &agent_id, &forward_id, port, body.route_name)
        .await
        .map_err(port_forward_error_to_api)?;

    Ok(Json(port_forward_json(&state, &forward)))
}

pub async fn delete_port_forward(
    Extension(user):  Extension<Claims>,
    State(state):     State<Arc<MachineState>>,
    Path((project_id, agent_id, forward_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let neo4j = neo4j_or_err(&state)?;
    require_project_access(neo4j, &user.sub, &user.role, &project_id).await?;

    port_forwards::delete(neo4j, &project_id, &agent_id, &forward_id)
        .await
        .map_err(port_forward_error_to_api)?;

    Ok(Json(json!({ "ok": true })))
}

pub(super) async fn require_project_access(
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
