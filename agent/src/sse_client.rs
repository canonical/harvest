use anyhow::Result;
use futures_util::StreamExt as _;
use serde::Deserialize;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex};

use crate::{
    config::Config, console, executor,
    executor::OutputLine,
    terraform,
    terraform::{TerraformAction, TerraformFlavor},
    tunnel,
};

const PING_INTERVAL_SECS: u64 = 30;
const PING_TIMEOUT_SECS: u64 = 10;
const CONNECT_TIMEOUT_SECS: u64 = 30;
const RESULT_POST_TIMEOUT_SECS: u64 = 30;
const MAX_RECONNECT_BACKOFF_SECS: u64 = 60;
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 30;

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Registered { agent_token: String },
    HelloAck,
    Execute {
        request_id:   String,
        command:      String,
        #[serde(default = "default_command_timeout")]
        timeout_secs: u64,
    },
    RunTerraform {
        request_id:   String,
        artifact_id:  String,
        flavor:       TerraformFlavor,
        action:       TerraformAction,
        files:        std::collections::BTreeMap<String, String>,
        #[serde(default = "default_command_timeout")]
        timeout_secs: u64,
    },
    OpenShell {
        session_id: String,
        cols:       u16,
        rows:       u16,
    },
    OpenTunnel {
        session_id: String,
        port:       u16,
    },
    Uninstall,
    Error { message: String },
    #[serde(other)]
    Unknown,
}

fn default_command_timeout() -> u64 { DEFAULT_COMMAND_TIMEOUT_SECS }

async fn post_result(
    client:      &reqwest::Client,
    results_url: &str,
    token:       &str,
    request_id:  String,
    result:      Result<executor::CommandResult, String>,
) {
    let body = match result {
        Ok(r) => serde_json::json!({
            "request_id": request_id,
            "stdout":     r.stdout,
            "stderr":     r.stderr,
            "exit_code":  r.exit_code,
        }),
        Err(e) => serde_json::json!({
            "request_id": request_id,
            "stdout":     "",
            "stderr":     e,
            "exit_code":  -1,
        }),
    };
    let _ = client
        .post(results_url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(RESULT_POST_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await;
}

async fn post_output(
    client:      &reqwest::Client,
    outputs_url: &str,
    token:       &str,
    request_id:  &str,
    line:        &OutputLine,
) {
    let body = serde_json::json!({
        "request_id": request_id,
        "stream":     if line.stderr { "stderr" } else { "stdout" },
        "line":       line.line,
    });
    let _ = client
        .post(outputs_url)
        .header("Authorization", format!("Bearer {token}"))
        .timeout(Duration::from_secs(RESULT_POST_TIMEOUT_SECS))
        .json(&body)
        .send()
        .await;
}

fn hostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".into())
}

fn make_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()?)
}

pub async fn run_with_reconnect(config: Arc<Mutex<Config>>, config_path: &Path) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let current_config = config.lock().await.clone();
        match connect_and_run(&current_config, Arc::clone(&config), config_path).await {
            Ok(()) => tracing::info!("SSE stream ended, reconnecting"),
            Err(e) => tracing::warn!(
                error = %e,
                backoff_secs = backoff.as_secs(),
                "connection failed, retrying"
            ),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(MAX_RECONNECT_BACKOFF_SECS));
    }
}

