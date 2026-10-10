use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::config::ClusterConfig;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct ClusterNode {
    node_id: String,
    internal_url: Option<String>,
    heartbeat_interval: Duration,
    node_timeout: Duration,
    draining: AtomicBool,
    db: Arc<Db>,
}

impl ClusterNode {
    pub fn new(db: Arc<Db>, config: &ClusterConfig) -> Arc<Self> {
        let name = config
            .node_name
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()))
            .unwrap_or_else(|| "harvest".to_string());
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        Arc::new(Self {
            node_id: format!("{name}-{}", &suffix[..12]),
            internal_url: config.internal_url.clone().filter(|u| !u.is_empty()),
            heartbeat_interval: Duration::from_millis(config.heartbeat_interval_ms.max(10)),
            node_timeout: Duration::from_millis(config.node_timeout_ms.max(20)),
            draining: AtomicBool::new(false),
            db,
        })
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn internal_url(&self) -> Option<&str> {
        self.internal_url.as_deref()
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub fn node_timeout(&self) -> Duration {
        self.node_timeout
    }

    pub fn heartbeat_interval(&self) -> Duration {
        self.heartbeat_interval
    }

    pub fn timeout_secs(&self) -> f64 {
        self.node_timeout.as_secs_f64()
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    pub async fn heartbeat(&self) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO cluster_nodes (node_id, internal_url, version, heartbeat_at, draining)
                 VALUES ($id, $url, $version, now(), $draining)
                 ON CONFLICT (node_id) DO UPDATE
                 SET heartbeat_at = now(), internal_url = EXCLUDED.internal_url, draining = EXCLUDED.draining",
                json!({
                    "id": self.node_id,
                    "url": self.internal_url.clone().unwrap_or_default(),
                    "version": VERSION,
                    "draining": self.is_draining(),
                }),
            )
            .await?;
        Ok(())
    }

    pub fn spawn_heartbeat(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let node = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(node.heartbeat_interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(e) = node.heartbeat().await {
                    tracing::warn!(error = %e, node_id = %node.node_id, "cluster heartbeat failed");
                }
            }
        })
    }

    pub async fn set_draining(&self) -> Result<()> {
        self.draining.store(true, Ordering::SeqCst);
        self.heartbeat().await
    }

    pub async fn deregister(&self) -> Result<()> {
        self.db
            .execute("DELETE FROM cluster_nodes WHERE node_id = $id", json!({ "id": self.node_id }))
            .await?;
        Ok(())
    }

    pub async fn is_alive(&self, node_id: &str) -> Result<bool> {
        let rows = self
            .db
            .query(
                "SELECT 1 AS alive FROM cluster_nodes
                 WHERE node_id = $id AND heartbeat_at > now() - make_interval(secs => $timeout::float8)",
                json!({ "id": node_id, "timeout": self.timeout_secs() }),
            )
            .await?;
        Ok(!rows.is_empty())
    }

    pub async fn internal_url_of(&self, node_id: &str) -> Result<Option<String>> {
        let rows = self
            .db
            .query(
                "SELECT internal_url FROM cluster_nodes
                 WHERE node_id = $id AND heartbeat_at > now() - make_interval(secs => $timeout::float8)",
                json!({ "id": node_id, "timeout": self.timeout_secs() }),
            )
            .await?;
        Ok(rows
            .into_iter()
            .next()
            .and_then(|r| r["internal_url"].as_str().map(str::to_string))
            .filter(|u| !u.is_empty()))
    }

    pub async fn alive_nodes(&self) -> Result<Vec<Value>> {
        self.db
            .query(
                "SELECT node_id, internal_url, version, draining, started_at, heartbeat_at FROM cluster_nodes
                 WHERE heartbeat_at > now() - make_interval(secs => $timeout::float8)
                 ORDER BY started_at",
                json!({ "timeout": self.timeout_secs() }),
            )
            .await
    }
}
