use anyhow::Result;
use futures_util::StreamExt as _;
use serde::Deserialize;
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::Mutex;

use crate::{config::Config, executor};

// ── Server → Agent messages (received as SSE data lines) ─────────────────────

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMsg {
    Registered { agent_token: String },
    HelloAck,
    Execute {
        request_id:   String,
        command:      String,
        #[serde(default = "default_timeout")]
        timeout_secs: u64,
    },
    Error { message: String },
    #[serde(other)]
    Unknown,
}

fn default_timeout() -> u64 { 30 }

// ── Helpers ────────────────────────────────────────────────────────────────────

fn hostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".into())
}

fn make_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()?)
}

// ── Main reconnect loop ────────────────────────────────────────────────────────

pub async fn run_with_reconnect(config: Arc<Mutex<Config>>, config_path: &Path) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let cfg = config.lock().await.clone();
        match connect_and_run(&cfg, Arc::clone(&config), config_path).await {
            Ok(()) => tracing::info!("SSE stream ended, reconnecting"),
            Err(e) => tracing::warn!(
                error = %e,
                backoff_secs = backoff.as_secs(),
                "connection failed, retrying"
            ),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

async fn connect_and_run(
    cfg:           &Config,
    shared_config: Arc<Mutex<Config>>,
    config_path:   &Path,
) -> Result<()> {
    let client      = make_client()?;
    let host        = hostname();
    let events_url  = format!("{}/agent/events", cfg.server_url);
    let results_url = format!("{}/agent/results", cfg.server_url);
    let ping_url    = format!("{}/agent/ping", cfg.server_url);

    tracing::info!(url = %events_url, "connecting via SSE");

    let response = client
        .get(&events_url)
        .query(&[("hostname", &host)])
        .header("Authorization", format!("Bearer {}", cfg.agent_token))
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

    // Shared token — updated when we receive a Registered event.
    let shared_token = Arc::new(Mutex::new(cfg.agent_token.clone()));

    // Ping task: POST /agent/ping every 30 s.
    let ping_client = client.clone();
    let ping_token  = Arc::clone(&shared_token);
    let ping_task   = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            let tok = ping_token.lock().await.clone();
            let _ = ping_client
                .post(&ping_url)
                .header("Authorization", format!("Bearer {}", tok))
                .timeout(Duration::from_secs(10))
                .send()
                .await;
        }
    });

    // Consume SSE byte stream, parse events.
    let mut stream = response.bytes_stream();
    let mut buf    = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // SSE messages are separated by blank lines (\n\n).
        while let Some(pos) = buf.find("\n\n") {
            let raw = buf[..pos].to_string();
            buf.drain(..pos + 2);

            for line in raw.lines() {
                if line.starts_with(':') { continue; } // SSE keep-alive comment
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
                        let c2 = client.clone();
                        let u2 = results_url.clone();
                        let t2 = Arc::clone(&shared_token);
                        tokio::spawn(async move {
                            let result = executor::run_command(&command, timeout_secs).await;
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
                            let tok = t2.lock().await.clone();
                            let _ = c2
                                .post(&u2)
                                .header("Authorization", format!("Bearer {}", tok))
                                .timeout(Duration::from_secs(30))
                                .json(&body)
                                .send()
                                .await;
                        });
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
        // Simulate the SSE parser skipping comment lines
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
    fn unknown_server_message_does_not_panic() {
        let data = r#"{"type":"future_unknown_type","extra":"field"}"#;
        let msg: Result<ServerMsg, _> = serde_json::from_str(data);
        // Should deserialize to Unknown without panicking
        assert!(matches!(msg, Ok(ServerMsg::Unknown)));
    }
}
