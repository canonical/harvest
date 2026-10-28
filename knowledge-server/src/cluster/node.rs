use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use harvest_db::Db;
use serde_json::Value;
use tokio::sync::watch;
use tokio::time::Instant;

use crate::cluster::membership::{
    Announcement, Member, Membership, MembershipError, MembershipHealth,
};
use crate::cluster::postgres::PostgresMembership;
use crate::config::ClusterConfig;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStanding {
    Confirmed,
    Fenced,
    Evicted,
}

pub struct NodeIdentity {
    pub node_id: String,
    pub internal_url: Option<String>,
}

impl NodeIdentity {
    pub fn from_config(config: &ClusterConfig) -> Self {
        let name = config
            .node_name
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| std::env::var("HOSTNAME").ok().filter(|h| !h.is_empty()))
            .unwrap_or_else(|| "harvest".to_string());
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        Self {
            node_id: format!("{name}-{}", &suffix[..12]),
            internal_url: config.internal_url.clone().filter(|u| !u.is_empty()),
        }
    }
}

pub struct ClusterNode {
    node_id: String,
    internal_url: Option<String>,
    heartbeat_interval: Duration,
    node_timeout: Duration,
    draining: AtomicBool,
    membership: Arc<dyn Membership>,
    db: Arc<Db>,
    confirmed_at: Mutex<Option<Instant>>,
    standing: watch::Sender<NodeStanding>,
}

impl ClusterNode {
    pub fn new(db: Arc<Db>, config: &ClusterConfig) -> Arc<Self> {
        let membership = PostgresMembership::new(Arc::clone(&db));
        Self::assemble(NodeIdentity::from_config(config), config, membership, db)
    }

