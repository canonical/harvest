use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use harvest_db::Db;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, RwLock};

use crate::agent::AgentEvent;
use crate::cluster::bus::ClusterBus;
use crate::cluster::node::ClusterNode;
use crate::cluster::peer::PeerClient;
use crate::cluster::Shutdown;
use crate::config::ClusterConfig;

const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InFlightEvent {
    Thinking   { text: String },
    TextDelta  { text: String },
    ToolCall   { name: String, input: Value, description: Option<String>, hostname: Option<String> },
    ToolResult { name: String, preview: String },
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct InFlightState {
    pub query:       String,
    pub username:    String,
    pub attachments: Vec<Value>,
    pub events:      Vec<InFlightEvent>,
}

pub fn record_in_flight(
    state:       &mut InFlightState,
    event:       &AgentEvent,
    description: Option<String>,
    hostname:    Option<String>,
) {
    match event {
        AgentEvent::ThinkingDelta { text } => {
            if let Some(InFlightEvent::Thinking { text: t }) = state.events.last_mut() {
                t.push_str(text);
            } else {
                state.events.push(InFlightEvent::Thinking { text: text.clone() });
            }
        }
        AgentEvent::Thinking { text } => {
            state.events.push(InFlightEvent::Thinking { text: text.clone() });
        }
        AgentEvent::TextDelta { text } => {
            if let Some(InFlightEvent::TextDelta { text: t }) = state.events.last_mut() {
                t.push_str(text);
            } else {
                state.events.push(InFlightEvent::TextDelta { text: text.clone() });
            }
        }
        AgentEvent::ToolCall { name, input } => {
            state.events.push(InFlightEvent::ToolCall {
                name: name.clone(), input: input.clone(), description, hostname,
            });
        }
        AgentEvent::ToolResult { name, preview } => {
            state.events.push(InFlightEvent::ToolResult {
                name: name.clone(), preview: preview.clone(),
            });
        }
        _ => {}
    }
}

pub fn build_catchup_events(cid: &str, state: &InFlightState) -> Vec<Value> {
    state.events.iter().map(|ev| match ev {
        InFlightEvent::Thinking { text } => json!({
            "type": "thinking", "conv_id": cid, "text": text,
        }),
        InFlightEvent::TextDelta { text } => json!({
            "type": "text_delta", "conv_id": cid, "text": text,
        }),
        InFlightEvent::ToolCall { name, input, description, hostname } => {
            let mut v = json!({
                "type": "tool_call", "conv_id": cid,
                "name": name, "input": input,
            });
            if let Some(d) = description { v["description"] = json!(d); }
            if let Some(h) = hostname    { v["hostname"]    = json!(h); }
            v
        }
        InFlightEvent::ToolResult { name, preview } => json!({
            "type": "tool_result", "conv_id": cid,
            "name": name, "preview": preview,
        }),
    }).collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub state: InFlightState,
    pub seq:   u64,
}

struct LocalTurn {
    project_id: String,
    turn_id:    String,
    state:      InFlightState,
    seq:        u64,
    abort:      Option<tokio::task::AbortHandle>,
}

pub struct ProjectLive {
    db:    Arc<Db>,
    node:  Arc<ClusterNode>,
    bus:   Arc<ClusterBus>,
    peers: Option<Arc<PeerClient>>,
    turns: RwLock<HashMap<String, LocalTurn>>,
    shutdown: Shutdown,
}

pub fn topic(project_id: &str) -> String {
    format!("project:{project_id}")
}

impl ProjectLive {
    pub fn new(
        db: Arc<Db>,
        node: Arc<ClusterNode>,
        bus: Arc<ClusterBus>,
        peers: Option<Arc<PeerClient>>,
        shutdown: Shutdown,
    ) -> Arc<Self> {
        Arc::new(Self { db, node, bus, peers, turns: RwLock::new(HashMap::new()), shutdown })
    }

    pub fn shutdown_signal(&self) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.shutdown.wait()
    }

    pub fn standalone(db: Arc<Db>) -> Arc<Self> {
        let node = ClusterNode::new(Arc::clone(&db), &ClusterConfig::default());
        let bus = ClusterBus::local(node.node_id().to_string());
        if tokio::runtime::Handle::try_current().is_ok() {
            let heartbeat = Arc::clone(&node);
            tokio::spawn(async move {
                let _ = heartbeat.heartbeat().await;
            });
            node.spawn_heartbeat();
        }
        Self::new(db, node, bus, None, Shutdown::new())
    }

    pub fn node(&self) -> &Arc<ClusterNode> {
        &self.node
    }

    pub fn bus(&self) -> &Arc<ClusterBus> {
        &self.bus
    }

    pub fn subscribe(&self, project_id: &str) -> broadcast::Receiver<String> {
        self.bus.subscribe(&topic(project_id))
    }

    pub fn broadcast(&self, project_id: &str, message: String) {
        self.bus.publish(&topic(project_id), message);
    }

    pub async fn try_lock(
        &self,
        project_id:  &str,
        conv_id:     &str,
        turn_id:     &str,
        by:          &str,
        username:    &str,
        query:       &str,
        attachments: &[Value],
    ) -> Result<bool> {
        let rows = self.db.query(
            "INSERT INTO active_turns (conv_id, project_id, turn_id, node_id, locked_by, query, attachments)
             VALUES ($cid, $pid, $tid, $node, $by, $query, $attachments)
             ON CONFLICT (conv_id) DO UPDATE SET
                 project_id = EXCLUDED.project_id, turn_id = EXCLUDED.turn_id, node_id = EXCLUDED.node_id,
                 locked_by = EXCLUDED.locked_by, query = EXCLUDED.query, attachments = EXCLUDED.attachments,
                 started_at = now()
             WHERE active_turns.node_id <> $node
               AND NOT EXISTS (
                   SELECT 1 FROM cluster_nodes n
                   WHERE n.node_id = active_turns.node_id
                     AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8))
             RETURNING turn_id",
            json!({
                "cid": conv_id, "pid": project_id, "tid": turn_id, "node": self.node.node_id(),
                "by": by, "query": query, "attachments": attachments,
                "timeout": self.node.timeout_secs(),
            }),
        ).await?;
        if rows.is_empty() {
            return Ok(false);
        }
        self.turns.write().await.insert(conv_id.to_string(), LocalTurn {
            project_id: project_id.to_string(),
            turn_id:    turn_id.to_string(),
            state: InFlightState {
                query:       query.to_string(),
                username:    username.to_string(),
                attachments: attachments.to_vec(),
                events:      vec![],
            },
            seq:   0,
            abort: None,
        });
        Ok(true)
    }

    pub async fn set_abort_handle(&self, conv_id: &str, handle: tokio::task::AbortHandle) {
        if let Some(turn) = self.turns.write().await.get_mut(conv_id) {
            turn.abort = Some(handle);
        }
    }

    pub async fn record(&self, conv_id: &str, event: &AgentEvent, description: Option<String>, hostname: Option<String>) -> u64 {
        let mut turns = self.turns.write().await;
        let Some(turn) = turns.get_mut(conv_id) else { return 0 };
        record_in_flight(&mut turn.state, event, description, hostname);
        turn.seq += 1;
        turn.seq
    }

    pub async fn finish(&self, conv_id: &str, turn_id: &str) -> Result<()> {
        {
            let mut turns = self.turns.write().await;
            if turns.get(conv_id).is_some_and(|t| t.turn_id == turn_id) {
                turns.remove(conv_id);
            }
        }
        self.db.execute(
            "DELETE FROM active_turns WHERE conv_id = $cid AND turn_id = $tid",
            json!({ "cid": conv_id, "tid": turn_id }),
        ).await?;
        Ok(())
    }

    pub async fn local_turn_count(&self) -> usize {
        self.turns.read().await.len()
    }

    pub async fn locks(&self, project_id: &str) -> Result<Vec<(String, String)>> {
        let rows = self.db.query(
            "SELECT conv_id, locked_by FROM active_turns t
             WHERE t.project_id = $pid
               AND (t.node_id = $node OR EXISTS (
                   SELECT 1 FROM cluster_nodes n
                   WHERE n.node_id = t.node_id
                     AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8)))
             ORDER BY t.started_at",
            json!({ "pid": project_id, "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await?;
        Ok(rows.into_iter().map(|r| (
            r["conv_id"].as_str().unwrap_or_default().to_string(),
            r["locked_by"].as_str().unwrap_or_default().to_string(),
        )).collect())
    }

    pub async fn is_locked(&self, project_id: &str, conv_id: &str) -> Result<bool> {
        Ok(self.locks(project_id).await?.iter().any(|(cid, _)| cid == conv_id))
    }

    pub async fn local_snapshot(&self, conv_id: &str) -> Option<TurnSnapshot> {
        self.turns.read().await.get(conv_id).map(|t| TurnSnapshot { state: t.state.clone(), seq: t.seq })
    }

    pub async fn snapshot(&self, conv_id: &str) -> Option<TurnSnapshot> {
        if let Some(local) = self.local_snapshot(conv_id).await {
            return Some(local);
        }
        let rows = self.db.query(
            "SELECT node_id FROM active_turns WHERE conv_id = $cid",
            json!({ "cid": conv_id }),
        ).await.ok()?;
        let owner = rows.into_iter().next()?["node_id"].as_str()?.to_string();
        if owner == self.node.node_id() {
            return None;
        }
        let peers = self.peers.as_ref()?;
        let url = self.node.internal_url_of(&owner).await.ok()??;
        let value = peers
            .get_json(&url, &format!("/internal/turns/{conv_id}/snapshot"), SNAPSHOT_TIMEOUT)
            .await
            .map_err(|e| tracing::warn!(error = %e, conv_id, owner, "fetching turn snapshot from peer failed"))
            .ok()??;
        serde_json::from_value(value).ok()
    }

    pub async fn join(&self, project_id: &str, connection_id: &str, user_id: &str, name: &str, conv_id: Option<&str>) -> Result<()> {
        self.db.execute(
            "INSERT INTO project_presence (connection_id, project_id, user_id, name, conv_id, node_id)
             VALUES ($conn, $pid, $uid, $name, $cid, $node)
             ON CONFLICT (connection_id) DO UPDATE SET conv_id = EXCLUDED.conv_id",
            json!({
                "conn": connection_id, "pid": project_id, "uid": user_id, "name": name,
                "cid": conv_id, "node": self.node.node_id(),
            }),
        ).await?;
        Ok(())
    }

    pub async fn leave(&self, connection_id: &str) -> Result<()> {
        let rows = self.db.query(
            "DELETE FROM project_presence WHERE connection_id = $conn RETURNING project_id, user_id, name",
            json!({ "conn": connection_id }),
        ).await?;
        for row in rows {
            self.announce_leave_if_gone(&row).await;
        }
        Ok(())
    }

    async fn announce_leave_if_gone(&self, row: &Value) {
        let project_id = row["project_id"].as_str().unwrap_or_default();
        let user_id = row["user_id"].as_str().unwrap_or_default();
        let remaining = self.db.query(
            "SELECT 1 AS present FROM project_presence WHERE project_id = $pid AND user_id = $uid LIMIT 1",
            json!({ "pid": project_id, "uid": user_id }),
        ).await.map(|r| !r.is_empty()).unwrap_or(false);
        if !remaining {
            self.broadcast(project_id, json!({
                "type": "user_leave", "user_id": user_id, "name": row["name"],
            }).to_string());
        }
    }

    pub async fn presence(&self, project_id: &str) -> Result<Vec<Value>> {
        self.db.query(
            "SELECT DISTINCT ON (p.user_id) p.user_id, p.name, p.conv_id
             FROM project_presence p
             WHERE p.project_id = $pid
               AND (p.node_id = $node OR EXISTS (
                   SELECT 1 FROM cluster_nodes n
                   WHERE n.node_id = p.node_id
                     AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8)))
             ORDER BY p.user_id, p.connected_at DESC",
            json!({ "pid": project_id, "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await
    }

    pub async fn save_paused(&self, project_id: &str, conv_id: &str, payload: &Value) -> Result<()> {
        self.db.execute(
            "INSERT INTO paused_turns (conv_id, project_id, payload) VALUES ($cid, $pid, $payload)
             ON CONFLICT (conv_id) DO UPDATE SET project_id = EXCLUDED.project_id, payload = EXCLUDED.payload, created_at = now()",
            json!({ "cid": conv_id, "pid": project_id, "payload": payload }),
        ).await?;
        Ok(())
    }

    pub async fn has_paused(&self, project_id: &str, conv_id: &str) -> bool {
        self.db.query(
            "SELECT 1 AS present FROM paused_turns WHERE conv_id = $cid AND project_id = $pid",
            json!({ "cid": conv_id, "pid": project_id }),
        ).await.map(|rows| !rows.is_empty()).unwrap_or(false)
    }

    pub async fn update_paused<F>(&self, project_id: &str, conv_id: &str, update: F) -> Result<Option<(Value, bool)>>
    where
        F: FnOnce(&mut Value) -> bool,
    {
        let tx = self.db.begin().await?;
        let rows = tx.query(
            "SELECT payload FROM paused_turns WHERE conv_id = $cid AND project_id = $pid FOR UPDATE",
            json!({ "cid": conv_id, "pid": project_id }),
        ).await?;
        let Some(row) = rows.into_iter().next() else {
            tx.rollback().await?;
            return Ok(None);
        };
        let mut payload = row["payload"].clone();
        let ready = update(&mut payload);
        if ready {
            tx.execute("DELETE FROM paused_turns WHERE conv_id = $cid", json!({ "cid": conv_id })).await?;
        } else {
            tx.execute(
                "UPDATE paused_turns SET payload = $payload WHERE conv_id = $cid",
                json!({ "cid": conv_id, "payload": payload }),
            ).await?;
        }
        tx.commit().await?;
        Ok(Some((payload, ready)))
    }

    fn announce_abort(&self, project_id: &str, conv_id: &str) {
        self.broadcast(project_id, json!({ "type": "turn_aborted", "conv_id": conv_id }).to_string());
        self.broadcast(project_id, json!({ "type": "unlock", "conv_id": conv_id }).to_string());
    }

    pub async fn reap_dead_nodes(&self) -> Result<usize> {
        let turns = self.db.query(
            "DELETE FROM active_turns t
             WHERE t.node_id <> $node AND NOT EXISTS (
                 SELECT 1 FROM cluster_nodes n
                 WHERE n.node_id = t.node_id
                   AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8))
             RETURNING project_id, conv_id",
            json!({ "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await?;
        for row in &turns {
            let project_id = row["project_id"].as_str().unwrap_or_default();
            let conv_id = row["conv_id"].as_str().unwrap_or_default();
            tracing::warn!(project_id, conv_id, "turn owned by a lost node was aborted");
            self.announce_abort(project_id, conv_id);
        }
        let presence = self.db.query(
            "DELETE FROM project_presence p
             WHERE p.node_id <> $node AND NOT EXISTS (
                 SELECT 1 FROM cluster_nodes n
                 WHERE n.node_id = p.node_id
                   AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8))
             RETURNING project_id, user_id, name",
            json!({ "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await?;
        for row in &presence {
            self.announce_leave_if_gone(row).await;
        }
        Ok(turns.len() + presence.len())
    }

    pub async fn abort_local_turns(&self) -> usize {
        let drained: Vec<(String, LocalTurn)> = self.turns.write().await.drain().collect();
        for (conv_id, turn) in &drained {
            if let Some(abort) = &turn.abort {
                abort.abort();
            }
            let _ = self.db.execute(
                "DELETE FROM active_turns WHERE conv_id = $cid AND turn_id = $tid",
                json!({ "cid": conv_id, "tid": turn.turn_id }),
            ).await;
            self.announce_abort(&turn.project_id, conv_id);
        }
        drained.len()
    }

    pub async fn release_presence(&self) -> Result<()> {
        let rows = self.db.query(
            "DELETE FROM project_presence WHERE node_id = $node RETURNING project_id, user_id, name",
            json!({ "node": self.node.node_id() }),
        ).await?;
        for row in &rows {
            self.announce_leave_if_gone(row).await;
        }
        Ok(())
    }
}

pub fn seq_filter(conv_id: String, cutoff: u64) -> impl Fn(&str) -> bool {
    move |raw: &str| {
        if cutoff == 0 || !raw.contains("\"seq\"") {
            return true;
        }
        let Ok(value) = serde_json::from_str::<Value>(raw) else { return true };
        if value["conv_id"].as_str() != Some(conv_id.as_str()) {
            return true;
        }
        value["seq"].as_u64().map_or(true, |seq| seq > cutoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_filter_drops_events_already_covered_by_the_snapshot() {
        let keep = seq_filter("c1".into(), 5);
        assert!(!keep(&json!({ "type": "text_delta", "conv_id": "c1", "seq": 5 }).to_string()));
        assert!(!keep(&json!({ "type": "text_delta", "conv_id": "c1", "seq": 2 }).to_string()));
        assert!(keep(&json!({ "type": "text_delta", "conv_id": "c1", "seq": 6 }).to_string()));
        assert!(keep(&json!({ "type": "text_delta", "conv_id": "c2", "seq": 1 }).to_string()));
        assert!(keep(&json!({ "type": "lock", "conv_id": "c1" }).to_string()));
    }

    #[test]
    fn seq_filter_without_snapshot_keeps_everything() {
        let keep = seq_filter("c1".into(), 0);
        assert!(keep(&json!({ "type": "text_delta", "conv_id": "c1", "seq": 1 }).to_string()));
    }

    #[test]
    fn snapshots_round_trip_through_json() {
        let snapshot = TurnSnapshot {
            state: InFlightState {
                query: "q".into(),
                username: "u".into(),
                attachments: vec![json!({ "name": "a" })],
                events: vec![
                    InFlightEvent::Thinking { text: "t".into() },
                    InFlightEvent::ToolCall { name: "n".into(), input: json!({}), description: None, hostname: Some("h".into()) },
                ],
            },
            seq: 7,
        };
        let back: TurnSnapshot = serde_json::from_value(serde_json::to_value(&snapshot).unwrap()).unwrap();
        assert_eq!(back.seq, 7);
        assert_eq!(build_catchup_events("c", &back.state), build_catchup_events("c", &snapshot.state));
    }
}
