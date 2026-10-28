use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const EVICTION_MEMORY_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    pub node_id: String,
    pub internal_url: String,
    pub version: String,
    pub draining: bool,
    pub joined_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Command {
    Bootstrap { cluster_id: String },
    Register { member: MemberRecord },
    Withdraw { node_id: String },
    Expire { node_id: String, at_ms: u64 },
    IssueToken { digest: String, expires_at_ms: u64 },
    ConsumeToken { digest: String, at_ms: u64 },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    #[default]
    Done,
    Evicted,
    TokenAccepted(bool),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterState {
    pub cluster_id: String,
    pub members: BTreeMap<String, MemberRecord>,
    pub evicted: BTreeMap<String, u64>,
    pub tokens: BTreeMap<String, u64>,
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
                Response::Done
            }
            Command::Expire { node_id, at_ms } => {
                if self.members.remove(&node_id).is_some() {
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
        }
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
        }
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
}
