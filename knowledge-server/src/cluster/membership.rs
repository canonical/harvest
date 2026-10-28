use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Announcement {
    pub node_id: String,
    pub internal_url: String,
    pub version: String,
    pub draining: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub node_id: String,
    pub internal_url: String,
    pub version: String,
    pub draining: bool,
    pub started_at: String,
    pub heartbeat_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MembershipHealth {
    pub backend: &'static str,
    pub quorum: bool,
    pub leader: Option<String>,
    pub voters: usize,
    pub term: u64,
    pub apply_lag: u64,
    pub leader_changes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum MembershipError {
    #[error("this node was evicted from the cluster")]
    Evicted,
    #[error(transparent)]
    Unavailable(#[from] anyhow::Error),
}

#[async_trait]
pub trait Membership: Send + Sync {
    async fn announce(&self, announcement: &Announcement) -> Result<(), MembershipError>;
    async fn withdraw(&self, node_id: &str) -> Result<()>;
    async fn alive(&self, timeout: Duration) -> Result<Vec<Member>>;
    async fn prune(&self) -> Result<()>;
    fn health(&self) -> MembershipHealth;
}
