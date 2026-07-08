use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const PTY_READ_BUF_SIZE: usize = 8192;
const DEFAULT_TERM: &str = "xterm-256color";

#[derive(serde::Deserialize)]
struct ControlMsg {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    cols: u16,
    #[serde(default)]
    rows: u16,
}

fn console_ws_url(server_url: &str, session_id: &str) -> String {
    let ws_base = if let Some(rest) = server_url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = server_url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        server_url.to_string()
    };
    format!("{}/agent/console/{session_id}", ws_base.trim_end_matches('/'))
}

pub async fn run_console_session(
    server_url:  &str,
    agent_token: &str,
    session_id:  &str,
    cols:        u16,
    rows:        u16,
) -> Result<()> {
    let url = console_ws_url(server_url, session_id);
    let mut request = url.into_client_request().context("building console request")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {agent_token}").parse().context("invalid agent token header")?,
    );

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("connecting console websocket")?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    ws_tx.send(Message::text(r#"{"type":"ready"}"#)).await.ok();

    let (pty, pts) = pty_process::open().context("allocating pty")?;
    pty.resize(pty_process::Size::new(rows, cols)).context("sizing pty")?;

    let child = pty_process::Command::new("bash")
        .env("TERM", DEFAULT_TERM)
        .kill_on_drop(true)
        .spawn(pts)
        .context("spawning shell")?;

    let (mut pty_read, mut pty_write) = pty.into_split();

    let reader = tokio::spawn(async move {
        let mut child = child;
        let mut buf = [0u8; PTY_READ_BUF_SIZE];
        loop {
            match pty_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_tx.send(Message::binary(buf[..n].to_vec())).await.is_err() {
                        return;
                    }
                }
            }
        }

        let code = child.wait().await.ok().and_then(|s| s.code());
        let payload = match code {
            Some(c) => format!(r#"{{"type":"exited","code":{c}}}"#),
            None     => r#"{"type":"exited","code":null}"#.to_string(),
        };
        let _ = ws_tx.send(Message::text(payload)).await;
        let _ = ws_tx.close().await;
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Binary(data) => {
                if pty_write.write_all(&data).await.is_err() {
                    break;
                }
            }
            Message::Text(text) => {
                if let Ok(ctrl) = serde_json::from_str::<ControlMsg>(&text) {
                    if ctrl.kind == "resize" {
                        let _ = pty_write.resize(pty_process::Size::new(ctrl.rows, ctrl.cols));
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    reader.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_ws_url_converts_https_to_wss() {
        let url = console_ws_url("https://harvest.example.com", "sess-1");
        assert_eq!(url, "wss://harvest.example.com/agent/console/sess-1");
    }

    #[test]
    fn console_ws_url_converts_http_to_ws() {
        let url = console_ws_url("http://localhost:8080", "sess-1");
        assert_eq!(url, "ws://localhost:8080/agent/console/sess-1");
    }

    #[test]
    fn console_ws_url_strips_trailing_slash() {
        let url = console_ws_url("https://harvest.example.com/", "sess-1");
        assert_eq!(url, "wss://harvest.example.com/agent/console/sess-1");
    }

    #[tokio::test]
    async fn pty_echoes_command_output() {
        let (pty, pts) = pty_process::open().unwrap();
        pty.resize(pty_process::Size::new(24, 80)).unwrap();
        let mut child = pty_process::Command::new("bash")
            .kill_on_drop(true)
            .spawn(pts)
            .unwrap();

        let (mut read_half, mut write_half) = pty.into_split();
        write_half.write_all(b"echo hello-from-pty\n").await.unwrap();

        let mut buf = [0u8; 4096];
        let mut collected = String::new();
        loop {
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), read_half.read(&mut buf))
                .await.unwrap().unwrap();
            if n == 0 { break; }
            collected.push_str(&String::from_utf8_lossy(&buf[..n]));
            if collected.contains("hello-from-pty\r\n") { break; }
        }

        assert!(collected.contains("hello-from-pty"), "got: {collected:?}");

        write_half.write_all(b"exit\n").await.unwrap();
        let _ = child.wait().await;
    }

    #[tokio::test]
    async fn pty_shell_has_term_set_for_ncurses_tools() {
        let (pty, pts) = pty_process::open().unwrap();
        pty.resize(pty_process::Size::new(24, 80)).unwrap();
        let mut child = pty_process::Command::new("bash")
            .env("TERM", DEFAULT_TERM)
            .kill_on_drop(true)
            .spawn(pts)
            .unwrap();

        let (mut read_half, mut write_half) = pty.into_split();
        write_half.write_all(b"echo TERM-IS-$TERM\n").await.unwrap();

        let mut buf = [0u8; 4096];
        let mut collected = String::new();
        loop {
            let n = tokio::time::timeout(std::time::Duration::from_secs(5), read_half.read(&mut buf))
                .await.unwrap().unwrap();
            if n == 0 { break; }
            collected.push_str(&String::from_utf8_lossy(&buf[..n]));
            if collected.contains(&format!("TERM-IS-{DEFAULT_TERM}\r\n")) { break; }
        }

        assert!(collected.contains(&format!("TERM-IS-{DEFAULT_TERM}")), "got: {collected:?}");

        write_half.write_all(b"exit\n").await.unwrap();
        let _ = child.wait().await;
    }
}
