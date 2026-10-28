use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use openraft::error::{ClientWriteError, RaftError};
use openraft::{ChangeMembers, ServerState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

use crate::network::HttpNetwork;
use crate::state::{Command, MemberRecord, Response};
use crate::store::{StateMachineStore, Storage};
use crate::token::JoinToken;
use crate::types::{Metrics, Node, Raft, ServerId};

pub const MAX_VOTERS: usize = 5;
const CONTROL_TICK: Duration = Duration::from_millis(200);
const LEADER_RETRY: Duration = Duration::from_millis(100);
const LEADER_ATTEMPTS: usize = 30;
const CAUGHT_UP_LAG: u64 = 16;

#[derive(Clone, Debug)]
pub struct QuoriumConfig {
    pub data_dir: PathBuf,
    pub advertise_url: String,
    pub heartbeat_interval: Duration,
    pub election_timeout: Duration,
    pub session_ttl: Duration,
    pub http: reqwest::Client,
    pub admit_without_token: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepAlive {
    Alive,
    Evicted,
    Unknown,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub quorum: bool,
    pub leader: Option<String>,
    pub voters: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub id: ServerId,
    pub address: String,
    pub voter: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Status {
    pub server_id: ServerId,
    pub cluster_id: String,
    pub role: String,
    pub term: u64,
    pub leader: Option<ServerId>,
    pub last_applied: Option<u64>,
    pub servers: Vec<ServerInfo>,
    pub members: Vec<MemberRecord>,
    pub token_expiries_ms: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinRequest {
    pub secret: String,
    pub server_id: ServerId,
    pub address: String,
}

#[derive(Default)]
struct Sessions {
    term: Option<u64>,
    seen: HashMap<String, Instant>,
}

pub struct Quorium {
    id: ServerId,
    advertise_url: String,
    raft: Raft,
    state: StateMachineStore,
    http: reqwest::Client,
    session_ttl: Duration,
    admit_without_token: bool,
    sessions: Mutex<Sessions>,
    controller: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn digest(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Quorium {
    pub async fn start(config: QuoriumConfig) -> Result<Arc<Self>> {
        let storage = Storage::open(&config.data_dir)
            .await
            .with_context(|| format!("opening quorium data in {}", config.data_dir.display()))?;
        let election_ms = config.election_timeout.as_millis() as u64;
        let raft_config = openraft::Config {
            cluster_name: "harvest".into(),
            heartbeat_interval: config.heartbeat_interval.as_millis() as u64,
            election_timeout_min: election_ms,
            election_timeout_max: election_ms * 2,
            install_snapshot_timeout: election_ms * 4,
            snapshot_policy: openraft::SnapshotPolicy::LogsSinceLast(1_000),
            max_in_snapshot_log_to_keep: 200,
            ..Default::default()
        }
        .validate()?;
        let state = storage.state_machine.clone();
        let raft = Raft::new(
            storage.server_id,
            Arc::new(raft_config),
            HttpNetwork::new(config.http.clone()),
            storage.log,
            storage.state_machine,
        )
        .await?;
        let quorium = Arc::new(Self {
            id: storage.server_id,
            advertise_url: config.advertise_url.trim_end_matches('/').to_string(),
            raft,
            state,
            http: config.http,
            session_ttl: config.session_ttl,
            admit_without_token: config.admit_without_token,
            sessions: Mutex::new(Sessions::default()),
            controller: Mutex::new(None),
        });
        let controller = tokio::spawn(run_controller(Arc::downgrade(&quorium)));
        *quorium.controller.lock().unwrap_or_else(|e| e.into_inner()) = Some(controller);
        Ok(quorium)
    }

    pub fn id(&self) -> ServerId {
        self.id
    }

    pub fn raft(&self) -> &Raft {
        &self.raft
    }

    pub fn advertise_url(&self) -> &str {
        &self.advertise_url
    }

    fn metrics(&self) -> Metrics {
        self.raft.metrics().borrow().clone()
    }

    pub fn is_leader(&self) -> bool {
        self.metrics().state == ServerState::Leader
    }

    pub async fn is_initialized(&self) -> Result<bool> {
        Ok(self.raft.is_initialized().await?)
    }

    pub fn cluster_id(&self) -> String {
        self.state.read(|a| a.cluster.cluster_id.clone())
    }

    pub async fn initialize(&self) -> Result<()> {
        if !self.is_initialized().await? {
            let node = Node {
                addr: self.advertise_url.clone(),
            };
            self.raft
                .initialize(BTreeMap::from([(self.id, node)]))
                .await
                .context("initializing the quorium cluster")?;
        }
        if self.cluster_id().is_empty() {
            self.write(Command::Bootstrap {
                cluster_id: uuid::Uuid::new_v4().to_string(),
            })
            .await?;
        }
        Ok(())
    }

    pub async fn join(&self, token: &JoinToken, timeout: Duration) -> Result<()> {
        if self.is_initialized().await? {
            return Ok(());
        }
        let request = JoinRequest {
            secret: token.secret.clone(),
            server_id: self.id,
            address: self.advertise_url.clone(),
        };
        let mut last_error = anyhow!("the join token lists no addresses");
        let mut accepted = false;
        for address in &token.addresses {
            match self
                .post::<_, ()>(address, "/internal/quorium/servers", &request)
                .await
            {
                Ok(()) => {
                    accepted = true;
                    break;
                }
                Err(e) => last_error = e,
            }
        }
        if !accepted {
            return Err(last_error.context("no cluster member accepted the join request"));
        }
        let id = self.id;
        self.raft
            .wait(Some(timeout))
            .metrics(
                |m| m.membership_config.membership().get_node(&id).is_some(),
                "joined the cluster",
            )
            .await
            .map_err(|e| anyhow!("waiting to be replicated into the cluster: {e}"))?;
        Ok(())
    }

    async fn request_admission(&self, token: &JoinToken, request: &JoinRequest) -> Result<()> {
        let mut last_error = anyhow!("the join token lists no addresses");
        for address in token
            .addresses
            .iter()
            .filter(|a| a.trim_end_matches('/') != self.advertise_url)
        {
            match self
                .post::<_, ()>(address, "/internal/quorium/servers", request)
                .await
            {
                Ok(()) => return Ok(()),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    pub async fn admit(&self, request: JoinRequest) -> Result<bool> {
        if !self.accepts(&request).await? {
            return Ok(false);
        }
        self.raft
            .add_learner(
                request.server_id,
                Node {
                    addr: request.address,
                },
                false,
            )
            .await
            .map_err(|e| anyhow!("adding server {}: {e}", request.server_id))?;
        Ok(true)
    }

    async fn accepts(&self, request: &JoinRequest) -> Result<bool> {
        if request.secret.is_empty() {
            return Ok(self.admit_without_token);
        }
        let accepted = self
            .write(Command::ConsumeToken {
                digest: digest(&request.secret),
                at_ms: now_ms(),
            })
            .await?;
        Ok(accepted == Response::TokenAccepted(true))
    }

    pub async fn revoke_tokens(&self) -> Result<()> {
        self.write(Command::RevokeTokens).await?;
        Ok(())
    }

    pub async fn issue_token(&self, ttl: Duration) -> Result<JoinToken> {
        let secret = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.write(Command::IssueToken {
            digest: digest(&secret),
            expires_at_ms: now_ms() + ttl.as_millis() as u64,
        })
        .await?;
        let addresses = self.servers().into_iter().map(|s| s.address).collect();
        Ok(JoinToken {
            cluster_id: self.cluster_id(),
            addresses,
            secret,
            shared_secret: String::new(),
        })
    }

    pub async fn remove_server(&self, id: ServerId) -> Result<()> {
        if !self.is_leader() {
            return self
                .retrying(|| async {
                    let leader = self.known_leader_url()?;
                    self.delete(&leader, &format!("/internal/quorium/servers/{id}"))
                        .await
                })
                .await;
        }
        let voters: BTreeSet<ServerId> = self
            .metrics()
            .membership_config
            .membership()
            .voter_ids()
            .collect();
        if voters.contains(&id) {
            if voters.len() == 1 {
                bail!("cannot remove the last voter of the cluster");
            }
            self.raft
                .change_membership(ChangeMembers::RemoveVoters(BTreeSet::from([id])), true)
                .await
                .map_err(|e| anyhow!("demoting server {id}: {e}"))?;
        }
        self.raft
            .change_membership(ChangeMembers::RemoveNodes(BTreeSet::from([id])), false)
            .await
            .map_err(|e| anyhow!("removing server {id}: {e}"))?;
        Ok(())
    }

    pub fn servers(&self) -> Vec<ServerInfo> {
        let metrics = self.metrics();
        let membership = metrics.membership_config.membership();
        let voters: BTreeSet<ServerId> = membership.voter_ids().collect();
        membership
            .nodes()
            .map(|(id, node)| ServerInfo {
                id: *id,
                address: node.addr.clone(),
                voter: voters.contains(id),
            })
            .collect()
    }

    pub fn members(&self) -> Vec<MemberRecord> {
        self.state
            .read(|a| a.cluster.members.values().cloned().collect())
    }

    pub async fn members_linearizable(&self) -> Result<Vec<MemberRecord>> {
        self.retrying(|| async {
            if self.is_leader() {
                self.raft
                    .ensure_linearizable()
                    .await
                    .map_err(|e| anyhow!("confirming leadership: {e}"))?;
                return Ok(self.members());
            }
            let leader = self.known_leader_url()?;
            self.get(&leader, "/internal/quorium/members").await
        })
        .await
    }

    pub fn health(&self) -> Health {
        let metrics = self.metrics();
        let leader = metrics.current_leader.and_then(|id| {
            metrics
                .membership_config
                .membership()
                .get_node(&id)
                .map(|n| n.addr.clone())
        });
        let quorum = match metrics.state {
            ServerState::Leader => metrics
                .millis_since_quorum_ack
                .is_some_and(|ms| ms < self.session_ttl.as_millis() as u64),
            ServerState::Follower | ServerState::Learner => leader.is_some(),
            _ => false,
        };
        Health {
            quorum,
            leader,
            voters: metrics.membership_config.membership().voter_ids().count(),
        }
    }

    pub fn status(&self) -> Status {
        let metrics = self.metrics();
        Status {
            server_id: self.id,
            cluster_id: self.cluster_id(),
            role: format!("{:?}", metrics.state).to_lowercase(),
            term: metrics.current_term,
            leader: metrics.current_leader,
            last_applied: metrics.last_applied.map(|l| l.index),
            servers: self.servers(),
            members: self.members(),
            token_expiries_ms: self
                .state
                .read(|a| a.cluster.tokens.values().copied().collect()),
        }
    }

    pub async fn write(&self, command: Command) -> Result<Response> {
        self.retrying(|| async {
            match self.raft.client_write(command.clone()).await {
                Ok(response) => Ok(response.data),
                Err(RaftError::APIError(ClientWriteError::ForwardToLeader(forward))) => {
                    let node = forward
                        .leader_node
                        .ok_or_else(|| anyhow!("no quorium leader is available"))?;
                    self.post(&node.addr, "/internal/quorium/write", &command)
                        .await
                }
                Err(e) => Err(anyhow!("quorium write failed: {e}")),
            }
        })
        .await
    }

    pub async fn write_locally(&self, command: Command) -> Result<Response> {
        self.raft
            .client_write(command)
            .await
            .map(|r| r.data)
            .map_err(|e| anyhow!("quorium write failed: {e}"))
    }

    pub async fn keepalive(&self, node_id: &str) -> Result<KeepAlive> {
        self.retrying(|| async {
            if self.is_leader() {
                return Ok(self.touch(node_id));
            }
            let leader = self.known_leader_url()?;
            self.post(&leader, "/internal/quorium/keepalive", &node_id)
                .await
        })
        .await
    }

    pub fn touch(&self, node_id: &str) -> KeepAlive {
        let (known, evicted) = self.state.read(|a| {
            (
                a.cluster.members.contains_key(node_id),
                a.cluster.is_evicted(node_id),
            )
        });
        if evicted {
            return KeepAlive::Evicted;
        }
        if !known {
            return KeepAlive::Unknown;
        }
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .seen
            .insert(node_id.to_string(), Instant::now());
        KeepAlive::Alive
    }

    pub async fn shutdown(&self) {
        if let Some(controller) = self
            .controller
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            controller.abort();
        }
        let _ = self.raft.shutdown().await;
    }

    fn known_leader_url(&self) -> Result<String> {
        let metrics = self.metrics();
        metrics
            .current_leader
            .filter(|id| *id != self.id)
            .and_then(|id| {
                metrics
                    .membership_config
                    .membership()
                    .get_node(&id)
                    .cloned()
            })
            .map(|node| node.addr)
            .ok_or_else(|| anyhow!("no quorium leader is available"))
    }

    async fn retrying<T, F, Fut>(&self, mut attempt: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut tries = 0;
        loop {
            tries += 1;
            match attempt().await {
                Ok(value) => return Ok(value),
                Err(e) if tries >= LEADER_ATTEMPTS => return Err(e),
                Err(e) => tracing::debug!(error = %e, tries, "retrying a quorium leader call"),
            }
            tokio::time::sleep(LEADER_RETRY).await;
        }
    }

    pub(crate) async fn post<B: Serialize + ?Sized, T: for<'de> Deserialize<'de>>(
        &self,
        base: &str,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = self
            .http
            .post(format!("{}{path}", base.trim_end_matches('/')))
            .json(body)
            .timeout(self.session_ttl)
            .send()
            .await
            .with_context(|| format!("POST {path} on {base}"))?
            .error_for_status()
            .with_context(|| format!("POST {path} on {base}"))?;
        Ok(response.json().await?)
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, base: &str, path: &str) -> Result<T> {
        let response = self
            .http
            .get(format!("{}{path}", base.trim_end_matches('/')))
            .timeout(self.session_ttl)
            .send()
            .await
            .with_context(|| format!("GET {path} on {base}"))?
            .error_for_status()
            .with_context(|| format!("GET {path} on {base}"))?;
        Ok(response.json().await?)
    }

    async fn delete(&self, base: &str, path: &str) -> Result<()> {
        self.http
            .delete(format!("{}{path}", base.trim_end_matches('/')))
            .timeout(self.session_ttl)
            .send()
            .await
            .with_context(|| format!("DELETE {path} on {base}"))?
            .error_for_status()
            .with_context(|| format!("DELETE {path} on {base}"))?;
        Ok(())
    }

    fn lapsed_sessions(&self, term: u64) -> Vec<String> {
        let members: Vec<String> = self
            .state
            .read(|a| a.cluster.members.keys().cloned().collect());
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if sessions.term != Some(term) {
            sessions.term = Some(term);
            sessions.seen.clear();
        }
        let now = Instant::now();
        sessions.seen.retain(|id, _| members.contains(id));
        members
            .into_iter()
            .filter(|id| {
                let seen = *sessions.seen.entry(id.clone()).or_insert(now);
                now.duration_since(seen) > self.session_ttl
            })
            .collect()
    }

    async fn expire_lapsed_sessions(&self, term: u64) {
        for node_id in self.lapsed_sessions(term) {
            tracing::warn!(node_id, "quorium session lapsed: evicting the member");
            let expired = self
                .write_locally(Command::Expire {
                    node_id: node_id.clone(),
                    at_ms: now_ms(),
                })
                .await;
            if let Err(e) = expired {
                tracing::warn!(error = %e, node_id, "could not evict a lapsed member");
            }
        }
    }

    async fn promote_caught_up_learner(&self, metrics: &Metrics) {
        let membership = metrics.membership_config.membership();
        if membership.get_joint_config().len() > 1 {
            return;
        }
        let voters: BTreeSet<ServerId> = membership.voter_ids().collect();
        let learners: Vec<ServerId> = membership.learner_ids().collect();
        if voters.len() >= target_voters(voters.len() + learners.len()) {
            return;
        }
        let commit = metrics.last_log_index.unwrap_or(0);
        let replication = metrics.replication.clone().unwrap_or_default();
        let Some(candidate) = learners.into_iter().find(|id| {
            replication
                .get(id)
                .and_then(|matched| *matched)
                .is_some_and(|matched| matched.index + CAUGHT_UP_LAG >= commit)
        }) else {
            return;
        };
        tracing::info!(
            server_id = candidate,
            "promoting a caught-up learner to voter"
        );
        if let Err(e) = self
            .raft
            .change_membership(
                ChangeMembers::AddVoterIds(BTreeSet::from([candidate])),
                true,
            )
            .await
        {
            tracing::warn!(error = %e, server_id = candidate, "could not promote learner");
        }
    }
}

pub fn target_voters(servers: usize) -> usize {
    let odd = if servers % 2 == 0 {
        servers.saturating_sub(1)
    } else {
        servers
    };
    odd.clamp(1, MAX_VOTERS)
}

async fn run_controller(quorium: std::sync::Weak<Quorium>) {
    let mut ticker = tokio::time::interval(CONTROL_TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let Some(quorium) = quorium.upgrade() else {
            return;
        };
        let metrics = quorium.metrics();
        if metrics.state != ServerState::Leader {
            continue;
        }
        quorium.expire_lapsed_sessions(metrics.current_term).await;
        quorium.promote_caught_up_learner(&metrics).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voter_targets_stay_odd_and_bounded() {
        assert_eq!(target_voters(0), 1);
        assert_eq!(target_voters(1), 1);
        assert_eq!(target_voters(2), 1);
        assert_eq!(target_voters(3), 3);
        assert_eq!(target_voters(4), 3);
        assert_eq!(target_voters(5), 5);
        assert_eq!(target_voters(9), 5);
    }

    #[test]
    fn digests_are_stable_hex() {
        assert_eq!(digest("a").len(), 64);
        assert_eq!(digest("a"), digest("a"));
        assert_ne!(digest("a"), digest("b"));
    }
}
