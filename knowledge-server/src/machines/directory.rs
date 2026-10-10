use std::sync::Arc;

use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::cluster::node::ClusterNode;
use crate::cluster::peer::PeerClient;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentLocation {
    pub agent_id:   String,
    pub node_id:    String,
    pub project_id: String,
    pub hostname:   String,
    pub connected_at: String,
}

pub struct AgentDirectory {
    db:    Arc<Db>,
    node:  Arc<ClusterNode>,
    peers: Option<Arc<PeerClient>>,
}

fn location_from_row(row: &Value) -> AgentLocation {
    AgentLocation {
        agent_id:     row["agent_id"].as_str().unwrap_or_default().to_string(),
        node_id:      row["node_id"].as_str().unwrap_or_default().to_string(),
        project_id:   row["project_id"].as_str().unwrap_or_default().to_string(),
        hostname:     row["hostname"].as_str().unwrap_or_default().to_string(),
        connected_at: row["connected_at"].as_str().unwrap_or_default().to_string(),
    }
}

const ALIVE: &str = "EXISTS (SELECT 1 FROM cluster_nodes n
                      WHERE n.node_id = c.node_id
                        AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8))";

impl AgentDirectory {
    pub fn new(db: Arc<Db>, node: Arc<ClusterNode>, peers: Option<Arc<PeerClient>>) -> Arc<Self> {
        Arc::new(Self { db, node, peers })
    }

    pub fn node_id(&self) -> &str {
        self.node.node_id()
    }

    pub fn node(&self) -> &Arc<ClusterNode> {
        &self.node
    }

    pub fn peers(&self) -> Option<&Arc<PeerClient>> {
        self.peers.as_ref()
    }

    pub async fn register(&self, agent_id: &str, connection_id: &str, project_id: &str, hostname: &str) -> Result<()> {
        self.db.execute(
            "INSERT INTO agent_connections (agent_id, node_id, connection_id, project_id, hostname, connected_at)
             VALUES ($aid, $node, $conn, $pid, $host, now())
             ON CONFLICT (agent_id) DO UPDATE SET
                 node_id = EXCLUDED.node_id, connection_id = EXCLUDED.connection_id,
                 project_id = EXCLUDED.project_id, hostname = EXCLUDED.hostname, connected_at = now()",
            json!({ "aid": agent_id, "node": self.node.node_id(), "conn": connection_id, "pid": project_id, "host": hostname }),
        ).await?;
        Ok(())
    }

    pub async fn unregister(&self, agent_id: &str, connection_id: &str) -> Result<()> {
        self.db.execute(
            "DELETE FROM agent_connections WHERE agent_id = $aid AND connection_id = $conn",
            json!({ "aid": agent_id, "conn": connection_id }),
        ).await?;
        Ok(())
    }

    pub async fn locate(&self, agent_id: &str) -> Option<AgentLocation> {
        let rows = self.db.query(
            &format!(
                "SELECT c.agent_id, c.node_id, c.project_id, c.hostname, c.connected_at FROM agent_connections c
                 WHERE c.agent_id = $aid AND (c.node_id = $node OR {ALIVE})"
            ),
            json!({ "aid": agent_id, "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await.ok()?;
        rows.first().map(location_from_row)
    }

    pub async fn online_in_project(&self, project_id: &str) -> Vec<AgentLocation> {
        self.db.query(
            &format!(
                "SELECT c.agent_id, c.node_id, c.project_id, c.hostname, c.connected_at FROM agent_connections c
                 WHERE c.project_id = $pid AND (c.node_id = $node OR {ALIVE})
                 ORDER BY c.connected_at"
            ),
            json!({ "pid": project_id, "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await.map(|rows| rows.iter().map(location_from_row).collect()).unwrap_or_default()
    }

    pub async fn owner_url(&self, agent_id: &str) -> Option<String> {
        let location = self.locate(agent_id).await?;
        if location.node_id == self.node.node_id() {
            return None;
        }
        self.node.internal_url_of(&location.node_id).await.ok().flatten()
    }

    pub async fn node_url(&self, node_id: &str) -> Option<String> {
        if node_id == self.node.node_id() {
            return None;
        }
        self.node.internal_url_of(node_id).await.ok().flatten()
    }

    pub async fn agent_for_token_hash(&self, token_hash: &str) -> Option<String> {
        let rows = self.db.query(
            "SELECT id FROM machines WHERE agent_token_hash = $h",
            json!({ "h": token_hash }),
        ).await.ok()?;
        rows.first()?["id"].as_str().map(str::to_string)
    }

    pub async fn hostname_from_inventory(&self, agent_id: &str) -> Option<String> {
        let rows = self.db.query(
            "SELECT hostname FROM machines WHERE id = $aid",
            json!({ "aid": agent_id }),
        ).await.ok()?;
        rows.first()?["hostname"].as_str().map(str::to_string)
    }

    pub async fn release_node(&self) -> Result<()> {
        self.db.execute(
            "DELETE FROM agent_connections WHERE node_id = $node",
            json!({ "node": self.node.node_id() }),
        ).await?;
        Ok(())
    }

    pub async fn reap_dead_nodes(&self) -> Result<u64> {
        self.db.execute(
            &format!("DELETE FROM agent_connections c WHERE c.node_id <> $node AND NOT {ALIVE}"),
            json!({ "node": self.node.node_id(), "timeout": self.node.timeout_secs() }),
        ).await
    }
}
