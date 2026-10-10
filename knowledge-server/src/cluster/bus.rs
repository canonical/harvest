use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use harvest_db::Db;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch};

pub const CHANNEL: &str = "harvest_bus";
const MAX_NOTIFY_BYTES: usize = 7_000;
const MAX_BATCH_MESSAGES: usize = 500;
const TOPIC_CAPACITY: usize = 4_096;

pub fn resync_event() -> String {
    json!({ "type": "resync" }).to_string()
}

pub struct ClusterBus {
    node_id: String,
    topics: Mutex<HashMap<String, broadcast::Sender<String>>>,
    outbox: Option<mpsc::UnboundedSender<(String, String)>>,
    listening: watch::Sender<bool>,
    listener_pid: Mutex<Option<i32>>,
}

impl ClusterBus {
    pub fn local(node_id: impl Into<String>) -> Arc<Self> {
        let (listening, _) = watch::channel(true);
        Arc::new(Self {
            node_id: node_id.into(),
            topics: Mutex::new(HashMap::new()),
            outbox: None,
            listening,
            listener_pid: Mutex::new(None),
        })
    }

    pub fn start(db: Arc<Db>, node_id: String, batch_window: Duration) -> Arc<Self> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (listening, _) = watch::channel(false);
        let bus = Arc::new(Self {
            node_id: node_id.clone(),
            topics: Mutex::new(HashMap::new()),
            outbox: Some(sender),
            listening,
            listener_pid: Mutex::new(None),
        });
        tokio::spawn(run_publisher(Arc::clone(&db), node_id, batch_window, receiver));
        tokio::spawn(run_listener(Arc::downgrade(&bus), db));
        bus
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn subscribe(&self, topic: &str) -> broadcast::Receiver<String> {
        let mut topics = self.topics.lock().unwrap_or_else(|e| e.into_inner());
        topics
            .entry(topic.to_string())
            .or_insert_with(|| broadcast::channel(TOPIC_CAPACITY).0)
            .subscribe()
    }

    pub fn publish(&self, topic: &str, data: String) {
        self.deliver(topic, data.clone());
        if let Some(outbox) = &self.outbox {
            let _ = outbox.send((topic.to_string(), data));
        }
    }

    pub fn publish_local(&self, topic: &str, data: String) {
        self.deliver(topic, data);
    }

    pub async fn wait_until_listening(&self) {
        let mut receiver = self.listening.subscribe();
        let _ = receiver.wait_for(|listening| *listening).await;
    }

    pub fn is_listening(&self) -> bool {
        *self.listening.borrow()
    }

    pub async fn listener_pid(&self) -> Option<i32> {
        *self.listener_pid.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn deliver(&self, topic: &str, data: String) {
        let mut topics = self.topics.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = topics.get(topic) {
            if sender.send(data).is_err() && sender.receiver_count() == 0 {
                topics.remove(topic);
            }
        }
    }

    fn deliver_to_all(&self, data: &str) {
        let topics = self.topics.lock().unwrap_or_else(|e| e.into_inner());
        for sender in topics.values() {
            let _ = sender.send(data.to_string());
        }
    }

    fn deliver_envelope(&self, envelope: &Value) {
        if envelope["o"].as_str() == Some(self.node_id.as_str()) {
            return;
        }
        let Some(messages) = envelope["m"].as_array() else { return };
        for message in messages {
            if let (Some(topic), Some(data)) = (message[0].as_str(), message[1].as_str()) {
                self.deliver(topic, data.to_string());
            }
        }
    }
}

enum Encoded {
    Inline(String),
    Stored(String),
}

fn envelope(node_id: &str, messages: &[(String, String)]) -> String {
    json!({ "o": node_id, "m": messages.iter().map(|(t, d)| json!([t, d])).collect::<Vec<_>>() }).to_string()
}

fn encode(node_id: &str, batch: Vec<(String, String)>) -> Vec<Encoded> {
    let mut out = Vec::new();
    let mut pending: Vec<(String, String)> = Vec::new();
    for message in batch {
        let single = envelope(node_id, std::slice::from_ref(&message));
        if single.len() > MAX_NOTIFY_BYTES {
            if !pending.is_empty() {
                out.push(Encoded::Inline(envelope(node_id, &pending)));
                pending.clear();
            }
            out.push(Encoded::Stored(single));
            continue;
        }
        pending.push(message);
        if envelope(node_id, &pending).len() > MAX_NOTIFY_BYTES {
            let last = pending.pop().expect("just pushed");
            out.push(Encoded::Inline(envelope(node_id, &pending)));
            pending.clear();
            pending.push(last);
        }
    }
    if !pending.is_empty() {
        out.push(Encoded::Inline(envelope(node_id, &pending)));
    }
    out
}

async fn publisher_client(db: &Db) -> Result<harvest_db::tokio_postgres::Client> {
    let client = db.dedicated_client().await?;
    client.batch_execute("SET synchronous_commit = off").await?;
    Ok(client)
}

async fn send_one(client: &harvest_db::tokio_postgres::Client, node_id: &str, item: &Encoded) -> Result<()> {
    let payload = match item {
        Encoded::Inline(text) => text.clone(),
        Encoded::Stored(body) => {
            let id: i64 = client
                .query_one("INSERT INTO bus_payloads (body) VALUES ($1) RETURNING id", &[body])
                .await
                .context("storing large bus payload")?
                .get(0);
            json!({ "o": node_id, "r": id }).to_string()
        }
    };
    let channel = CHANNEL.to_string();
    client.execute("SELECT pg_notify($1, $2)", &[&channel, &payload]).await?;
    Ok(())
}

async fn run_publisher(
    db: Arc<Db>,
    node_id: String,
    batch_window: Duration,
    mut receiver: mpsc::UnboundedReceiver<(String, String)>,
) {
    let mut client: Option<harvest_db::tokio_postgres::Client> = None;
    while let Some(first) = receiver.recv().await {
        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + batch_window;
        while batch.len() < MAX_BATCH_MESSAGES {
            match tokio::time::timeout_at(deadline, receiver.recv()).await {
                Ok(Some(message)) => batch.push(message),
                _ => break,
            }
        }
        for item in encode(&node_id, batch) {
            let mut attempts = 0;
            loop {
                attempts += 1;
                if client.as_ref().map_or(true, |c| c.is_closed()) {
                    client = match publisher_client(&db).await {
                        Ok(c) => Some(c),
                        Err(e) => {
                            tracing::warn!(error = %e, "cluster bus publisher could not connect");
                            None
                        }
                    };
                }
                let result = match &client {
                    Some(c) => send_one(c, &node_id, &item).await,
                    None => Err(anyhow::anyhow!("no publisher connection")),
                };
                match result {
                    Ok(()) => break,
                    Err(e) if attempts < 3 => {
                        tracing::warn!(error = %e, attempts, "cluster bus publish failed, retrying");
                        client = None;
                        tokio::time::sleep(Duration::from_millis(100 * attempts)).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "cluster bus dropped an event batch");
                        client = None;
                        break;
                    }
                }
            }
        }
    }
}

