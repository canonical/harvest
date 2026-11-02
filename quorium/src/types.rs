use std::io::Cursor;

use crate::state::{Command, Response};

pub type ServerId = u64;
pub type Node = openraft::BasicNode;

openraft::declare_raft_types!(
    pub TypeConfig:
        D = Command,
        R = Response,
        NodeId = ServerId,
        Node = Node,
        SnapshotData = Cursor<Vec<u8>>,
);

pub type Raft = openraft::Raft<TypeConfig>;
pub type Entry = openraft::Entry<TypeConfig>;
pub type LogId = openraft::LogId<ServerId>;
pub type Vote = openraft::Vote<ServerId>;
pub type StorageError = openraft::StorageError<ServerId>;
pub type StoredMembership = openraft::StoredMembership<ServerId, Node>;
pub type SnapshotMeta = openraft::SnapshotMeta<ServerId, Node>;
pub type Snapshot = openraft::Snapshot<TypeConfig>;
pub type Metrics = openraft::RaftMetrics<ServerId, Node>;