    pub fn assemble(
        identity: NodeIdentity,
        config: &ClusterConfig,
        membership: Arc<dyn Membership>,
        db: Arc<Db>,
    ) -> Arc<Self> {
        Arc::new(Self {
            node_id: identity.node_id,
            internal_url: identity.internal_url,
            heartbeat_interval: Duration::from_millis(config.heartbeat_interval_ms.max(10)),
            node_timeout: Duration::from_millis(config.node_timeout_ms.max(20)),
            draining: AtomicBool::new(false),
            membership,
            db,
            confirmed_at: Mutex::new(None),
            standing: watch::channel(NodeStanding::Confirmed).0,
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

    pub fn membership_health(&self) -> MembershipHealth {
        self.membership.health()
    }

    pub fn standing(&self) -> NodeStanding {
        *self.standing.borrow()
    }

    pub fn is_fenced(&self) -> bool {
        self.standing() != NodeStanding::Confirmed
    }

    pub fn watch_standing(&self) -> watch::Receiver<NodeStanding> {
        self.standing.subscribe()
    }

    fn fence_after(&self) -> Duration {
        self.node_timeout.saturating_sub(self.heartbeat_interval)
    }

    fn announcement(&self) -> Announcement {
        Announcement {
            node_id: self.node_id.clone(),
            internal_url: self.internal_url.clone().unwrap_or_default(),
            version: VERSION.to_string(),
            draining: self.is_draining(),
        }
    }

    pub async fn heartbeat(&self) -> Result<()> {
        let sent_at = Instant::now();
        let outcome = tokio::time::timeout(
            self.heartbeat_interval.max(Duration::from_millis(500)),
            self.membership.announce(&self.announcement()),
        )
        .await
        .unwrap_or_else(|_| {
            Err(MembershipError::Unavailable(anyhow::anyhow!(
                "heartbeat timed out"
            )))
        });
        match outcome {
            Ok(()) => {
                *self.confirmed_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(sent_at);
                self.standing.send_if_modified(|s| {
                    let changed = *s == NodeStanding::Fenced;
                    if changed {
                        *s = NodeStanding::Confirmed;
                    }
                    changed
                });
                Ok(())
            }
            Err(MembershipError::Evicted) => {
                self.standing.send_replace(NodeStanding::Evicted);
                Err(MembershipError::Evicted.into())
            }
            Err(MembershipError::Unavailable(e)) => Err(e),
        }
    }

    fn check_fence(&self) {
        let confirmed_at = *self.confirmed_at.lock().unwrap_or_else(|e| e.into_inner());
        let Some(confirmed_at) = confirmed_at else {
            return;
        };
        if confirmed_at.elapsed() < self.fence_after() {
            return;
        }
        self.standing.send_if_modified(|s| {
            let changed = *s == NodeStanding::Confirmed;
            if changed {
                *s = NodeStanding::Fenced;
            }
            changed
        });
    }

    pub fn spawn_heartbeat(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let node = Arc::clone(self);
        tokio::spawn(async move {
            let mut beat = tokio::time::interval(node.heartbeat_interval);
            beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut watchdog =
                tokio::time::interval((node.heartbeat_interval / 4).max(Duration::from_millis(5)));
            watchdog.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = beat.tick() => {
                        if let Err(e) = node.heartbeat().await {
                            tracing::warn!(error = %e, node_id = %node.node_id, "cluster heartbeat failed");
                        }
                        if node.standing() == NodeStanding::Evicted {
                            return;
                        }
                    }
                    _ = watchdog.tick() => node.check_fence(),
                }
            }
        })
    }

    pub async fn set_draining(&self) -> Result<()> {
        self.draining.store(true, Ordering::SeqCst);
        self.heartbeat().await
    }

    pub async fn deregister(&self) -> Result<()> {
        self.membership.withdraw(&self.node_id).await
    }

    pub async fn prune(&self) -> Result<()> {
        self.membership.prune().await
    }

    pub async fn members(&self) -> Result<Vec<Member>> {
        self.membership.alive(self.node_timeout).await
    }

    pub async fn alive_node_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .members()
            .await?
            .into_iter()
            .map(|m| m.node_id)
            .collect())
    }

    pub async fn live_node_ids(&self) -> Result<Vec<String>> {
        let mut ids = self.alive_node_ids().await?;
        if !ids.iter().any(|id| *id == self.node_id) {
            ids.push(self.node_id.clone());
        }
        Ok(ids)
    }

    pub async fn is_alive(&self, node_id: &str) -> Result<bool> {
        Ok(self.members().await?.iter().any(|m| m.node_id == node_id))
    }

    pub async fn internal_url_of(&self, node_id: &str) -> Result<Option<String>> {
        Ok(self
            .members()
            .await?
            .into_iter()
            .find(|m| m.node_id == node_id)
            .map(|m| m.internal_url)
            .filter(|u| !u.is_empty()))
    }

    pub async fn alive_nodes(&self) -> Result<Vec<Value>> {
        Ok(self
            .members()
            .await?
            .into_iter()
            .map(|m| serde_json::to_value(m).unwrap_or_default())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;

    struct Scripted {
        failing: AtomicBool,
        evict: AtomicBool,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Membership for Scripted {
        async fn announce(&self, _: &Announcement) -> Result<(), MembershipError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.evict.load(Ordering::SeqCst) {
                return Err(MembershipError::Evicted);
            }
            if self.failing.load(Ordering::SeqCst) {
                return Err(MembershipError::Unavailable(anyhow::anyhow!("down")));
            }
            Ok(())
        }
        async fn withdraw(&self, _: &str) -> Result<()> {
            Ok(())
        }
        async fn alive(&self, _: Duration) -> Result<Vec<Member>> {
            Ok(vec![])
        }
        async fn prune(&self) -> Result<()> {
            Ok(())
        }
        fn health(&self) -> MembershipHealth {
            MembershipHealth::default()
        }
    }

    fn node(membership: Arc<Scripted>) -> Arc<ClusterNode> {
        let config = ClusterConfig {
            heartbeat_interval_ms: 20,
            node_timeout_ms: 80,
            ..ClusterConfig::default()
        };
        let db = Arc::new(
            Db::connect_without_migrating("postgres://unused@127.0.0.1:1/unused").unwrap(),
        );
        ClusterNode::assemble(NodeIdentity::from_config(&config), &config, membership, db)
    }

    fn scripted() -> Arc<Scripted> {
        Arc::new(Scripted {
            failing: AtomicBool::new(false),
            evict: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        })
    }

    #[tokio::test]
    async fn a_node_fences_itself_before_its_peers_consider_it_dead_and_recovers() {
        let membership = scripted();
        let node = node(Arc::clone(&membership));
        node.spawn_heartbeat();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(node.standing(), NodeStanding::Confirmed);

        membership.failing.store(true, Ordering::SeqCst);
        let mut standing = node.watch_standing();
        let fenced_after = Instant::now();
        tokio::time::timeout(
            Duration::from_secs(1),
            standing.wait_for(|s| *s == NodeStanding::Fenced),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(fenced_after.elapsed() < node.node_timeout());

        membership.failing.store(false, Ordering::SeqCst);
        tokio::time::timeout(
            Duration::from_secs(1),
            standing.wait_for(|s| *s == NodeStanding::Confirmed),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn eviction_is_final_and_stops_heartbeats() {
        let membership = scripted();
        let node = node(Arc::clone(&membership));
        let heartbeat = node.spawn_heartbeat();
        membership.evict.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(1), heartbeat)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.standing(), NodeStanding::Evicted);
        membership.evict.store(false, Ordering::SeqCst);
        assert!(node.heartbeat().await.is_ok());
        assert_eq!(node.standing(), NodeStanding::Evicted);
    }
}
