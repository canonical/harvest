use axum::{
    body::Body,
    extract::{ws::Message, Extension, Path, Request, State},
    http::{header::HOST, HeaderName, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use hyper_util::rt::TokioIo;
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

use super::handlers::{err, neo4j_or_err, require_project_access, MachineState};
use super::port_forwards;
use crate::{auth::jwt::Claims, neo4j::Neo4jClient};

const TUNNEL_CLAIM_TIMEOUT_SECS: u64 = 15;
const PROXY_REQUEST_TIMEOUT_SECS: u64 = 30;

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection" | "keep-alive" | "proxy-authenticate" | "proxy-authorization"
            | "te" | "trailer" | "trailers" | "transfer-encoding" | "upgrade"
    )
}

fn api_error(status: StatusCode, msg: &str) -> Response {
    err(status, msg).into_response()
}

async fn lookup_agent_project_id(neo4j: &Neo4jClient, agent_id: &str) -> Option<String> {
    let rows = neo4j.query_read(
        "MATCH (m:Machine {id: $aid}) RETURN m.project_id AS project_id",
        json!({ "aid": agent_id }),
    ).await.ok()?;

    rows.into_iter().next()?["project_id"].as_str().map(str::to_string)
}

type SendFut = Pin<Box<dyn Future<Output = Result<(), mpsc::error::SendError<Message>>> + Send>>;

struct TunnelIo {
    to_caller_rx:  mpsc::Receiver<Message>,
    to_agent_tx:   mpsc::Sender<Message>,
    read_buf:      Vec<u8>,
    eof:           bool,
    pending_write: Option<SendFut>,
}

impl TunnelIo {
    fn new(to_caller_rx: mpsc::Receiver<Message>, to_agent_tx: mpsc::Sender<Message>) -> Self {
        Self {
            to_caller_rx,
            to_agent_tx,
            read_buf: Vec::new(),
            eof: false,
            pending_write: None,
        }
    }
}

impl AsyncRead for TunnelIo {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();

        if !this.read_buf.is_empty() {
            let n = buf.remaining().min(this.read_buf.len());
            buf.put_slice(&this.read_buf[..n]);
            this.read_buf.drain(..n);
            return Poll::Ready(Ok(()));
        }

        if this.eof {
            return Poll::Ready(Ok(()));
        }

        loop {
            return match this.to_caller_rx.poll_recv(cx) {
                Poll::Ready(Some(Message::Binary(data))) => {
                    let n = buf.remaining().min(data.len());
                    buf.put_slice(&data[..n]);
                    if n < data.len() {
                        this.read_buf.extend_from_slice(&data[n..]);
                    }
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Some(Message::Close(_))) | Poll::Ready(None) => {
                    this.eof = true;
                    Poll::Ready(Ok(()))
                }
                Poll::Ready(Some(_)) => continue,
                Poll::Pending => Poll::Pending,
            };
        }
    }
}

impl AsyncWrite for TunnelIo {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let this = self.get_mut();

