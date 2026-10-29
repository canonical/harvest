use std::fmt::Debug;
use std::io::Cursor;
use std::ops::RangeBounds;
use std::path::Path;
use std::sync::{Arc, RwLock};

use openraft::storage::{LogFlushed, RaftLogStorage, RaftStateMachine};
use openraft::{
    EntryPayload, LogState, OptionalSend, RaftLogId, RaftLogReader, RaftSnapshotBuilder,
    StorageIOError,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::state::{ClusterState, Response};
use crate::types::{
    Entry, LogId, ServerId, Snapshot, SnapshotMeta, StorageError, StoredMembership, TypeConfig,
    Vote,
};

pub const DATABASE_FILE: &str = "quorium.redb";
const LOGS: TableDefinition<u64, &[u8]> = TableDefinition::new("logs");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

const VOTE: &str = "vote";
const COMMITTED: &str = "committed";
const LAST_PURGED: &str = "last_purged";
const SNAPSHOT: &str = "snapshot";
const SERVER_ID: &str = "server_id";

fn read_error(e: impl std::error::Error + 'static) -> StorageError {
    StorageIOError::read(&e).into()
}

fn write_error(e: impl std::error::Error + 'static) -> StorageError {
    StorageIOError::write(&e).into()
}

#[derive(Clone)]
struct Disk(Arc<Database>);

impl Disk {
    fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StorageError> {
        let txn = self.0.begin_read().map_err(read_error)?;
        let table = match txn.open_table(META) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(read_error(e)),
        };
        let Some(value) = table.get(key).map_err(read_error)? else {
            return Ok(None);
        };
        serde_json::from_slice(value.value())
            .map(Some)
            .map_err(read_error)
    }

    async fn put<T: Serialize>(&self, key: &'static str, value: &T) -> Result<(), StorageError> {
        let bytes = serde_json::to_vec(value).map_err(write_error)?;
        self.write(move |txn| {
            txn.open_table(META)?.insert(key, bytes.as_slice())?;
            Ok(())
        })
        .await
    }

    async fn write<F>(&self, f: F) -> Result<(), StorageError>
    where
        F: FnOnce(&redb::WriteTransaction) -> Result<(), redb::Error> + Send + 'static,
    {
        let db = Arc::clone(&self.0);
        tokio::task::spawn_blocking(move || -> Result<(), redb::Error> {
            let txn = db.begin_write()?;
            f(&txn)?;
            txn.commit()?;
            Ok(())
        })
        .await
        .map_err(write_error)?
        .map_err(write_error)
    }
}

pub struct Storage {
    pub server_id: ServerId,
    pub log: LogStore,
    pub state_machine: StateMachineStore,
}

impl Storage {
    pub async fn open(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let disk = Disk(Arc::new(open_database(&dir.join(DATABASE_FILE))?));
        let server_id = match disk.get::<ServerId>(SERVER_ID)? {
            Some(id) => id,
            None => {
                let id = new_server_id();
                disk.put(SERVER_ID, &id).await?;
                id
            }
        };
        let state_machine = StateMachineStore::load(disk.clone())?;
        Ok(Self {
            server_id,
            log: LogStore { disk },
            state_machine,
        })
    }
}

fn open_database(path: &Path) -> anyhow::Result<Database> {
    Database::create(path).map_err(|e| match e {
        redb::DatabaseError::DatabaseAlreadyOpen => anyhow::anyhow!(
            "{} is in use by a running server; stop it first",
            path.display()
        ),
        other => anyhow::anyhow!(
            "{} is unreadable ({other}); move the data directory aside and recover the cluster",
            path.display()
        ),
    })
}

fn new_server_id() -> ServerId {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    u64::from_le_bytes(bytes[..8].try_into().expect("eight bytes")) >> 1
}

#[derive(Clone)]
pub struct LogStore {
    disk: Disk,
}

impl LogStore {
    fn last_entry(&self) -> Result<Option<Entry>, StorageError> {
        let txn = self.disk.0.begin_read().map_err(read_error)?;
        let table = match txn.open_table(LOGS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(read_error(e)),
        };
        let Some((_, value)) = table.last().map_err(read_error)? else {
            return Ok(None);
        };
        serde_json::from_slice(value.value())
            .map(Some)
            .map_err(read_error)
    }
}

