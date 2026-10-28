use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WireMessage;

use crate::cluster::bus::{next_batch, resync_event, ClusterBus, Outbox};
use crate::cluster::node::ClusterNode;
use crate::cluster::peer::{FORWARDED_HEADER, SECRET_HEADER};

pub const MESH_PATH: &str = "/internal/bus";
const LINK_QUEUE: usize = 4_096;
const RECONNECT_MIN: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
enum Frame {
    Batch {
        o: String,
        s: u64,
        m: Vec<(String, String)>,
    },
    Hello {
        o: String,
        h: Option<u64>,
    },
}

#[derive(Clone)]
struct Envelope {
    seq: u64,
    text: Arc<str>,
}

pub struct Mesh {
    bus: Arc<ClusterBus>,
    links: Mutex<HashMap<String, Link>>,
    seen: Mutex<HashMap<String, Option<u64>>>,
    published: AtomicU64,
    secret: String,
}

struct Link {
    url: String,
    queue: mpsc::Sender<Envelope>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Link {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn start(
    node: Arc<ClusterNode>,
    secret: String,
    batch_window: Duration,
) -> (Arc<ClusterBus>, Arc<Mesh>) {
    let (bus, outbox) = ClusterBus::with_outbox(node.node_id().to_string());
    let mesh = Arc::new(Mesh {
        bus: Arc::clone(&bus),
        links: Mutex::new(HashMap::new()),
        seen: Mutex::new(HashMap::new()),
        published: AtomicU64::new(0),
        secret,
    });
    tokio::spawn(run_publisher(Arc::clone(&mesh), batch_window, outbox));
    tokio::spawn(run_membership(Arc::clone(&mesh), node));
    bus.set_listening(true);
    (bus, mesh)
}

pub fn router(mesh: Arc<Mesh>) -> Router {
    Router::new().route(MESH_PATH, get(accept)).with_state(mesh)
}

async fn accept(State(mesh): State<Arc<Mesh>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| mesh.consume(socket))
}

impl Mesh {
    fn origin(&self) -> &str {
        self.bus.node_id()
    }

    async fn consume(self: Arc<Self>, mut socket: WebSocket) {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Text(text) = message {
                self.receive(&text);
            }
        }
    }

    fn receive(&self, text: &str) {
        let Ok(frame) = serde_json::from_str::<Frame>(text) else {
            tracing::warn!("dropping a malformed mesh frame");
            return;
        };
        match frame {
            Frame::Hello { o, h } => self.greet(o, h),
            Frame::Batch { o, s, m } => self.accept_batch(o, s, m),
        }
    }

    fn greet(&self, origin: String, last_sent: Option<u64>) {
        let previous = self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(origin, last_sent);
        if previous.flatten().is_some() && previous.flatten() != last_sent {
            self.bus.deliver_to_all(&resync_event());
        }
    }

    fn accept_batch(&self, origin: String, seq: u64, messages: Vec<(String, String)>) {
        if origin == self.origin() {
            return;
        }
        let previous = self
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(origin, Some(seq));
        if previous.flatten().is_some_and(|last| seq != last + 1) {
            self.bus.deliver_to_all(&resync_event());
        }
        for (topic, data) in messages {
            self.bus.deliver(&topic, data);
        }
    }

    fn broadcast(&self, batch: Vec<(String, String)>) {
        let seq = self.published.fetch_add(1, Ordering::SeqCst) + 1;
        let frame = Frame::Batch {
            o: self.origin().to_string(),
            s: seq,
            m: batch,
        };
        let envelope = Envelope {
            seq,
            text: serde_json::to_string(&frame).unwrap_or_default().into(),
        };
        let links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        for (peer, link) in links.iter() {
            if link.queue.try_send(envelope.clone()).is_err() {
                tracing::warn!(peer, seq, "mesh link is saturated: dropping a batch");
            }
        }
    }

    fn reconcile(self: &Arc<Self>, peers: HashMap<String, String>) {
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        links.retain(|id, link| peers.get(id) == Some(&link.url));
        for (id, url) in peers {
            if links.contains_key(&id) {
                continue;
            }
            let (queue, inbox) = mpsc::channel(LINK_QUEUE);
            let task = tokio::spawn(run_link(Arc::clone(self), url.clone(), inbox));
            links.insert(id, Link { url, queue, task });
        }
    }

    fn connected_peers(&self) -> usize {
        self.links.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

async fn run_publisher(mesh: Arc<Mesh>, batch_window: Duration, mut outbox: Outbox) {
    while let Some(batch) = next_batch(&mut outbox, batch_window).await {
        mesh.broadcast(batch);
    }
}

async fn run_membership(mesh: Arc<Mesh>, node: Arc<ClusterNode>) {
    let mut ticker = tokio::time::interval(node.heartbeat_interval());
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        match node.members().await {
            Ok(members) => {
                let peers = members
                    .into_iter()
                    .filter(|m| m.node_id != node.node_id() && !m.internal_url.is_empty())
                    .map(|m| (m.node_id, m.internal_url))
                    .collect();
                mesh.reconcile(peers);
                tracing::trace!(peers = mesh.connected_peers(), "mesh links reconciled");
            }
            Err(e) => tracing::debug!(error = %e, "mesh could not refresh cluster members"),
        }
    }
}

fn websocket_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let base = base
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| {
            base.strip_prefix("http://")
                .map(|rest| format!("ws://{rest}"))
        })
        .unwrap_or_else(|| base.to_string());
    format!("{base}{MESH_PATH}")
}