        loop {
            if let Some(fut) = this.pending_write.as_mut() {
                match fut.as_mut().poll(cx) {
                    Poll::Ready(Ok(())) => this.pending_write = None,
                    Poll::Ready(Err(_)) => {
                        this.pending_write = None;
                        return Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "tunnel closed")));
                    }
                    Poll::Pending => return Poll::Pending,
                }
            } else {
                break;
            }
        }

        let sender = this.to_agent_tx.clone();
        let msg = Message::Binary(buf.to_vec());
        let mut fut: SendFut = Box::pin(async move { sender.send(msg).await });

        match fut.as_mut().poll(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(buf.len())),
            Poll::Ready(Err(_)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "tunnel closed"))),
            Poll::Pending => {
                this.pending_write = Some(fut);
                Poll::Pending
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn build_outbound_request(req: Request, outbound_path: &str, port: u16) -> Result<Request, String> {
    let (parts, body) = req.into_parts();

    let mut path_and_query = outbound_path.to_string();
    if let Some(q) = parts.uri.query() {
        path_and_query.push('?');
        path_and_query.push_str(q);
    }
    let uri: Uri = path_and_query.parse().map_err(|e| format!("invalid outbound path: {e}"))?;

    let mut builder = Request::builder().method(parts.method).uri(uri);
    for (name, value) in parts.headers.iter() {
        if is_hop_by_hop(name) || name == HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder = builder.header(HOST, format!("127.0.0.1:{port}"));

    builder.body(body).map_err(|e| format!("failed to build request: {e}"))
}

async fn proxy_request_inner(
    user:          Claims,
    state:         Arc<MachineState>,
    agent_id:      String,
    route_name:    String,
    outbound_path: String,
    req:           Request,
) -> Response {
    let neo4j = match neo4j_or_err(&state) {
        Ok(n) => n,
        Err(e) => return e.into_response(),
    };

    let project_id = match lookup_agent_project_id(neo4j, &agent_id).await {
        Some(pid) => pid,
        None => return api_error(StatusCode::NOT_FOUND, "agent not found"),
    };

    if let Err(e) = require_project_access(neo4j, &user.sub, &user.role, &project_id).await {
        return e.into_response();
    }

    let forward = match port_forwards::get_by_route(neo4j, &agent_id, &route_name).await {
        Ok(Some(f)) => f,
        Ok(None)    => return api_error(StatusCode::NOT_FOUND, "no such port forward"),
        Err(_)      => return api_error(StatusCode::INTERNAL_SERVER_ERROR, "server error"),
    };

    if req.headers().contains_key(axum::http::header::UPGRADE) {
        return api_error(StatusCode::NOT_IMPLEMENTED, "websocket/upgrade forwarding is not supported");
    }

    if !state.registry.agents.contains_key(&agent_id) {
        return api_error(StatusCode::BAD_GATEWAY, "agent not connected");
    }

    let (session_id, mut to_caller_rx, to_agent_tx) =
        match state.registry.open_tunnel_session(&agent_id, forward.port).await {
            Ok(v)  => v,
            Err(e) => return api_error(StatusCode::BAD_GATEWAY, &e),
        };

    let first = tokio::select! {
        msg = to_caller_rx.recv() => msg,
        _ = tokio::time::sleep(Duration::from_secs(TUNNEL_CLAIM_TIMEOUT_SECS)) => None,
    };

    let Some(first) = first else {
        state.registry.expire_tunnel_session(&session_id);
        return api_error(StatusCode::GATEWAY_TIMEOUT, "agent did not respond");
    };

    match first {
        Message::Text(text) => {
            let parsed: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
            match parsed["type"].as_str() {
                Some("connected") => {}
                Some("error") => {
                    let msg = parsed["message"].as_str().unwrap_or("agent failed to connect to port");
                    return api_error(StatusCode::BAD_GATEWAY, msg);
                }
                _ => return api_error(StatusCode::BAD_GATEWAY, "unexpected tunnel handshake"),
            }
        }
        _ => return api_error(StatusCode::BAD_GATEWAY, "unexpected tunnel handshake"),
    }

    let tunnel_io = TunnelIo::new(to_caller_rx, to_agent_tx);
    let io = TokioIo::new(tunnel_io);

    let (mut send_request, connection) = match hyper::client::conn::http1::handshake(io).await {
        Ok(v)  => v,
        Err(_) => return api_error(StatusCode::BAD_GATEWAY, "failed to establish tunnel connection"),
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let outbound = match build_outbound_request(req, &outbound_path, forward.port) {
        Ok(r)  => r,
        Err(e) => return api_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };

    match send_request.send_request(outbound).await {
        Ok(resp) => {
            let (parts, incoming) = resp.into_parts();
            Response::from_parts(parts, Body::new(incoming))
        }
        Err(_) => api_error(StatusCode::BAD_GATEWAY, "port forward target did not respond"),
    }
}

async fn proxy_request(
    user:          Claims,
    state:         Arc<MachineState>,
    agent_id:      String,
    route_name:    String,
    outbound_path: String,
    req:           Request,
) -> Response {
    match tokio::time::timeout(
        Duration::from_secs(PROXY_REQUEST_TIMEOUT_SECS),
        proxy_request_inner(user, state, agent_id, route_name, outbound_path, req),
    ).await {
        Ok(resp) => resp,
        Err(_)   => api_error(StatusCode::GATEWAY_TIMEOUT, "port forward request timed out"),
    }
}

pub async fn port_forward_proxy_handler(
    Extension(user): Extension<Claims>,
    State(state):    State<Arc<MachineState>>,
    Path((agent_id, route_name)): Path<(String, String)>,
    req: Request,
) -> Response {
    proxy_request(user, state, agent_id, route_name, "/".to_string(), req).await
}

pub async fn port_forward_proxy_handler_subpath(
    Extension(user): Extension<Claims>,
    State(state):    State<Arc<MachineState>>,
    Path((agent_id, route_name, subpath)): Path<(String, String, String)>,
    req: Request,
) -> Response {
    proxy_request(user, state, agent_id, route_name, format!("/{subpath}"), req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_outbound_request_sends_exactly_one_host_header() {
        let req = Request::builder()
            .method("GET")
            .uri("/hello")
            .header(HOST, "harvest-development.thinking-dragon.net")
            .body(Body::empty())
            .unwrap();

        let outbound = build_outbound_request(req, "/", 80).unwrap();
        let hosts: Vec<_> = outbound.headers().get_all(HOST).iter().collect();
        assert_eq!(hosts.len(), 1, "expected exactly one Host header, got {hosts:?}");
        assert_eq!(hosts[0], "127.0.0.1:80");
    }

    #[test]
    fn build_outbound_request_sets_host_even_without_an_inbound_host_header() {
        let req = Request::builder().method("GET").uri("/").body(Body::empty()).unwrap();
        let outbound = build_outbound_request(req, "/", 8080).unwrap();
        assert_eq!(outbound.headers().get(HOST).unwrap(), "127.0.0.1:8080");
    }

    #[test]
    fn build_outbound_request_strips_hop_by_hop_headers() {
        let req = Request::builder()
            .method("GET")
            .uri("/")
            .header(axum::http::header::CONNECTION, "keep-alive")
            .header(axum::http::header::UPGRADE, "websocket")
            .body(Body::empty())
            .unwrap();

        let outbound = build_outbound_request(req, "/", 8080).unwrap();
        assert!(outbound.headers().get(axum::http::header::CONNECTION).is_none());
        assert!(outbound.headers().get(axum::http::header::UPGRADE).is_none());
    }

    #[test]
    fn build_outbound_request_preserves_other_headers() {
        let req = Request::builder()
            .method("GET")
            .uri("/")
            .header("x-custom", "value")
            .body(Body::empty())
            .unwrap();

        let outbound = build_outbound_request(req, "/", 8080).unwrap();
        assert_eq!(outbound.headers().get("x-custom").unwrap(), "value");
    }

    #[test]
    fn build_outbound_request_rewrites_path_and_preserves_query() {
        let req = Request::builder().method("GET").uri("/ignored?a=1").body(Body::empty()).unwrap();
        let outbound = build_outbound_request(req, "/foo/bar", 8080).unwrap();
        assert_eq!(outbound.uri().path(), "/foo/bar");
        assert_eq!(outbound.uri().query(), Some("a=1"));
    }
}