impl RaftLogReader<TypeConfig> for LogStore {
    async fn try_get_log_entries<RB: RangeBounds<u64> + Clone + Debug + OptionalSend>(
        &mut self,
        range: RB,
    ) -> Result<Vec<Entry>, StorageError> {
        let txn = self.disk.0.begin_read().map_err(read_error)?;
        let table = match txn.open_table(LOGS) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(vec![]),
            Err(e) => return Err(read_error(e)),
        };
        table
            .range(range)
            .map_err(read_error)?
            .map(|item| {
                let (_, value) = item.map_err(read_error)?;
                serde_json::from_slice(value.value()).map_err(read_error)
            })
            .collect()
    }
}

impl RaftLogStorage<TypeConfig> for LogStore {
    type LogReader = Self;

    async fn get_log_state(&mut self) -> Result<LogState<TypeConfig>, StorageError> {
        let last_purged_log_id: Option<LogId> = self.disk.get(LAST_PURGED)?;
        let last_log_id = self
            .last_entry()?
            .map(|e| *e.get_log_id())
            .or(last_purged_log_id);
        Ok(LogState {
            last_purged_log_id,
            last_log_id,
        })
    }

    async fn get_log_reader(&mut self) -> Self::LogReader {
        self.clone()
    }

    async fn save_vote(&mut self, vote: &Vote) -> Result<(), StorageError> {
        self.disk.put(VOTE, vote).await
    }

    async fn read_vote(&mut self) -> Result<Option<Vote>, StorageError> {
        self.disk.get(VOTE)
    }

    async fn save_committed(&mut self, committed: Option<LogId>) -> Result<(), StorageError> {
        self.disk.put(COMMITTED, &committed).await
    }

    async fn read_committed(&mut self) -> Result<Option<LogId>, StorageError> {
        Ok(self.disk.get::<Option<LogId>>(COMMITTED)?.flatten())
    }

    async fn append<I>(
        &mut self,
        entries: I,
        callback: LogFlushed<TypeConfig>,
    ) -> Result<(), StorageError>
    where
        I: IntoIterator<Item = Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let encoded = entries
            .into_iter()
            .map(|e| {
                Ok((
                    e.get_log_id().index,
                    serde_json::to_vec(&e).map_err(write_error)?,
                ))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        let result = self
            .disk
            .write(move |txn| {
                let mut table = txn.open_table(LOGS)?;
                for (index, bytes) in &encoded {
                    table.insert(*index, bytes.as_slice())?;
                }
                Ok(())
            })
            .await;
        callback.log_io_completed(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|e| std::io::Error::other(e.to_string())),
        );
        result
    }

    async fn truncate(&mut self, log_id: LogId) -> Result<(), StorageError> {
        let from = log_id.index;
        self.disk
            .write(move |txn| {
                txn.open_table(LOGS)?.retain(|index, _| index < from)?;
                Ok(())
            })
            .await
    }

    async fn purge(&mut self, log_id: LogId) -> Result<(), StorageError> {
        self.disk.put(LAST_PURGED, &log_id).await?;
        let upto = log_id.index;
        self.disk
            .write(move |txn| {
                txn.open_table(LOGS)?.retain(|index, _| index > upto)?;
                Ok(())
            })
            .await
    }
}

#[derive(Clone, Debug, Default)]
pub struct Applied {
    pub last_applied: Option<LogId>,
    pub membership: StoredMembership,
    pub cluster: ClusterState,
}

#[derive(Serialize, Deserialize)]
struct StoredSnapshot {
    meta: SnapshotMeta,
    data: Vec<u8>,
}

#[derive(Clone)]
pub struct StateMachineStore {
    disk: Disk,
    applied: Arc<RwLock<Applied>>,
    changes: Arc<watch::Sender<u64>>,
}

impl StateMachineStore {
    fn load(disk: Disk) -> anyhow::Result<Self> {
        let mut applied = Applied::default();
        if let Some(stored) = disk.get::<StoredSnapshot>(SNAPSHOT)? {
            applied.cluster = serde_json::from_slice(&stored.data)?;
            applied.last_applied = stored.meta.last_log_id;
            applied.membership = stored.meta.last_membership;
        }
        let index = applied.last_applied.map_or(0, |l| l.index);
        Ok(Self {
            disk,
            applied: Arc::new(RwLock::new(applied)),
            changes: Arc::new(watch::channel(index).0),
        })
    }

