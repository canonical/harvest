use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context, Result};
use harvest_db::Db;
use serde_json::{json, Value};

use crate::cluster::bus::{next_batch, resync_event, ClusterBus, Outbox};

pub const CHANNEL: &str = "harvest_bus";
const MAX_NOTIFY_BYTES: usize = 7_000;

pub fn start(db: Arc<Db>, node_id: String, batch_window: Duration) -> Arc<ClusterBus> {
    let (bus, outbox) = ClusterBus::with_outbox(node_id.clone());
    tokio::spawn(run_publisher(
        Arc::clone(&db),
        node_id,
        batch_window,
        outbox,
    ));
    tokio::spawn(run_listener(Arc::downgrade(&bus), db));
    bus
}

enum Encoded {
    Inline(String),
    Stored(String),
}

fn envelope(node_id: &str, messages: &[(String, String)]) -> String {
    json!({ "o": node_id, "m": messages.iter().map(|(t, d)| json!([t, d])).collect::<Vec<_>>() })
        .to_string()
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

async fn send_one(
    client: &harvest_db::tokio_postgres::Client,
    node_id: &str,
    item: &Encoded,
) -> Result<()> {
    let payload = match item {
        Encoded::Inline(text) => text.clone(),
        Encoded::Stored(body) => {
            let id: i64 = client
                .query_one(
                    "INSERT INTO bus_payloads (body) VALUES ($1) RETURNING id",
                    &[body],
                )
                .await
                .context("storing large bus payload")?
                .get(0);
            json!({ "o": node_id, "r": id }).to_string()
        }
    };
    let channel = CHANNEL.to_string();
    client
        .execute("SELECT pg_notify($1, $2)", &[&channel, &payload])
        .await?;
    Ok(())
}

async fn run_publisher(db: Arc<Db>, node_id: String, batch_window: Duration, mut outbox: Outbox) {
    let mut client: Option<harvest_db::tokio_postgres::Client> = None;
    while let Some(batch) = next_batch(&mut outbox, batch_window).await {
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
    let Some(id) = envelope["r"].as_i64() else {
        return Some(envelope);
    };
    let rows = db
        .query(
            "SELECT body FROM bus_payloads WHERE id = $id",
            json!({ "id": id }),
        )
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
                    bus.set_listener_pid(Some(listener.backend_pid()));
                    if connected_before {
                        bus.deliver_to_all(&resync_event());
                    }
                    bus.set_listening(true);
                }
                connected_before = true;
                while let Some(notification) = listener.recv().await {
                    let Some(envelope) = resolve_envelope(&db, &notification.payload).await else {
                        continue;
                    };
                    let Some(bus) = bus.upgrade() else { return };
                    bus.deliver_envelope(&envelope);
                }
                let Some(bus) = bus.upgrade() else { return };
                bus.set_listener_pid(None);
                bus.set_listening(false);
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
        let batch: Vec<(String, String)> = (0..10)
            .map(|i| ("t".to_string(), format!("m{i}")))
            .collect();
        let encoded = encode("n", batch);
        assert_eq!(encoded.len(), 1);
        assert!(matches!(&encoded[0], Encoded::Inline(text) if text.len() <= MAX_NOTIFY_BYTES));
    }

    #[test]
    fn batches_split_before_reaching_the_notify_limit() {
        let batch: Vec<(String, String)> = (0..100)
            .map(|i| ("t".to_string(), format!("{i}:{}", "x".repeat(300))))
            .collect();
        let encoded = encode("n", batch);
        assert!(encoded.len() > 1);
        let mut seen = Vec::new();
        for item in &encoded {
            let Encoded::Inline(text) = item else {
                panic!("unexpected stored payload")
            };
            assert!(text.len() <= MAX_NOTIFY_BYTES);
            let value: Value = serde_json::from_str(text).unwrap();
            for m in value["m"].as_array().unwrap() {
                seen.push(
                    m[1].as_str()
                        .unwrap()
                        .split(':')
                        .next()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap(),
                );
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
}
