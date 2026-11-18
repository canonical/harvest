use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::LxdClient;

pub type LxdSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const EXIT_WAIT_SECS: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsoleSize {
    pub width: u16,
    pub height: u16,
}

pub struct ConsoleSpec<'a> {
    pub command: &'a [String],
    pub environment: &'a [(String, String)],
    pub cwd: &'a str,
    pub size: ConsoleSize,
}

pub struct Console {
    pub data: LxdSocket,
    pub control: LxdSocket,
    pub operation_id: String,
}

impl LxdClient {
    pub async fn open_console(&self, container: &str, spec: ConsoleSpec<'_>) -> Result<Console> {
        let environment: serde_json::Map<String, Value> =
            spec.environment.iter().map(|(name, value)| (name.clone(), json!(value))).collect();
        let response = self
            .http
            .post(self.url(&format!("/1.0/instances/{container}/exec?{}", self.project_qs())))
            .json(&json!({
                "command": spec.command,
                "environment": environment,
                "cwd": spec.cwd,
                "interactive": true,
                "wait-for-websocket": true,
                "width": spec.size.width,
                "height": spec.size.height,
            }))
            .send()
            .await
            .context("starting an LXD console")?;
        let status = response.status();
        let envelope: Value = response.json().await.context("parsing the LXD console response")?;
        if !status.is_success() {
            bail!("LXD refused the console ({status}): {envelope}");
        }
        let operation = &envelope["metadata"];
        let operation_id = operation["id"].as_str().context("the LXD console has no operation id")?.to_string();
        let secret = |fd: &str| {
            operation["metadata"]["fds"][fd]
                .as_str()
                .map(String::from)
                .with_context(|| format!("the LXD console has no {fd} socket"))
        };
        let data = self.connect_socket(&operation_id, &secret("0")?).await?;
        let control = self.connect_socket(&operation_id, &secret("control")?).await?;
        Ok(Console { data, control, operation_id })
    }

    pub async fn console_exit_code(&self, operation_id: &str) -> Result<Option<i64>> {
        let operation = self
            .wait_for_operation(&json!({ "metadata": { "id": operation_id } }), EXIT_WAIT_SECS)
            .await?;
        Ok(operation["metadata"]["return"].as_i64())
    }

    async fn connect_socket(&self, operation_id: &str, secret: &str) -> Result<LxdSocket> {
        let url = socket_url(&self.base_url, operation_id, secret, &self.project);
        let (socket, _) = tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(self.sockets.clone()))
            .await
            .context("connecting to an LXD console socket")?;
        Ok(socket)
    }
}

fn socket_url(base_url: &str, operation_id: &str, secret: &str, project: &str) -> String {
    let websocket_base = base_url
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| base_url.strip_prefix("http://").map(|rest| format!("ws://{rest}")))
        .unwrap_or_else(|| base_url.to_string());
    format!("{websocket_base}/1.0/operations/{operation_id}/websocket?secret={secret}&project={project}")
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::extract::{Path, Query};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as LxdMessage;

    use super::*;

    #[test]
    fn console_sockets_follow_the_scheme_of_the_lxd_endpoint() {
        assert_eq!(
            socket_url("https://127.0.0.1:8443", "op1", "s3cret", "default"),
            "wss://127.0.0.1:8443/1.0/operations/op1/websocket?secret=s3cret&project=default"
        );
        assert_eq!(socket_url("http://lxd:80", "op", "s", "p"), "ws://lxd:80/1.0/operations/op/websocket?secret=s&project=p");
    }

    async fn echo_or_report(socket: WebSocket, secret: String) {
        let (mut outgoing, mut incoming) = socket.split();
        while let Some(Ok(message)) = incoming.next().await {
            let reply = match message {
                Message::Binary(bytes) if secret == "data" => Message::Binary(bytes),
                Message::Text(text) if secret == "control" => Message::Text(format!("control:{text}")),
                Message::Close(_) => break,
                _ => continue,
            };
            if outgoing.send(reply).await.is_err() {
                break;
            }
        }
    }

    async fn fake_lxd() -> SocketAddr {
        let app = Router::new()
            .route(
                "/1.0/instances/:name/exec",
                post(|Path(name): Path<String>, Json(body): Json<Value>| async move {
                    assert_eq!(name, "hcs-test");
                    assert_eq!(body["interactive"], true);
                    assert_eq!(body["width"], 120);
                    Json(json!({ "metadata": { "id": "op-1", "metadata": { "fds": { "0": "data", "control": "control" } } } }))
                }),
            )
            .route(
                "/1.0/operations/:id/websocket",
                get(|ws: WebSocketUpgrade, Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    let secret = query.get("secret").cloned().unwrap_or_default();
                    ws.on_upgrade(move |socket| echo_or_report(socket, secret))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        address
    }

    #[tokio::test]
    async fn a_console_carries_bytes_on_its_data_socket_and_commands_on_its_control_socket() {
        let address = fake_lxd().await;
        let client = LxdClient::test_client(&format!("http://{address}"));
        let command = vec!["bash".to_string()];
        let spec = ConsoleSpec { command: &command, environment: &[], cwd: "/workspace", size: ConsoleSize { width: 120, height: 40 } };

        let mut console = client.open_console("hcs-test", spec).await.unwrap();

        assert_eq!(console.operation_id, "op-1");
        console.data.send(LxdMessage::binary(b"ls\r".to_vec())).await.unwrap();
        assert_eq!(console.data.next().await.unwrap().unwrap().into_data().to_vec(), b"ls\r");
        console.control.send(LxdMessage::text("resize")).await.unwrap();
        assert_eq!(console.control.next().await.unwrap().unwrap().into_text().unwrap().as_str(), "control:resize");
    }
}