async fn connect_and_run(
    config:        &Config,
    shared_config: Arc<Mutex<Config>>,
    config_path:   &Path,
) -> Result<()> {
    let client      = make_client()?;
    let host        = hostname();
    let events_url  = format!("{}/agent/events", config.server_url);
    let results_url = format!("{}/agent/results", config.server_url);
    let outputs_url = format!("{}/agent/output", config.server_url);
    let ping_url    = format!("{}/agent/ping", config.server_url);

    tracing::info!(url = %events_url, "connecting via SSE");

    let response = client
        .get(&events_url)
        .query(&[("hostname", &host)])
        .header("Authorization", format!("Bearer {}", config.agent_token))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "server returned {}: {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }

    tracing::info!("SSE connection established");

    let shared_token = Arc::new(Mutex::new(config.agent_token.clone()));

    let ping_client = client.clone();
    let ping_token  = Arc::clone(&shared_token);
    let ping_task   = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(PING_INTERVAL_SECS));
        interval.tick().await;
        loop {
            interval.tick().await;
            let token = ping_token.lock().await.clone();
            let _ = ping_client
                .post(&ping_url)
                .header("Authorization", format!("Bearer {}", token))
                .timeout(Duration::from_secs(PING_TIMEOUT_SECS))
                .send()
                .await;
        }
    });

    let mut stream = response.bytes_stream();
    let mut byte_buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        byte_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = byte_buffer.find("\n\n") {
            let raw = byte_buffer[..pos].to_string();
            byte_buffer.drain(..pos + 2);

            for line in raw.lines() {
                if line.starts_with(':') { continue; }
                let data = match line.strip_prefix("data: ") {
                    Some(d) => d,
                    None    => continue,
                };

                match serde_json::from_str::<ServerMsg>(data) {
                    Ok(ServerMsg::Registered { agent_token: new_token }) => {
                        tracing::info!("registered — saving permanent token");
                        let mut guard = shared_config.lock().await;
                        guard.agent_token = new_token.clone();
                        if let Err(e) = guard.save(config_path) {
                            tracing::error!(error = %e, "failed to save config");
                        }
                        *shared_token.lock().await = new_token;
                    }

                    Ok(ServerMsg::HelloAck) => {
                        tracing::info!("reconnected to server");
                    }

                    Ok(ServerMsg::Execute { request_id, command, timeout_secs }) => {
                        let command_client = client.clone();
                        let results_url    = results_url.clone();
                        let token_ref      = Arc::clone(&shared_token);
                        tokio::spawn(async move {
                            let result = executor::run_command(&command, timeout_secs).await;
                            let token = token_ref.lock().await.clone();
                            post_result(&command_client, &results_url, &token, request_id, result).await;
                        });
                    }

                    Ok(ServerMsg::RunTerraform { request_id, artifact_id, flavor, action, files, timeout_secs }) => {
                        let command_client = client.clone();
                        let results_url    = results_url.clone();
                        let outputs_url    = outputs_url.clone();
                        let token_ref      = Arc::clone(&shared_token);
                        tokio::spawn(async move {
                            let (line_tx, mut line_rx) = mpsc::unbounded_channel::<OutputLine>();

                            let output_client   = command_client.clone();
                            let output_token_ref = Arc::clone(&token_ref);
                            let output_request_id = request_id.clone();
                            let output_task = tokio::spawn(async move {
                                while let Some(line) = line_rx.recv().await {
                                    let token = output_token_ref.lock().await.clone();
                                    post_output(&output_client, &outputs_url, &token, &output_request_id, &line).await;
                                }
                            });

                            let result = terraform::run(&artifact_id, flavor, action, &files, timeout_secs, Some(line_tx)).await;
                            let _ = output_task.await;

                            let token = token_ref.lock().await.clone();
                            post_result(&command_client, &results_url, &token, request_id, result).await;
                        });
                    }

                    Ok(ServerMsg::OpenShell { session_id, cols, rows }) => {
                        let server_url = config.server_url.clone();
                        let token_ref   = Arc::clone(&shared_token);
                        tokio::spawn(async move {
                            let token = token_ref.lock().await.clone();
                            if let Err(e) = console::run_console_session(&server_url, &token, &session_id, cols, rows).await {
                                tracing::warn!(session_id, error = %e, "console session ended with error");
                            }
                        });
                    }

                    Ok(ServerMsg::OpenTunnel { session_id, port }) => {
                        let server_url = config.server_url.clone();
                        let token_ref   = Arc::clone(&shared_token);
                        tokio::spawn(async move {
                            let token = token_ref.lock().await.clone();
                            if let Err(e) = tunnel::run_tunnel_session(&server_url, &token, &session_id, port).await {
                                tracing::warn!(session_id, error = %e, "tunnel session ended with error");
                            }
                        });
                    }

                    Ok(ServerMsg::Uninstall) => {
                        tracing::info!("uninstall command received — triggering self-removal");
                        ping_task.abort();
                        let _ = std::process::Command::new("/usr/local/bin/uninstall-harvest-agent").spawn();
                        std::process::exit(0);
                    }

                    Ok(ServerMsg::Error { message }) => {
                        tracing::error!(message, "server error");
                    }

                    Ok(ServerMsg::Unknown) | Err(_) => {
                        tracing::debug!(data, "unrecognised SSE data");
                    }
                }
            }
        }
    }

    ping_task.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_keep_alive_comment_is_skipped() {
        let line = ": keep-alive";
        assert!(line.starts_with(':'));
    }

    #[test]
    fn sse_data_prefix_stripped() {
        let line = "data: {\"type\":\"hello_ack\"}";
        let data = line.strip_prefix("data: ").unwrap();
        let msg: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(msg["type"], "hello_ack");
    }

    #[test]
    fn run_terraform_message_deserializes_with_expected_fields() {
        let data = r#"{"type":"run_terraform","request_id":"r1","artifact_id":"art1","flavor":"terraform","action":"plan","files":{"main.tf":"..."},"timeout_secs":120}"#;
        let msg: ServerMsg = serde_json::from_str(data).unwrap();
        match msg {
            ServerMsg::RunTerraform { request_id, artifact_id, flavor, action, files, timeout_secs } => {
                assert_eq!(request_id, "r1");
                assert_eq!(artifact_id, "art1");
                assert_eq!(flavor, TerraformFlavor::Terraform);
                assert_eq!(action, TerraformAction::Plan);
                assert_eq!(files.get("main.tf").unwrap(), "...");
                assert_eq!(timeout_secs, 120);
            }
            other => panic!("expected RunTerraform, got {other:?}"),
        }
    }

    #[test]
    fn run_terraform_message_defaults_timeout_when_omitted() {
        let data = r#"{"type":"run_terraform","request_id":"r1","artifact_id":"art1","flavor":"terragrunt","action":"destroy","files":{}}"#;
        let msg: ServerMsg = serde_json::from_str(data).unwrap();
        match msg {
            ServerMsg::RunTerraform { timeout_secs, flavor, action, .. } => {
                assert_eq!(timeout_secs, DEFAULT_COMMAND_TIMEOUT_SECS);
                assert_eq!(flavor, TerraformFlavor::Terragrunt);
                assert_eq!(action, TerraformAction::Destroy);
            }
            other => panic!("expected RunTerraform, got {other:?}"),
        }
    }

    #[test]
    fn unknown_server_message_does_not_panic() {
        let data = r#"{"type":"future_unknown_type","extra":"field"}"#;
        let msg: Result<ServerMsg, _> = serde_json::from_str(data);
        assert!(matches!(msg, Ok(ServerMsg::Unknown)));
    }
}