async fn resolve_envelope(db: &Db, payload: &str) -> Option<Value> {
    let envelope: Value = serde_json::from_str(payload).ok()?;
    let Some(id) = envelope["r"].as_i64() else { return Some(envelope) };
    let rows = db
        .query("SELECT body FROM bus_payloads WHERE id = $id", json!({ "id": id }))
        .await
        .ok()?;
    let body = rows.into_iter().next()?["body"].as_str()?.to_string();
    serde_json::from_str(&body).ok()
}

async fn run_listener(bus: Weak<ClusterBus>, db: Arc<Db>) {
    let mut connected_before = false;
    let mut backoff = Duration::from_millis(100);
    loop {
        if bus.strong_count() == 0 {
            return;
        }
        match db.listen(&[CHANNEL]).await {
            Ok(mut listener) => {
                backoff = Duration::from_millis(100);
                {
                    let Some(bus) = bus.upgrade() else { return };
                    *bus.listener_pid.lock().unwrap_or_else(|e| e.into_inner()) = Some(listener.backend_pid());
                    if connected_before {
                        bus.deliver_to_all(&resync_event());
                    }
                    bus.listening.send_replace(true);
                }
                connected_before = true;
                while let Some(notification) = listener.recv().await {
                    let Some(envelope) = resolve_envelope(&db, &notification.payload).await else { continue };
                    let Some(bus) = bus.upgrade() else { return };
                    bus.deliver_envelope(&envelope);
                }
                let Some(bus) = bus.upgrade() else { return };
                *bus.listener_pid.lock().unwrap_or_else(|e| e.into_inner()) = None;
                bus.listening.send_replace(false);
                tracing::warn!("cluster bus listener disconnected, reconnecting");
            }
            Err(e) => {
                tracing::warn!(error = %e, "cluster bus listener could not connect");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_messages_share_one_notification() {
        let batch: Vec<(String, String)> = (0..10).map(|i| ("t".to_string(), format!("m{i}"))).collect();
        let encoded = encode("n", batch);
        assert_eq!(encoded.len(), 1);
        assert!(matches!(&encoded[0], Encoded::Inline(text) if text.len() <= MAX_NOTIFY_BYTES));
    }

    #[test]
    fn batches_split_before_reaching_the_notify_limit() {
        let batch: Vec<(String, String)> = (0..100).map(|i| ("t".to_string(), format!("{i}:{}", "x".repeat(300)))).collect();
        let encoded = encode("n", batch);
        assert!(encoded.len() > 1);
        let mut seen = Vec::new();
        for item in &encoded {
            let Encoded::Inline(text) = item else { panic!("unexpected stored payload") };
            assert!(text.len() <= MAX_NOTIFY_BYTES);
            let value: Value = serde_json::from_str(text).unwrap();
            for m in value["m"].as_array().unwrap() {
                seen.push(m[1].as_str().unwrap().split(':').next().unwrap().parse::<usize>().unwrap());
            }
        }
        assert_eq!(seen, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn oversized_messages_are_stored_without_reordering() {
        let batch = vec![
            ("t".to_string(), "a".to_string()),
            ("t".to_string(), "b".repeat(10_000)),
            ("t".to_string(), "c".to_string()),
        ];
        let encoded = encode("n", batch);
        assert_eq!(encoded.len(), 3);
        assert!(matches!(encoded[0], Encoded::Inline(_)));
        assert!(matches!(encoded[1], Encoded::Stored(_)));
        assert!(matches!(encoded[2], Encoded::Inline(_)));
    }

    #[tokio::test]
    async fn local_bus_delivers_to_subscribers_of_the_topic_only() {
        let bus = ClusterBus::local("n");
        let mut a = bus.subscribe("a");
        let mut b = bus.subscribe("b");
        bus.publish("a", "hello".into());
        assert_eq!(a.recv().await.unwrap(), "hello");
        assert!(b.try_recv().is_err());
    }

    #[test]
    fn envelopes_from_this_node_are_ignored() {
        let bus = ClusterBus::local("self");
        let mut rx = bus.subscribe("t");
        bus.deliver_envelope(&json!({ "o": "self", "m": [["t", "x"]] }));
        assert!(rx.try_recv().is_err());
        bus.deliver_envelope(&json!({ "o": "other", "m": [["t", "y"]] }));
        assert_eq!(rx.try_recv().unwrap(), "y");
    }
}
