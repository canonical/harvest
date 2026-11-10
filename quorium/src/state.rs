use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const EVICTION_MEMORY_MS: u64 = 24 * 60 * 60 * 1000;

pub const FEATURE_LEASES: &str = "leases";

pub const FEATURES: &[&str] = &[FEATURE_LEASES];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub node_id: String,
    pub internal_url: String,
    pub version: String,
    pub draining: bool,
    pub joined_at_ms: u64,
    #[serde(default)]
    pub features: Vec<String>,
}

impl MemberRecord {
    pub fn supports(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub holder: String,
    pub token: u64,
    pub acquired_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Bootstrap {
        cluster_id: String,
    },
    Register {
        member: MemberRecord,
    },
    Withdraw {
        node_id: String,
    },
    Expire {
        node_id: String,
        at_ms: u64,
    },
    IssueToken {
        digest: String,
        expires_at_ms: u64,
    },
    ConsumeToken {
        digest: String,
        at_ms: u64,
    },
    RevokeTokens,
    AcquireLease {
        key: String,
        holder: String,
        at_ms: u64,
    },
    ReleaseLease {
        key: String,
        holder: String,
        token: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    #[default]
    Done,
    Evicted,
    TokenAccepted(bool),
    Lease(Option<u64>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterState {
    pub cluster_id: String,
    pub members: BTreeMap<String, MemberRecord>,
    pub evicted: BTreeMap<String, u64>,
    pub tokens: BTreeMap<String, u64>,
    #[serde(default)]
    pub leases: BTreeMap<String, Lease>,
    #[serde(default)]
    pub next_token: u64,
}

impl ClusterState {
    pub fn apply(&mut self, command: Command) -> Response {
        match command {
            Command::Bootstrap { cluster_id } => {
                if self.cluster_id.is_empty() {
                    self.cluster_id = cluster_id;
                }
                Response::Done
            }
            Command::Register { member } => self.register(member),
            Command::Withdraw { node_id } => {
                self.members.remove(&node_id);
                self.release_all(&node_id);
                Response::Done
            }
            Command::Expire { node_id, at_ms } => {
                if self.members.remove(&node_id).is_some() {
                    self.release_all(&node_id);
                    self.evicted.insert(node_id, at_ms);
                }
                self.forget_old_evictions(at_ms);
                Response::Done
            }
            Command::IssueToken {
                digest,
                expires_at_ms,
            } => {
                self.tokens.insert(digest, expires_at_ms);
                Response::Done
            }
            Command::ConsumeToken { digest, at_ms } => {
                self.tokens.retain(|_, expires| *expires > at_ms);
                Response::TokenAccepted(self.tokens.remove(&digest).is_some())
            }
            Command::RevokeTokens => {
                self.tokens.clear();
                Response::Done
            }
            Command::AcquireLease { key, holder, at_ms } => {
                Response::Lease(self.acquire(key, holder, at_ms))
            }
            Command::ReleaseLease { key, holder, token } => {
                let held = self
                    .leases
                    .get(&key)
                    .is_some_and(|l| l.holder == holder && l.token == token);
                if held {
                    self.leases.remove(&key);
                }
                Response::Done
            }
        }
    }

    fn acquire(&mut self, key: String, holder: String, at_ms: u64) -> Option<u64> {
        if !self.members.contains_key(&holder) {
            return None;
        }
        if let Some(lease) = self.leases.get(&key) {
            return (lease.holder == holder).then_some(lease.token);
        }
        self.next_token += 1;
        self.leases.insert(
            key,
            Lease {
                holder,
                token: self.next_token,
                acquired_at_ms: at_ms,
            },
        );
        Some(self.next_token)
    }

    fn release_all(&mut self, node_id: &str) {
        self.leases.retain(|_, lease| lease.holder != node_id);
    }

    pub fn is_evicted(&self, node_id: &str) -> bool {
        self.evicted.contains_key(node_id)
    }

    fn register(&mut self, member: MemberRecord) -> Response {
        if self.is_evicted(&member.node_id) {
            return Response::Evicted;
        }
        let joined_at_ms = self
            .members
            .get(&member.node_id)
            .map_or(member.joined_at_ms, |m| m.joined_at_ms);
        self.members.insert(
            member.node_id.clone(),
            MemberRecord {
                joined_at_ms,
                ..member
            },
        );
        Response::Done
    }

    fn forget_old_evictions(&mut self, now_ms: u64) {
        self.evicted
            .retain(|_, at| now_ms.saturating_sub(*at) < EVICTION_MEMORY_MS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, joined_at_ms: u64) -> MemberRecord {
        MemberRecord {
            node_id: id.into(),
            internal_url: format!("http://{id}"),
            version: "1".into(),
            draining: false,
            joined_at_ms,
            features: vec![FEATURE_LEASES.into()],
        }
    }

    fn acquire(state: &mut ClusterState, key: &str, holder: &str) -> Option<u64> {
        match state.apply(Command::AcquireLease {
            key: key.into(),
            holder: holder.into(),
            at_ms: 1,
        }) {
            Response::Lease(token) => token,
            other => panic!("unexpected response {other:?}"),
        }
    }

    #[test]
    fn a_lease_has_one_holder_and_reacquiring_returns_the_same_token() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 1),
        });
        state.apply(Command::Register {
            member: member("n2", 1),
        });
        let token = acquire(&mut state, "job:a", "n1").unwrap();
        assert_eq!(acquire(&mut state, "job:a", "n1"), Some(token));
        assert_eq!(acquire(&mut state, "job:a", "n2"), None);
    }

    #[test]
    fn only_registered_members_can_hold_leases() {
        let mut state = ClusterState::default();
        assert_eq!(acquire(&mut state, "job:a", "ghost"), None);
    }

    #[test]
    fn tokens_increase_across_keys_and_holders() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 1),
        });
        state.apply(Command::Register {
            member: member("n2", 1),
        });
        let first = acquire(&mut state, "job:a", "n1").unwrap();
        let second = acquire(&mut state, "job:b", "n2").unwrap();
        assert!(second > first);
    }

    #[test]
    fn expiring_or_withdrawing_a_member_releases_its_leases() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 1),
        });
        state.apply(Command::Register {
            member: member("n2", 1),
        });
        state.apply(Command::Register {
            member: member("n3", 1),
        });
        let old = acquire(&mut state, "job:a", "n1").unwrap();
        acquire(&mut state, "job:b", "n2").unwrap();
        state.apply(Command::Expire {
            node_id: "n1".into(),
            at_ms: 5,
        });
        state.apply(Command::Withdraw {
            node_id: "n2".into(),
        });
        let new = acquire(&mut state, "job:a", "n3").unwrap();
        assert!(new > old);
        assert_eq!(acquire(&mut state, "job:b", "n3").is_some(), true);
    }

    #[test]
    fn release_requires_the_matching_holder_and_token() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 1),
        });
        state.apply(Command::Register {
            member: member("n2", 1),
        });
        let token = acquire(&mut state, "job:a", "n1").unwrap();
        state.apply(Command::ReleaseLease {
            key: "job:a".into(),
            holder: "n2".into(),
            token,
        });
        state.apply(Command::ReleaseLease {
            key: "job:a".into(),
            holder: "n1".into(),
            token: token + 1,
        });
        assert_eq!(acquire(&mut state, "job:a", "n2"), None);
        state.apply(Command::ReleaseLease {
            key: "job:a".into(),
            holder: "n1".into(),
            token,
        });
        assert!(acquire(&mut state, "job:a", "n2").is_some());
    }

    #[test]
    fn snapshots_written_before_leases_existed_still_load() {
        let old = r#"{"cluster_id":"c","members":{},"evicted":{},"tokens":{}}"#;
        let state: ClusterState = serde_json::from_str(old).unwrap();
        assert!(state.leases.is_empty());
        assert_eq!(state.next_token, 0);
        let old_member =
            r#"{"node_id":"n","internal_url":"u","version":"1","draining":false,"joined_at_ms":0}"#;
        let record: MemberRecord = serde_json::from_str(old_member).unwrap();
        assert!(!record.supports(FEATURE_LEASES));
    }

    #[test]
    fn bootstrap_sets_the_cluster_id_once() {
        let mut state = ClusterState::default();
        state.apply(Command::Bootstrap {
            cluster_id: "a".into(),
        });
        state.apply(Command::Bootstrap {
            cluster_id: "b".into(),
        });
        assert_eq!(state.cluster_id, "a");
    }

    #[test]
    fn registering_again_updates_the_record_but_keeps_the_join_time() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 10),
        });
        let mut draining = member("n1", 99);
        draining.draining = true;
        state.apply(Command::Register { member: draining });
        assert_eq!(state.members["n1"].joined_at_ms, 10);
        assert!(state.members["n1"].draining);
    }

    #[test]
    fn an_expired_member_can_never_register_again() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 1),
        });
        state.apply(Command::Expire {
            node_id: "n1".into(),
            at_ms: 5,
        });
        assert!(state.members.is_empty());
        assert_eq!(
            state.apply(Command::Register {
                member: member("n1", 6)
            }),
            Response::Evicted
        );
        assert!(state.members.is_empty());
    }

    #[test]
    fn withdrawn_members_are_not_evicted() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("n1", 1),
        });
        state.apply(Command::Withdraw {
            node_id: "n1".into(),
        });
        assert!(!state.is_evicted("n1"));
    }

    #[test]
    fn evictions_are_forgotten_after_a_day() {
        let mut state = ClusterState::default();
        state.apply(Command::Register {
            member: member("old", 1),
        });
        state.apply(Command::Expire {
            node_id: "old".into(),
            at_ms: 1,
        });
        state.apply(Command::Expire {
            node_id: "none".into(),
            at_ms: 1 + EVICTION_MEMORY_MS,
        });
        assert!(!state.is_evicted("old"));
    }

    #[test]
    fn tokens_are_single_use_and_expire() {
        let mut state = ClusterState::default();
        state.apply(Command::IssueToken {
            digest: "t".into(),
            expires_at_ms: 100,
        });
        state.apply(Command::IssueToken {
            digest: "late".into(),
            expires_at_ms: 100,
        });
        assert_eq!(
            state.apply(Command::ConsumeToken {
                digest: "t".into(),
                at_ms: 50
            }),
            Response::TokenAccepted(true)
        );
        assert_eq!(
            state.apply(Command::ConsumeToken {
                digest: "t".into(),
                at_ms: 51
            }),
            Response::TokenAccepted(false)
        );
        assert_eq!(
            state.apply(Command::ConsumeToken {
                digest: "late".into(),
                at_ms: 100
            }),
            Response::TokenAccepted(false)
        );
    }

    #[test]
    fn revoking_tokens_invalidates_every_outstanding_token() {
        let mut state = ClusterState::default();
        state.apply(Command::IssueToken {
            digest: "t".into(),
            expires_at_ms: 100,
        });
        state.apply(Command::RevokeTokens);
        assert_eq!(
            state.apply(Command::ConsumeToken {
                digest: "t".into(),
                at_ms: 1
            }),
            Response::TokenAccepted(false)
        );
    }
}