async fn run_link(mesh: Arc<Mesh>, url: String, mut inbox: mpsc::Receiver<Envelope>) {
    let mut last_sent: Option<u64> = None;
    let mut backoff = RECONNECT_MIN;
    loop {
        match stream_to(&mesh, &url, &mut inbox, &mut last_sent).await {
            Ok(()) => return,
            Err(e) => {
                tracing::debug!(error = %e, url, "mesh link dropped, reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
    }
}

async fn stream_to(
    mesh: &Mesh,
    url: &str,
    inbox: &mut mpsc::Receiver<Envelope>,
    last_sent: &mut Option<u64>,
) -> Result<()> {
    let mut request = websocket_url(url).into_client_request()?;
    request
        .headers_mut()
        .insert(SECRET_HEADER, mesh.secret.parse()?);
    request
        .headers_mut()
        .insert(FORWARDED_HEADER, mesh.origin().parse()?);
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("connecting the mesh link to {url}"))?;
    let hello = json!({ "o": mesh.origin(), "h": last_sent }).to_string();
    socket.send(WireMessage::Text(hello.into())).await?;
    loop {
        tokio::select! {
            envelope = inbox.recv() => {
                let Some(envelope) = envelope else {
                    return Ok(());
                };
                socket.send(WireMessage::Text(envelope.text.to_string().into())).await?;
                *last_sent = Some(envelope.seq);
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Err(e.into()),
                    None => anyhow::bail!("the peer closed the mesh link"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh(origin: &str) -> Arc<Mesh> {
        let (bus, _outbox) = ClusterBus::with_outbox(origin);
        Arc::new(Mesh {
            bus,
            links: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
            published: AtomicU64::new(0),
            secret: "s".into(),
        })
    }

    fn batch(origin: &str, seq: u64, data: &str) -> String {
        serde_json::to_string(&Frame::Batch {
            o: origin.into(),
            s: seq,
            m: vec![("t".into(), data.into())],
        })
        .unwrap()
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<String>) -> Vec<String> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn websocket_urls_follow_the_scheme() {
        assert_eq!(
            websocket_url("http://10.0.0.1:8081/"),
            "ws://10.0.0.1:8081/internal/bus"
        );
        assert_eq!(websocket_url("https://a"), "wss://a/internal/bus");
    }

    #[test]
    fn frames_parse_by_shape() {
        assert_eq!(
            serde_json::from_str::<Frame>(r#"{"o":"a","h":null}"#).unwrap(),
            Frame::Hello {
                o: "a".into(),
                h: None
            }
        );
        assert!(matches!(
            serde_json::from_str::<Frame>(&batch("a", 1, "x")).unwrap(),
            Frame::Batch { s: 1, .. }
        ));
    }

    #[test]
    fn consecutive_batches_are_delivered_without_resync() {
        let mesh = mesh("self");
        let mut rx = mesh.bus.subscribe("t");
        mesh.receive(r#"{"o":"peer","h":null}"#);
        mesh.receive(&batch("peer", 1, "a"));
        mesh.receive(&batch("peer", 2, "b"));
        assert_eq!(drain(&mut rx), vec!["a", "b"]);
    }

    #[test]
    fn a_gap_in_sequence_triggers_a_resync_and_still_delivers() {
        let mesh = mesh("self");
        let mut rx = mesh.bus.subscribe("t");
        mesh.receive(&batch("peer", 1, "a"));
        mesh.receive(&batch("peer", 3, "c"));
        assert_eq!(
            drain(&mut rx),
            vec!["a".to_string(), resync_event(), "c".to_string()]
        );
    }

    #[test]
    fn a_reconnect_that_lost_batches_triggers_a_resync() {
        let mesh = mesh("self");
        let mut rx = mesh.bus.subscribe("t");
        mesh.receive(&batch("peer", 1, "a"));
        mesh.receive(r#"{"o":"peer","h":2}"#);
        assert_eq!(drain(&mut rx), vec!["a".to_string(), resync_event()]);
    }

    #[test]
    fn a_reconnect_without_losses_is_silent() {
        let mesh = mesh("self");
        let mut rx = mesh.bus.subscribe("t");
        mesh.receive(&batch("peer", 1, "a"));
        mesh.receive(r#"{"o":"peer","h":1}"#);
        mesh.receive(&batch("peer", 2, "b"));
        assert_eq!(drain(&mut rx), vec!["a", "b"]);
    }

    #[test]
    fn batches_from_this_node_are_ignored() {
        let mesh = mesh("self");
        let mut rx = mesh.bus.subscribe("t");
        mesh.receive(&batch("self", 1, "a"));
        assert!(drain(&mut rx).is_empty());
    }
}
