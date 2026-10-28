use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use quorium::{Command, JoinToken, KeepAlive, MemberRecord, Quorium, QuoriumConfig, Response};
use reqwest::header::{HeaderMap, HeaderValue};

use crate::cluster::membership::{
    Announcement, Member, Membership, MembershipError, MembershipHealth,
};
use crate::cluster::peer::{FORWARDED_HEADER, SECRET_HEADER};
use crate::config::{ClusterBootstrap, ClusterConfig};

const RAFT_HEARTBEAT: Duration = Duration::from_millis(250);

pub struct QuoriumMembership {
    quorium: Arc<Quorium>,
}

impl QuoriumMembership {
    pub fn new(quorium: Arc<Quorium>) -> Arc<Self> {
        Arc::new(Self { quorium })
    }

    pub fn quorium(&self) -> &Arc<Quorium> {
        &self.quorium
    }

    async fn register(&self, announcement: &Announcement) -> Result<(), MembershipError> {
        let member = MemberRecord {
            node_id: announcement.node_id.clone(),
            internal_url: announcement.internal_url.clone(),
            version: announcement.version.clone(),
            draining: announcement.draining,
            joined_at_ms: quorium::node::now_ms(),
        };
        match self.quorium.write(Command::Register { member }).await? {
            Response::Evicted => Err(MembershipError::Evicted),
            _ => Ok(()),
        }
    }

    fn is_current(&self, announcement: &Announcement) -> bool {
        self.quorium.members().iter().any(|m| {
            m.node_id == announcement.node_id
                && m.internal_url == announcement.internal_url
                && m.version == announcement.version
                && m.draining == announcement.draining
        })
    }
}

fn member_from_record(record: MemberRecord) -> Member {
    let started_at = chrono::DateTime::from_timestamp_millis(record.joined_at_ms as i64)
        .map(|t| t.to_rfc3339())
        .unwrap_or_default();
    Member {
        node_id: record.node_id,
        internal_url: record.internal_url,
        version: record.version,
        draining: record.draining,
        started_at,
        heartbeat_at: harvest_db::now_rfc3339(),
    }
}

#[async_trait]
impl Membership for QuoriumMembership {
    async fn announce(&self, announcement: &Announcement) -> Result<(), MembershipError> {
        if !self.is_current(announcement) {
            self.register(announcement).await?;
        }
        match self.quorium.keepalive(&announcement.node_id).await? {
            KeepAlive::Alive => Ok(()),
            KeepAlive::Evicted => Err(MembershipError::Evicted),
            KeepAlive::Unknown => {
                self.register(announcement).await?;
                match self.quorium.keepalive(&announcement.node_id).await? {
                    KeepAlive::Alive => Ok(()),
                    KeepAlive::Evicted => Err(MembershipError::Evicted),
                    KeepAlive::Unknown => Err(MembershipError::Unavailable(anyhow::anyhow!(
                        "the registration of this node is not visible to the leader yet"
                    ))),
                }
            }
        }
    }

    async fn withdraw(&self, node_id: &str) -> Result<()> {
        self.quorium
            .write(Command::Withdraw {
                node_id: node_id.to_string(),
            })
            .await?;
        Ok(())
    }

    async fn alive(&self, _timeout: Duration) -> Result<Vec<Member>> {
        let mut members = self.quorium.members_linearizable().await?;
        members.sort_by_key(|m| m.joined_at_ms);
        Ok(members.into_iter().map(member_from_record).collect())
    }

    async fn prune(&self) -> Result<()> {
        Ok(())
    }

    fn health(&self) -> MembershipHealth {
        let health = self.quorium.health();
        MembershipHealth {
            backend: "quorium",
            quorum: health.quorum,
            leader: health.leader,
            voters: health.voters,
        }
    }
}

pub fn peer_http_client(node_id: &str, secret: &str) -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(SECRET_HEADER, HeaderValue::from_str(secret)?);
    headers.insert(FORWARDED_HEADER, HeaderValue::from_str(node_id)?);
    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(2))
        .pool_idle_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

pub async fn start(
    config: &ClusterConfig,
    node_id: &str,
    advertise_url: &str,
) -> Result<Arc<Quorium>> {
    let secret = config.shared_secret.clone().unwrap_or_default();
    let data_dir = config
        .data_dir
        .clone()
        .context("cluster.data_dir is required by the quorium backend")?;
    Quorium::start(QuoriumConfig {
        data_dir,
        advertise_url: advertise_url.to_string(),
        heartbeat_interval: RAFT_HEARTBEAT
            .min(Duration::from_millis(config.election_timeout_ms / 3)),
        election_timeout: Duration::from_millis(config.election_timeout_ms),
        session_ttl: Duration::from_millis(config.node_timeout_ms),
        http: peer_http_client(node_id, &secret)?,
        admit_without_token: true,
    })
    .await
}

fn join_token(config: &ClusterConfig) -> Result<JoinToken> {
    match config.join_token.as_deref().filter(|t| !t.is_empty()) {
        Some(token) => JoinToken::decode(token),
        None => Ok(JoinToken {
            cluster_id: String::new(),
            addresses: config.join_addresses.clone(),
            secret: String::new(),
            shared_secret: String::new(),
        }),
    }
}

pub async fn bootstrap(quorium: &Quorium, config: &ClusterConfig) -> Result<()> {
    if quorium.is_initialized().await? {
        return Ok(());
    }
    match config.bootstrap {
        Some(ClusterBootstrap::Init) => quorium.initialize().await,
        Some(ClusterBootstrap::Join) => {
            quorium
                .join(&join_token(config)?, Duration::from_secs(config.join_timeout_secs))
                .await
        }
        None => bail!(
            "this quorium server is not part of a cluster yet: set cluster.bootstrap to \"init\" or \"join\""
        ),
    }
}
