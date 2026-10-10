use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{command_result_json, MachineRegistry, ResultBody, TerraformAction, TerraformFlavor};

#[derive(Deserialize)]
struct ExecuteRequest {
    command:      String,
    timeout_secs: u64,
}

#[derive(Deserialize)]
struct TerraformRequest {
    artifact_id:  String,
    flavor:       TerraformFlavor,
    action:       TerraformAction,
    files:        BTreeMap<String, String>,
    timeout_secs: u64,
}

#[derive(Deserialize)]
struct OutputRequest {
    request_id: String,
    stream:     String,
    line:       String,
}

fn not_here() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "agent not connected to this node" }))).into_response()
}

async fn execute(
    State(registry): State<Arc<MachineRegistry>>,
    Path(agent_id): Path<String>,
    Json(body): Json<ExecuteRequest>,
) -> Response {
    if !registry.is_local(&agent_id) {
        return not_here();
    }
    let result = registry.execute_local(&agent_id, body.command, body.timeout_secs).await;
    Json(command_result_json(&result)).into_response()
}

fn ndjson_line(value: &Value) -> Result<Bytes, Infallible> {
    let mut line = value.to_string();
    line.push('\n');
    Ok(Bytes::from(line))
}

async fn terraform(
    State(registry): State<Arc<MachineRegistry>>,
    Path(agent_id): Path<String>,
    Json(body): Json<TerraformRequest>,
) -> Response {
    if !registry.is_local(&agent_id) {
        return not_here();
    }
    let (lines_tx, lines_rx) = mpsc::channel::<Result<Bytes, Infallible>>(256);
    let (output_tx, mut output_rx) = mpsc::channel::<Value>(256);
    let relay_tx = lines_tx.clone();
    let relay = tokio::spawn(async move {
        while let Some(output) = output_rx.recv().await {
            if relay_tx.send(ndjson_line(&json!({ "output": output }))).await.is_err() {
                break;
            }
        }
    });
    tokio::spawn(async move {
        let result = registry
            .execute_terraform_local(&agent_id, body.artifact_id, body.flavor, body.action, body.files, body.timeout_secs, Some(output_tx))
            .await;
        let _ = relay.await;
        let _ = lines_tx.send(ndjson_line(&json!({ "result": command_result_json(&result) }))).await;
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(lines_rx)))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn uninstall(State(registry): State<Arc<MachineRegistry>>, Path(agent_id): Path<String>) -> Response {
    if !registry.is_local(&agent_id) {
        return not_here();
    }
    registry.uninstall(&agent_id).await;
    Json(json!({ "ok": true })).into_response()
}

async fn disconnect(State(registry): State<Arc<MachineRegistry>>, Path(agent_id): Path<String>) -> Response {
    if !registry.is_local(&agent_id) {
        return not_here();
    }
    registry.disconnect(&agent_id).await;
    Json(json!({ "ok": true })).into_response()
}

async fn results(State(registry): State<Arc<MachineRegistry>>, Json(body): Json<ResultBody>) -> Response {
    let resolved = registry.resolve_pending(body);
    Json(json!({ "resolved": resolved })).into_response()
}

async fn output(State(registry): State<Arc<MachineRegistry>>, Json(body): Json<OutputRequest>) -> Response {
    let relayed = registry.relay_output(&body.request_id, &body.stream, &body.line);
    Json(json!({ "relayed": relayed })).into_response()
}

pub fn router(registry: Arc<MachineRegistry>) -> Router {
    Router::new()
        .route("/internal/agents/results", post(results))
        .route("/internal/agents/output", post(output))
        .route("/internal/agents/:agent_id/execute", post(execute))
        .route("/internal/agents/:agent_id/terraform", post(terraform))
        .route("/internal/agents/:agent_id/uninstall", post(uninstall))
        .route("/internal/agents/:agent_id/disconnect", post(disconnect))
        .with_state(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machines::{CommandResult, ConnectedAgent, ServerToAgent};
    use axum::body::to_bytes;
    use tower::ServiceExt as _;

    fn register(registry: &MachineRegistry, agent_id: &str) -> mpsc::Receiver<ServerToAgent> {
        let (tx, rx) = mpsc::channel(8);
        registry.agents.insert(agent_id.into(), ConnectedAgent {
            id: agent_id.into(), project_id: "p".into(), hostname: "h".into(),
            connected_at: chrono::Utc::now(), sender: tx,
        });
        rx
    }

    fn post_json(path: &str, body: Value) -> axum::http::Request<Body> {
        axum::http::Request::post(path)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn execute_for_an_agent_on_another_node_is_not_found() {
        let response = router(MachineRegistry::new())
            .oneshot(post_json("/internal/agents/ghost/execute", json!({ "command": "true", "timeout_secs": 1 })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn execute_runs_against_the_local_agent_and_returns_its_result() {
        let registry = MachineRegistry::new();
        let mut rx = register(&registry, "a1");
        let responder = Arc::clone(&registry);
        tokio::spawn(async move {
            if let Some(ServerToAgent::Execute { request_id, .. }) = rx.recv().await {
                responder.resolve_pending(ResultBody { request_id, stdout: "hi".into(), stderr: String::new(), exit_code: 0 });
            }
        });
        let response = router(registry)
            .oneshot(post_json("/internal/agents/a1/execute", json!({ "command": "echo hi", "timeout_secs": 5 })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
        assert_eq!(value["stdout"], "hi");
        assert_eq!(value["exit_code"], 0);
    }

    #[tokio::test]
    async fn terraform_streams_output_lines_then_the_result() {
        let registry = MachineRegistry::new();
        let mut rx = register(&registry, "a1");
        let responder = Arc::clone(&registry);
        tokio::spawn(async move {
            if let Some(ServerToAgent::RunTerraform { request_id, .. }) = rx.recv().await {
                responder.relay_output(&request_id, "stdout", "Initializing...");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                responder.resolve_pending(ResultBody { request_id, stdout: "done".into(), stderr: String::new(), exit_code: 0 });
            }
        });
        let response = router(registry)
            .oneshot(post_json("/internal/agents/a1/terraform", json!({
                "artifact_id": "art", "flavor": "terraform", "action": "plan", "files": {}, "timeout_secs": 5,
            })))
            .await
            .unwrap();
        let text = String::from_utf8(to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(lines[0]["output"]["line"], "Initializing...");
        assert_eq!(lines[1]["result"]["stdout"], "done");
    }

    #[tokio::test]
    async fn forwarded_results_resolve_the_local_pending_command() {
        let registry = MachineRegistry::new();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<CommandResult, String>>();
        registry.pending.insert("r1".into(), crate::machines::PendingResult { tx, deadline: std::time::Instant::now() });
        let response = router(Arc::clone(&registry))
            .oneshot(post_json("/internal/agents/results", json!({ "request_id": "r1", "stdout": "ok", "stderr": "", "exit_code": 0 })))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(rx.await.unwrap().unwrap().stdout, "ok");
    }
}
