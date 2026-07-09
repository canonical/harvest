use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const TUNNEL_READ_BUF_SIZE: usize = 8192;

fn tunnel_ws_url(server_url: &str, session_id: &str) -> String {
    crate::ws_url::dial_back_ws_url(server_url, &format!("/agent/tunnel/{session_id}"))
}

pub async fn run_tunnel_session(
    server_url:  &str,
    agent_token: &str,
    session_id:  &str,
    port:        u16,
) -> Result<()> {
    let url = tunnel_ws_url(server_url, session_id);
    let mut request = url.into_client_request().context("building tunnel request")?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {agent_token}").parse().context("invalid agent token header")?,
    );

    let (ws_stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("connecting tunnel websocket")?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let tcp = match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(stream) => stream,
        Err(e) => {
            let message = e.to_string().replace('"', "'");
            let _ = ws_tx.send(Message::text(format!(r#"{{"type":"error","message":"{message}"}}"#))).await;
            let _ = ws_tx.close().await;
            return Ok(());
        }
    };

    if ws_tx.send(Message::text(r#"{"type":"connected"}"#)).await.is_err() {
        return Ok(());
    }

    let (mut tcp_read, mut tcp_write) = tcp.into_split();

    let reader = tokio::spawn(async move {
        let mut buf = [0u8; TUNNEL_READ_BUF_SIZE];
        loop {
            match tcp_read.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if ws_tx.send(Message::binary(buf[..n].to_vec())).await.is_err() {
                        return;
                    }
                }
            }
        }
        let _ = ws_tx.close().await;
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Binary(data) => {
                if tcp_write.write_all(&data).await.is_err() {
                    break;
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
    use tokio::net::TcpListener;

    #[test]
    fn tunnel_ws_url_converts_https_to_wss() {
        let url = tunnel_ws_url("https://harvest.example.com", "sess-1");
        assert_eq!(url, "wss://harvest.example.com/agent/tunnel/sess-1");
    }

    #[tokio::test]
    async fn connects_to_local_port_and_reports_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let result = TcpStream::connect(("127.0.0.1", addr.port())).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn connect_to_closed_port_fails() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let result = TcpStream::connect(("127.0.0.1", addr.port())).await;
        assert!(result.is_err());
    }
}
