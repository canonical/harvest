use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt as _, StreamExt as _};
use serde_json::Value;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TungsteniteCloseFrame;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

pub const SECRET_HEADER: &str = "x-harvest-cluster-secret";
pub const FORWARDED_HEADER: &str = "x-harvest-forwarded-by";

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection" | "keep-alive" | "proxy-authenticate" | "proxy-authorization" | "te" | "trailer"
            | "trailers" | "transfer-encoding" | "upgrade" | "host" | "content-length"
            | "sec-websocket-key" | "sec-websocket-version" | "sec-websocket-extensions" | "sec-websocket-accept"
    )
}

pub fn is_forwarded(headers: &HeaderMap) -> bool {
    headers.contains_key(FORWARDED_HEADER)
}

pub struct PeerClient {
    http: reqwest::Client,
    secret: String,
    node_id: String,
}

impl PeerClient {
    pub fn new(node_id: impl Into<String>, secret: impl Into<String>) -> Arc<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("building the peer http client");
        Arc::new(Self { http, secret: secret.into(), node_id: node_id.into() })
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    fn url(base: &str, path_and_query: &str) -> String {
        format!("{}{}", base.trim_end_matches('/'), path_and_query)
    }

    fn internal(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.header(SECRET_HEADER, &self.secret).header(FORWARDED_HEADER, &self.node_id)
    }

