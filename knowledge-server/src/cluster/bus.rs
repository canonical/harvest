use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harvest_db::Db;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch};

pub use crate::cluster::notify::CHANNEL;

const MAX_BATCH_MESSAGES: usize = 500;
const TOPIC_CAPACITY: usize = 4_096;

pub type Outbox = mpsc::UnboundedReceiver<(String, String)>;

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

    pub(crate) fn with_outbox(node_id: impl Into<String>) -> (Arc<Self>, Outbox) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (listening, _) = watch::channel(false);
        let bus = Arc::new(Self {
            node_id: node_id.into(),
            topics: Mutex::new(HashMap::new()),
            outbox: Some(sender),
            listening,
            listener_pid: Mutex::new(None),
        });
        (bus, receiver)
    }

    pub fn start(db: Arc<Db>, node_id: String, batch_window: Duration) -> Arc<Self> {
        crate::cluster::notify::start(db, node_id, batch_window)
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

    pub(crate) fn set_listener_pid(&self, pid: Option<i32>) {
        *self.listener_pid.lock().unwrap_or_else(|e| e.into_inner()) = pid;
    }

    pub(crate) fn set_listening(&self, listening: bool) {
        self.listening.send_replace(listening);
    }

    pub(crate) fn deliver(&self, topic: &str, data: String) {
        let mut topics = self.topics.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(sender) = topics.get(topic) {
            if sender.send(data).is_err() && sender.receiver_count() == 0 {
                topics.remove(topic);
            }
        }
    }

    pub(crate) fn deliver_to_all(&self, data: &str) {
        let topics = self.topics.lock().unwrap_or_else(|e| e.into_inner());
        for sender in topics.values() {
            let _ = sender.send(data.to_string());
        }
    }

    pub(crate) fn deliver_envelope(&self, envelope: &Value) {
        if envelope["o"].as_str() == Some(self.node_id.as_str()) {
            return;
        }
        let Some(messages) = envelope["m"].as_array() else {
            return;
        };
        for message in messages {
            if let (Some(topic), Some(data)) = (message[0].as_str(), message[1].as_str()) {
                self.deliver(topic, data.to_string());
            }
        }
    }
}

pub(crate) async fn next_batch(
    outbox: &mut Outbox,
    window: Duration,
) -> Option<Vec<(String, String)>> {
    let first = outbox.recv().await?;
    let mut batch = vec![first];
    let deadline = tokio::time::Instant::now() + window;
    while batch.len() < MAX_BATCH_MESSAGES {
        match tokio::time::timeout_at(deadline, outbox.recv()).await {
            Ok(Some(message)) => batch.push(message),
            _ => break,
        }
    }
    Some(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

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