    pub fn read<T>(&self, f: impl FnOnce(&Applied) -> T) -> T {
        f(&self.applied.read().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn publish(&self) {
        let index = self.read(|a| a.last_applied.map_or(0, |l| l.index));
        self.changes.send_replace(index);
    }
}

impl RaftSnapshotBuilder<TypeConfig> for StateMachineStore {
    async fn build_snapshot(&mut self) -> Result<Snapshot, StorageError> {
        let (meta, data) = self.read(|applied| {
            let data = serde_json::to_vec(&applied.cluster).map_err(read_error)?;
            let index = applied.last_applied.map_or(0, |l| l.index);
            let meta = SnapshotMeta {
                last_log_id: applied.last_applied,
                last_membership: applied.membership.clone(),
                snapshot_id: format!("{index}-{}", uuid::Uuid::new_v4().simple()),
            };
            Ok::<_, StorageError>((meta, data))
        })?;
        let stored = StoredSnapshot {
            meta: meta.clone(),
            data: data.clone(),
        };
        self.disk.put(SNAPSHOT, &stored).await?;
        Ok(Snapshot {
            meta,
            snapshot: Box::new(Cursor::new(data)),
        })
    }
}

impl RaftStateMachine<TypeConfig> for StateMachineStore {
    type SnapshotBuilder = Self;

    async fn applied_state(&mut self) -> Result<(Option<LogId>, StoredMembership), StorageError> {
        Ok(self.read(|a| (a.last_applied, a.membership.clone())))
    }

    async fn apply<I>(&mut self, entries: I) -> Result<Vec<Response>, StorageError>
    where
        I: IntoIterator<Item = Entry> + OptionalSend,
        I::IntoIter: OptionalSend,
    {
        let responses = {
            let mut applied = self.applied.write().unwrap_or_else(|e| e.into_inner());
            entries
                .into_iter()
                .map(|entry| {
                    applied.last_applied = Some(entry.log_id);
                    match entry.payload {
                        EntryPayload::Blank => Response::Done,
                        EntryPayload::Normal(command) => applied.cluster.apply(command),
                        EntryPayload::Membership(membership) => {
                            applied.membership =
                                StoredMembership::new(Some(entry.log_id), membership);
                            Response::Done
                        }
                    }
                })
                .collect()
        };
        self.publish();
        Ok(responses)
    }

    async fn get_snapshot_builder(&mut self) -> Self::SnapshotBuilder {
        self.clone()
    }

    async fn begin_receiving_snapshot(&mut self) -> Result<Box<Cursor<Vec<u8>>>, StorageError> {
        Ok(Box::new(Cursor::new(Vec::new())))
    }

    async fn install_snapshot(
        &mut self,
        meta: &SnapshotMeta,
        snapshot: Box<Cursor<Vec<u8>>>,
    ) -> Result<(), StorageError> {
        let data = snapshot.into_inner();
        let cluster: ClusterState = serde_json::from_slice(&data).map_err(read_error)?;
        self.disk
            .put(
                SNAPSHOT,
                &StoredSnapshot {
                    meta: meta.clone(),
                    data,
                },
            )
            .await?;
        {
            let mut applied = self.applied.write().unwrap_or_else(|e| e.into_inner());
            applied.cluster = cluster;
            applied.last_applied = meta.last_log_id;
            applied.membership = meta.last_membership.clone();
        }
        self.publish();
        Ok(())
    }

    async fn get_current_snapshot(&mut self) -> Result<Option<Snapshot>, StorageError> {
        Ok(self
            .disk
            .get::<StoredSnapshot>(SNAPSHOT)?
            .map(|stored| Snapshot {
                meta: stored.meta,
                snapshot: Box::new(Cursor::new(stored.data)),
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Command, MemberRecord};
    use openraft::{CommittedLeaderId, Entry as RaftEntry};

    fn entry(index: u64, command: Command) -> Entry {
        RaftEntry {
            log_id: LogId::new(CommittedLeaderId::new(1, 0), index),
            payload: EntryPayload::Normal(command),
        }
    }

    fn register(id: &str) -> Command {
        Command::Register {
            member: MemberRecord {
                node_id: id.into(),
                internal_url: String::new(),
                version: String::new(),
                draining: false,
                joined_at_ms: 0,
            },
        }
    }

    #[tokio::test]
    async fn the_server_id_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let first = Storage::open(dir.path()).await.unwrap().server_id;
        let second = Storage::open(dir.path()).await.unwrap().server_id;
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn a_file_in_use_is_reported_as_held_by_a_running_server() {
        let dir = tempfile::tempdir().unwrap();
        let _running = Storage::open(dir.path()).await.unwrap();
        let error = Storage::open(dir.path()).await.err().unwrap().to_string();
        assert!(error.contains("in use by a running server"), "{error}");
    }

    #[tokio::test]
    async fn a_corrupted_file_points_at_recovery() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(DATABASE_FILE),
            b"definitely not a redb file",
        )
        .unwrap();
        let error = Storage::open(dir.path()).await.err().unwrap().to_string();
        assert!(error.contains("recover the cluster"), "{error}");
    }

    #[tokio::test]
    async fn thousands_of_entries_survive_snapshot_and_purge() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path()).await.unwrap();
        let mut log = storage.log;
        let mut machine = storage.state_machine;
        let entries: Vec<Entry> = (1..=5_000)
            .map(|i| entry(i, register(&format!("n{}", i % 50))))
            .collect();
        let encoded = entries
            .iter()
            .map(|e| (e.log_id.index, serde_json::to_vec(e).unwrap()))
            .collect::<Vec<_>>();
        log.disk
            .write(move |txn| {
                let mut table = txn.open_table(LOGS)?;
                for (index, bytes) in &encoded {
                    table.insert(*index, bytes.as_slice())?;
                }
                Ok(())
            })
            .await
            .unwrap();
        machine.apply(entries).await.unwrap();
        machine.build_snapshot().await.unwrap();
        log.purge(LogId::new(CommittedLeaderId::new(1, 0), 4_800))
            .await
            .unwrap();
        assert_eq!(log.try_get_log_entries(0..10_000).await.unwrap().len(), 200);
        drop((log, machine));
        let reopened = Storage::open(dir.path()).await.unwrap().state_machine;
        reopened.read(|applied| {
            assert_eq!(applied.cluster.members.len(), 50);
            assert_eq!(applied.last_applied.unwrap().index, 5_000);
        });
    }

    #[tokio::test]
    async fn the_vote_is_durable() {
        let dir = tempfile::tempdir().unwrap();
        let vote = Vote::new(3, 7);
        Storage::open(dir.path())
            .await
            .unwrap()
            .log
            .save_vote(&vote)
            .await
            .unwrap();
        let mut reopened = Storage::open(dir.path()).await.unwrap().log;
        assert_eq!(reopened.read_vote().await.unwrap(), Some(vote));
    }

    #[tokio::test]
    async fn logs_can_be_read_truncated_and_purged() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = Storage::open(dir.path()).await.unwrap().log;
        let encoded = (1..=5)
            .map(|i| {
                (
                    i,
                    serde_json::to_vec(&entry(i, register(&i.to_string()))).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        log.disk
            .write(move |txn| {
                let mut table = txn.open_table(LOGS)?;
                for (index, bytes) in &encoded {
                    table.insert(*index, bytes.as_slice())?;
                }
                Ok(())
            })
            .await
            .unwrap();

        assert_eq!(log.try_get_log_entries(2..4).await.unwrap().len(), 2);
        log.truncate(LogId::new(CommittedLeaderId::new(1, 0), 4))
            .await
            .unwrap();
        assert_eq!(
            log.get_log_state()
                .await
                .unwrap()
                .last_log_id
                .unwrap()
                .index,
            3
        );
        log.purge(LogId::new(CommittedLeaderId::new(1, 0), 2))
            .await
            .unwrap();
        let state = log.get_log_state().await.unwrap();
        assert_eq!(state.last_purged_log_id.unwrap().index, 2);
        assert_eq!(log.try_get_log_entries(0..10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_snapshot_restores_the_state_machine_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut machine = Storage::open(dir.path()).await.unwrap().state_machine;
        machine
            .apply(vec![entry(1, register("a")), entry(2, register("b"))])
            .await
            .unwrap();
        machine.build_snapshot().await.unwrap();
        drop(machine);

        let reopened = Storage::open(dir.path()).await.unwrap().state_machine;
        reopened.read(|applied| {
            assert_eq!(applied.cluster.members.len(), 2);
            assert_eq!(applied.last_applied.unwrap().index, 2);
        });
    }
}