    pub async fn get_json(&self, base: &str, path: &str, timeout: Duration) -> Result<Option<Value>> {
        let response = self
            .internal(self.http.get(Self::url(base, path)))
            .timeout(timeout)
            .send()
            .await
            .with_context(|| format!("GET {path} on peer {base}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status().with_context(|| format!("GET {path} on peer {base}"))?;
        Ok(Some(response.json().await?))
    }

    pub async fn post_json(&self, base: &str, path: &str, body: &Value, timeout: Duration) -> Result<(StatusCode, Value)> {
        let response = self
            .internal(self.http.post(Self::url(base, path)))
            .json(body)
            .timeout(timeout)
            .send()
            .await
            .with_context(|| format!("POST {path} on peer {base}"))?;
        let status = StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let value = response.json().await.unwrap_or(Value::Null);
        Ok((status, value))
    }

    pub async fn post_streaming(&self, base: &str, path: &str, body: &Value) -> Result<reqwest::Response> {
        let response = self
            .internal(self.http.post(Self::url(base, path)))
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {path} on peer {base}"))?;
        Ok(response)
    }

    pub async fn forward(&self, base: &str, request: Request) -> Response {
        let (parts, body) = request.into_parts();
        let path_and_query = parts.uri.path_and_query().map(|p| p.as_str().to_string()).unwrap_or_else(|| "/".into());
        let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut builder = self.http.request(method, Self::url(base, &path_and_query));
        for (name, value) in parts.headers.iter() {
            if is_hop_by_hop(name.as_str()) || name.as_str() == SECRET_HEADER || name.as_str() == FORWARDED_HEADER {
                continue;
            }
            builder = builder.header(name.as_str(), value.as_bytes());
        }
        builder = self.internal(builder).body(reqwest::Body::wrap_stream(body.into_data_stream()));
        match builder.send().await {
            Ok(upstream) => {
                let status = StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                let mut response = Response::builder().status(status);
                for (name, value) in upstream.headers().iter() {
                    if is_hop_by_hop(name.as_str()) {
                        continue;
                    }
                    if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(name.as_str().as_bytes()), HeaderValue::from_bytes(value.as_bytes())) {
                        response = response.header(n, v);
                    }
                }
                response
                    .body(Body::from_stream(upstream.bytes_stream()))
                    .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
            }
            Err(e) => {
                tracing::warn!(error = %e, base, "forwarding to peer failed");
                (StatusCode::BAD_GATEWAY, axum::Json(serde_json::json!({ "error": "peer node unavailable" }))).into_response()
            }
        }
    }

    pub fn forward_websocket(
        self: &Arc<Self>,
        base: &str,
        path_and_query: &str,
        headers: &HeaderMap,
        upgrade: WebSocketUpgrade,
        max_message_size: usize,
    ) -> Response {
        let target = Self::url(&base.replacen("http", "ws", 1), path_and_query);
        let mut request = match target.as_str().into_client_request() {
            Ok(r) => r,
            Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
        };
        for (name, value) in headers.iter() {
            if is_hop_by_hop(name.as_str()) || name.as_str() == SECRET_HEADER || name.as_str() == FORWARDED_HEADER {
                continue;
            }
            request.headers_mut().insert(name.clone(), value.clone());
        }
        if let Ok(v) = HeaderValue::from_str(&self.secret) {
            request.headers_mut().insert(SECRET_HEADER, v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.node_id) {
            request.headers_mut().insert(FORWARDED_HEADER, v);
        }
        upgrade
            .max_message_size(max_message_size)
            .on_upgrade(move |socket| async move {
                match tokio_tungstenite::connect_async(request).await {
                    Ok((upstream, _)) => bridge(socket, upstream).await,
                    Err(e) => {
                        tracing::warn!(error = %e, "websocket forward to peer failed");
                        let mut socket = socket;
                        let _ = socket
                            .send(AxumMessage::Text(r#"{"type":"error","message":"peer node unavailable"}"#.into()))
                            .await;
                    }
                }
            })
    }
}

fn to_tungstenite(message: AxumMessage) -> TungsteniteMessage {
    match message {
        AxumMessage::Text(t) => TungsteniteMessage::Text(t.into()),
        AxumMessage::Binary(b) => TungsteniteMessage::Binary(b.into()),
        AxumMessage::Ping(p) => TungsteniteMessage::Ping(p.into()),
        AxumMessage::Pong(p) => TungsteniteMessage::Pong(p.into()),
        AxumMessage::Close(frame) => TungsteniteMessage::Close(frame.map(|f| TungsteniteCloseFrame {
            code: CloseCode::from(f.code),
            reason: f.reason.to_string().into(),
        })),
    }
}

fn to_axum(message: TungsteniteMessage) -> Option<AxumMessage> {
    match message {
        TungsteniteMessage::Text(t) => Some(AxumMessage::Text(t.to_string())),
        TungsteniteMessage::Binary(b) => Some(AxumMessage::Binary(b.to_vec())),
        TungsteniteMessage::Ping(p) => Some(AxumMessage::Ping(p.to_vec())),
        TungsteniteMessage::Pong(p) => Some(AxumMessage::Pong(p.to_vec())),
        TungsteniteMessage::Close(frame) => Some(AxumMessage::Close(frame.map(|f| CloseFrame {
            code: u16::from(f.code),
            reason: f.reason.to_string().into(),
        }))),
        TungsteniteMessage::Frame(_) => None,
    }
}

async fn bridge<S>(socket: WebSocket, upstream: tokio_tungstenite::WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut client_tx, mut client_rx) = socket.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();
    let inbound = async {
        while let Some(Ok(message)) = client_rx.next().await {
            let closing = matches!(message, AxumMessage::Close(_));
            if upstream_tx.send(to_tungstenite(message)).await.is_err() || closing {
                break;
            }
        }
        let _ = upstream_tx.close().await;
    };
    let outbound = async {
        while let Some(Ok(message)) = upstream_rx.next().await {
            let Some(message) = to_axum(message) else { continue };
            let closing = matches!(message, AxumMessage::Close(_));
            if client_tx.send(message).await.is_err() || closing {
                break;
            }
        }
        let _ = client_tx.close().await;
    };
    tokio::select! {
        _ = inbound => {}
        _ = outbound => {}
    }
}

pub async fn require_secret(State(secret): State<Arc<String>>, request: Request, next: Next) -> Response {
    let presented = request.headers().get(SECRET_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
    if secret.is_empty() || !constant_time_eq(presented.as_bytes(), secret.as_bytes()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use axum::routing::{any, get, post};
    use axum::Router;
    use serde_json::json;
    use std::collections::HashMap;

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn internal_calls_carry_the_secret_and_the_calling_node() {
        let router = Router::new().route(
            "/internal/echo",
            post(|headers: HeaderMap, axum::Json(body): axum::Json<Value>| async move {
                axum::Json(json!({
                    "secret": headers.get(SECRET_HEADER).and_then(|v| v.to_str().ok()),
                    "from": headers.get(FORWARDED_HEADER).and_then(|v| v.to_str().ok()),
                    "body": body,
                }))
            }),
        );
        let base = serve(router).await;
        let peer = PeerClient::new("node-a", "s3cret");
        let (status, value) = peer.post_json(&base, "/internal/echo", &json!({ "x": 1 }), Duration::from_secs(5)).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["secret"], "s3cret");
        assert_eq!(value["from"], "node-a");
        assert_eq!(value["body"]["x"], 1);
    }

    #[tokio::test]
    async fn get_json_maps_not_found_to_none() {
        let base = serve(Router::new()).await;
        let peer = PeerClient::new("node-a", "s");
        assert!(peer.get_json(&base, "/missing", Duration::from_secs(5)).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn forwarding_preserves_method_path_query_headers_and_body() {
        let router = Router::new().route(
            "/projects/:pid/thing",
            any(|method: axum::http::Method, headers: HeaderMap, Query(q): Query<HashMap<String, String>>, body: String| async move {
                (
                    StatusCode::CREATED,
                    [("x-upstream", "yes")],
                    axum::Json(json!({
                        "method": method.as_str(),
                        "q": q.get("a"),
                        "cookie": headers.get("cookie").and_then(|v| v.to_str().ok()),
                        "forwarded": headers.get(FORWARDED_HEADER).and_then(|v| v.to_str().ok()),
                        "body": body,
                    })),
                )
            }),
        );
        let base = serve(router).await;
        let peer = PeerClient::new("node-a", "s");
        let request = Request::builder()
            .method("PUT")
            .uri("/projects/p1/thing?a=42")
            .header("cookie", "token=abc")
            .header(FORWARDED_HEADER, "spoofed")
            .body(Body::from("payload"))
            .unwrap();
        let response = peer.forward(&base, request).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers().get("x-upstream").unwrap(), "yes");
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["method"], "PUT");
        assert_eq!(value["q"], "42");
        assert_eq!(value["cookie"], "token=abc");
        assert_eq!(value["forwarded"], "node-a");
        assert_eq!(value["body"], "payload");
    }

    #[tokio::test]
    async fn forwarding_to_an_unreachable_peer_returns_bad_gateway() {
        let peer = PeerClient::new("node-a", "s");
        let request = Request::builder().uri("/x").body(Body::empty()).unwrap();
        let response = peer.forward("http://127.0.0.1:1", request).await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn websocket_forwarding_bridges_frames_both_ways() {
        let upstream = Router::new().route(
            "/ws",
            get(|ws: WebSocketUpgrade, headers: HeaderMap| async move {
                let cookie = headers.get("cookie").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                ws.on_upgrade(move |mut socket| async move {
                    let _ = socket.send(AxumMessage::Text(format!("hello {cookie}"))).await;
                    while let Some(Ok(message)) = socket.next().await {
                        match message {
                            AxumMessage::Binary(b) => { let _ = socket.send(AxumMessage::Binary(b.iter().rev().cloned().collect())).await; }
                            AxumMessage::Close(_) => break,
                            _ => {}
                        }
                    }
                })
            }),
        );
        let upstream_base = serve(upstream).await;
        let peer = PeerClient::new("node-a", "s");
        let front = Router::new().route(
            "/ws",
            get(move |ws: WebSocketUpgrade, headers: HeaderMap| {
                let peer = Arc::clone(&peer);
                let upstream_base = upstream_base.clone();
                async move { peer.forward_websocket(&upstream_base, "/ws", &headers, ws, 64 * 1024) }
            }),
        );
        let front_base = serve(front).await;
        let mut request = format!("{}/ws", front_base.replacen("http", "ws", 1)).into_client_request().unwrap();
        request.headers_mut().insert("cookie", HeaderValue::from_static("token=abc"));
        let (mut client, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let greeting = client.next().await.unwrap().unwrap();
        assert_eq!(greeting.into_text().unwrap().as_str(), "hello token=abc");
        client.send(TungsteniteMessage::Binary(vec![1u8, 2, 3].into())).await.unwrap();
        let echoed = client.next().await.unwrap().unwrap();
        assert_eq!(echoed.into_data().to_vec(), vec![3u8, 2, 1]);
    }

    #[tokio::test]
    async fn the_secret_middleware_rejects_missing_or_wrong_secrets() {
        let secret = Arc::new("right".to_string());
        let router = Router::new()
            .route("/internal/ok", get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(secret, require_secret));
        let base = serve(router).await;
        let http = reqwest::Client::new();
        assert_eq!(http.get(format!("{base}/internal/ok")).send().await.unwrap().status(), 401);
        assert_eq!(http.get(format!("{base}/internal/ok")).header(SECRET_HEADER, "wrong").send().await.unwrap().status(), 401);
        assert_eq!(http.get(format!("{base}/internal/ok")).header(SECRET_HEADER, "right").send().await.unwrap().status(), 200);
    }

    #[test]
    fn constant_time_comparison_matches_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
