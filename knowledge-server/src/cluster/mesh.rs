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
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WireMessage;

use crate::cluster::bus::{next_batch, resync_event, ClusterBus, Outbox};
use crate::cluster::node::ClusterNode;
use crate::cluster::peer::{FORWARDED_HEADER, SECRET_HEADER};
use crate::metrics::{MESH_DROPPED, MESH_LINKS, MESH_RESYNCS};

pub const MESH_PATH: &str = "/internal/bus";
const LINK_QUEUE: usize = 4_096;
const RECONNECT_MIN: Duration = Duration::from_millis(100);
const RECONNECT_MAX: Duration = Duration::from_secs(2);
const DEGRADED_AFTER: Duration = Duration::from_secs(5);

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
    health: Arc<LinkHealth>,
}

struct LinkHealth {
    peer: String,
    state: Mutex<(bool, Instant)>,
}

impl LinkHealth {
    fn new(peer: &str) -> Arc<Self> {
        Arc::new(Self {
            peer: peer.to_string(),
            state: Mutex::new((false, Instant::now())),
        })
    }

    fn set(&self, connected: bool) {
        MESH_LINKS.set(&[("peer", &self.peer)], u64::from(connected));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.0 != connected {
            *state = (connected, Instant::now());
        }
    }

    fn is_connected(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0
    }

    fn down_for(&self, now: Instant) -> Option<Duration> {
        let (connected, since) = *self.state.lock().unwrap_or_else(|e| e.into_inner());
        (!connected).then(|| now.duration_since(since))
    }
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
            self.resync("reconnect");
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
            self.resync("gap");
        }
        for (topic, data) in messages {
            self.bus.deliver(&topic, data);
        }
    }

    fn resync(&self, cause: &str) {
        MESH_RESYNCS.inc(&[("cause", cause)]);
        self.bus.deliver_to_all(&resync_event());
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
                MESH_DROPPED.inc(&[("peer", peer)]);
                tracing::warn!(peer, seq, "mesh link is saturated: dropping a batch");
            }
        }
    }

    fn reconcile(self: &Arc<Self>, peers: HashMap<String, String>) {
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        links.retain(|id, link| {
            let keep = peers.get(id) == Some(&link.url);
            if !keep {
                MESH_LINKS.remove(&[("peer", id)]);
            }
            keep
        });
        for (id, url) in peers {
            if links.contains_key(&id) {
                continue;
            }
            let (queue, inbox) = mpsc::channel(LINK_QUEUE);
            let health = LinkHealth::new(&id);
            health.set(false);
            let task = tokio::spawn(run_link(
                Arc::clone(self),
                Arc::clone(&health),
                url.clone(),
                inbox,
            ));
            links.insert(
                id,
                Link {
                    url,
                    queue,
                    task,
                    health,
                },
            );
        }
    }

    fn degraded_peers(&self, now: Instant) -> Vec<String> {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, link)| {
                link.health
                    .down_for(now)
                    .is_some_and(|d| d >= DEGRADED_AFTER)
            })
            .map(|(peer, _)| peer.clone())
            .collect()
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
                mesh.bus.set_degraded(mesh.degraded_peers(Instant::now()));
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

async fn run_link(
    mesh: Arc<Mesh>,
    health: Arc<LinkHealth>,
    url: String,
    mut inbox: mpsc::Receiver<Envelope>,
) {
    let mut last_sent: Option<u64> = None;
    let mut backoff = RECONNECT_MIN;
    loop {
        match stream_to(&mesh, &health, &url, &mut inbox, &mut last_sent).await {
            Ok(()) => return,
            Err(e) => {
                if health.is_connected() {
                    backoff = RECONNECT_MIN;
                }
                health.set(false);
                tracing::debug!(error = %e, url, "mesh link dropped, reconnecting");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(RECONNECT_MAX);
            }
        }
    }
}

async fn stream_to(
    mesh: &Mesh,
    health: &LinkHealth,
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
    health.set(true);
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

    #[test]
    fn a_link_down_for_long_enough_is_degraded() {
        let health = LinkHealth::new("peer-health-test");
        let start = Instant::now();
        health.set(false);
        assert!(health.down_for(start + Duration::from_secs(1)).is_some());
        health.set(true);
        assert!(health.down_for(start + DEGRADED_AFTER * 2).is_none());
        health.set(false);
        assert!(health
            .down_for(Instant::now())
            .is_some_and(|d| d < DEGRADED_AFTER));
    }

    async fn serving_mesh(origin: &str) -> (Arc<Mesh>, String) {
        let (bus, outbox) = ClusterBus::with_outbox(origin);
        let mesh = Arc::new(Mesh {
            bus,
            links: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
            published: AtomicU64::new(0),
            secret: "s".into(),
        });
        tokio::spawn(run_publisher(
            Arc::clone(&mesh),
            Duration::from_millis(5),
            outbox,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(Arc::clone(&mesh));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (mesh, url)
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "stress test"]
    async fn ten_thousand_events_arrive_once_and_in_order() {
        let (sender, _) = serving_mesh("sender").await;
        let (receiver, receiver_url) = serving_mesh("receiver").await;
        let mut events = receiver.bus.subscribe("t");
        sender.reconcile(HashMap::from([("receiver".to_string(), receiver_url)]));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while sender.degraded_peers(Instant::now() + DEGRADED_AFTER).len() == 1 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "the link never connected"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        const COUNT: usize = 10_000;
        let publisher = Arc::clone(&sender);
        tokio::spawn(async move {
            for i in 0..COUNT {
                publisher.bus.publish("t", i.to_string());
                if i % 500 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        });
        let mut next = 0;
        let mut resyncs = 0;
        while next < COUNT {
            let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
                .await
                .expect("events stopped arriving")
                .unwrap();
            if event == resync_event() {
                resyncs += 1;
                continue;
            }
            let value: usize = event.parse().unwrap();
            assert!(value >= next, "event {value} arrived after {next}");
            if value > next {
                assert!(
                    resyncs > 0,
                    "events {next}..{value} were lost without a resync"
                );
            }
            next = value + 1;
        }
        assert_eq!(resyncs, 0, "a healthy link never needs a resync");
    }
}
