use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::cluster::membership::{
    Announcement, Member, Membership, MembershipError, MembershipHealth,
};

pub struct PostgresMembership {
    db: Arc<Db>,
}

impl PostgresMembership {
    pub fn new(db: Arc<Db>) -> Arc<Self> {
        Arc::new(Self { db })
    }
}

fn member_from_row(row: &Value) -> Member {
    let text = |key: &str| row[key].as_str().unwrap_or_default().to_string();
    Member {
        node_id: text("node_id"),
        internal_url: text("internal_url"),
        version: text("version"),
        draining: row["draining"].as_bool().unwrap_or(false),
        started_at: text("started_at"),
        heartbeat_at: text("heartbeat_at"),
    }
}

#[async_trait]
impl Membership for PostgresMembership {
    async fn announce(&self, announcement: &Announcement) -> Result<(), MembershipError> {
        self.db
            .execute(
                "INSERT INTO cluster_nodes (node_id, internal_url, version, heartbeat_at, draining)
                 VALUES ($id, $url, $version, now(), $draining)
                 ON CONFLICT (node_id) DO UPDATE
                 SET heartbeat_at = now(), internal_url = EXCLUDED.internal_url, draining = EXCLUDED.draining",
                json!({
                    "id": announcement.node_id,
                    "url": announcement.internal_url,
                    "version": announcement.version,
                    "draining": announcement.draining,
                }),
            )
            .await?;
        Ok(())
    }

    async fn withdraw(&self, node_id: &str) -> Result<()> {
        self.db
            .execute(
                "DELETE FROM cluster_nodes WHERE node_id = $id",
                json!({ "id": node_id }),
            )
            .await?;
        Ok(())
    }

    async fn alive(&self, timeout: Duration) -> Result<Vec<Member>> {
        let rows = self
            .db
            .query(
                "SELECT node_id, internal_url, version, draining, started_at, heartbeat_at FROM cluster_nodes
                 WHERE heartbeat_at > now() - make_interval(secs => $timeout::float8)
                 ORDER BY started_at",
                json!({ "timeout": timeout.as_secs_f64() }),
            )
            .await?;
        Ok(rows.iter().map(member_from_row).collect())
    }

    async fn prune(&self) -> Result<()> {
        self.db
            .execute(
                "DELETE FROM cluster_nodes WHERE heartbeat_at < now() - interval '1 hour'",
                json!({}),
            )
            .await?;
        Ok(())
    }

    fn health(&self) -> MembershipHealth {
        MembershipHealth {
            backend: "postgres",
            quorum: true,
            leader: None,
            voters: 0,
        }
    }
}
