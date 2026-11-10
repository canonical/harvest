use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use harvest_db::Db;
use quorium::Quorium;
use serde_json::json;
use tokio::sync::watch;

use crate::cluster::node::{ClusterNode, NodeStanding};

#[async_trait]
pub trait Leases: Send + Sync {
    async fn try_acquire(&self, key: &str) -> Result<Option<Lease>>;
}

#[async_trait]
pub trait Liveness: Send + Sync {
    async fn live_node_ids(&self) -> Result<Vec<String>>;
}

#[async_trait]
impl Liveness for ClusterNode {
    async fn live_node_ids(&self) -> Result<Vec<String>> {
        ClusterNode::live_node_ids(self).await
    }
}

type Release = Box<dyn FnOnce() + Send + Sync>;

pub struct Lease {
    key: String,
    token: u64,
    lost: watch::Receiver<bool>,
    release: Mutex<Option<Release>>,
}

impl Lease {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn token(&self) -> u64 {
        self.token
    }

    pub fn is_lost(&self) -> bool {
        *self.lost.borrow()
    }

    pub fn watch_lost(&self) -> watch::Receiver<bool> {
        self.lost.clone()
    }

    pub async fn lost(&self) {
        let mut lost = self.lost.clone();
        if lost.wait_for(|lost| *lost).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let release = self
            .release
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(release) = release {
            release();
        }
    }
}

fn standing_lost(standing: &watch::Receiver<NodeStanding>) -> bool {
    *standing.borrow() != NodeStanding::Confirmed
}

pub struct QuoriumLeases {
    quorium: Arc<Quorium>,
    holder: String,
    standing: watch::Receiver<NodeStanding>,
}

impl QuoriumLeases {
    pub fn new(
        quorium: Arc<Quorium>,
        holder: impl Into<String>,
        standing: watch::Receiver<NodeStanding>,
    ) -> Arc<Self> {
        Arc::new(Self {
            quorium,
            holder: holder.into(),
            standing,
        })
    }
}

fn quorium_still_held(quorium: &Quorium, key: &str, holder: &str, token: u64) -> bool {
    quorium
        .lease(key)
        .is_some_and(|l| l.holder == holder && l.token == token)
}

#[async_trait]
impl Leases for QuoriumLeases {
    async fn try_acquire(&self, key: &str) -> Result<Option<Lease>> {
        if standing_lost(&self.standing) {
            return Ok(None);
        }
        let Some(token) = self.quorium.try_acquire_lease(key, &self.holder).await? else {
            return Ok(None);
        };
        let (lost_tx, lost_rx) = watch::channel(false);
        let quorium = Arc::clone(&self.quorium);
        let holder = self.holder.clone();
        let mut standing = self.standing.clone();
        let mut changes = quorium.subscribe();
        let watched_key = key.to_string();
        let watcher = Arc::clone(&quorium);
        let watch_holder = holder.clone();
        tokio::spawn(async move {
            loop {
                if standing_lost(&standing)
                    || !quorium_still_held(&watcher, &watched_key, &watch_holder, token)
                {
                    let _ = lost_tx.send(true);
                    return;
                }
                tokio::select! {
                    _ = lost_tx.closed() => return,
                    changed = changes.changed() => if changed.is_err() {
                        let _ = lost_tx.send(true);
                        return;
                    },
                    changed = standing.changed() => if changed.is_err() {
                        let _ = lost_tx.send(true);
                        return;
                    },
                }
            }
        });
        let release_key = key.to_string();
        let release: Release = Box::new(move || {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    if let Err(e) = quorium.release_lease(&release_key, &holder, token).await {
                        tracing::debug!(error = %e, key = release_key, "releasing lease failed");
                    }
                });
            }
        });
        Ok(Some(Lease {
            key: key.to_string(),
            token,
            lost: lost_rx,
            release: Mutex::new(Some(release)),
        }))
    }
}

const ACQUIRE_SQL: &str = r#"
INSERT INTO cluster_leases (key, holder, token, acquired_at)
VALUES ($key, $holder, nextval('cluster_lease_tokens'), now())
ON CONFLICT (key) DO UPDATE
   SET holder = EXCLUDED.holder,
       token = CASE WHEN cluster_leases.holder = EXCLUDED.holder
                    THEN cluster_leases.token ELSE EXCLUDED.token END,
       acquired_at = CASE WHEN cluster_leases.holder = EXCLUDED.holder
                          THEN cluster_leases.acquired_at ELSE now() END
 WHERE cluster_leases.holder = EXCLUDED.holder
    OR NOT (cluster_leases.holder = ANY($alive::text[]))
RETURNING token
"#;

const HELD_SQL: &str =
    "SELECT EXISTS (SELECT 1 FROM cluster_leases WHERE key = $key AND holder = $holder AND token = $token) AS held";

const RELEASE_SQL: &str =
    "DELETE FROM cluster_leases WHERE key = $key AND holder = $holder AND token = $token";

pub struct PostgresLeases {
    db: Arc<Db>,
    holder: String,
    liveness: Arc<dyn Liveness>,
    standing: watch::Receiver<NodeStanding>,
    check_interval: Duration,
}

impl PostgresLeases {
    pub fn new(
        db: Arc<Db>,
        holder: impl Into<String>,
        liveness: Arc<dyn Liveness>,
        standing: watch::Receiver<NodeStanding>,
        check_interval: Duration,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            holder: holder.into(),
            liveness,
            standing,
            check_interval,
        })
    }
}

async fn still_held(db: &Db, key: &str, holder: &str, token: u64) -> bool {
    db.query(
        HELD_SQL,
        json!({ "key": key, "holder": holder, "token": token as i64 }),
    )
    .await
    .ok()
    .and_then(|rows| rows.first().and_then(|r| r["held"].as_bool()))
    .unwrap_or(false)
}

#[async_trait]
impl Leases for PostgresLeases {
    async fn try_acquire(&self, key: &str) -> Result<Option<Lease>> {
        if standing_lost(&self.standing) {
            return Ok(None);
        }
        let mut alive = self.liveness.live_node_ids().await?;
        if !alive.contains(&self.holder) {
            alive.push(self.holder.clone());
        }
        let rows = self
            .db
            .query(
                ACQUIRE_SQL,
                json!({ "key": key, "holder": self.holder, "alive": alive }),
            )
            .await?;
        let Some(token) = rows.first().and_then(|r| r["token"].as_i64()) else {
            return Ok(None);
        };
        let token = token as u64;
        let (lost_tx, lost_rx) = watch::channel(false);
        let db = Arc::clone(&self.db);
        let holder = self.holder.clone();
        let mut standing = self.standing.clone();
        let interval = self.check_interval;
        let watched_key = key.to_string();
        let watch_db = Arc::clone(&db);
        let watch_holder = holder.clone();
        tokio::spawn(async move {
            loop {
                if standing_lost(&standing)
                    || !still_held(&watch_db, &watched_key, &watch_holder, token).await
                {
                    let _ = lost_tx.send(true);
                    return;
                }
                tokio::select! {
                    _ = lost_tx.closed() => return,
                    _ = tokio::time::sleep(interval) => {},
                    changed = standing.changed() => if changed.is_err() {
                        let _ = lost_tx.send(true);
                        return;
                    },
                }
            }
        });
        let release_key = key.to_string();
        let release: Release = Box::new(move || {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let params =
                        json!({ "key": release_key, "holder": holder, "token": token as i64 });
                    if let Err(e) = db.execute(RELEASE_SQL, params).await {
                        tracing::debug!(error = %e, "releasing lease failed");
                    }
                });
            }
        });
        Ok(Some(Lease {
            key: key.to_string(),
            token,
            lost: lost_rx,
            release: Mutex::new(Some(release)),
        }))
    }
}

pub async fn run_exclusive<F, Fut>(leases: &dyn Leases, key: &str, job: F) -> Result<bool>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let Some(lease) = leases.try_acquire(key).await? else {
        return Ok(false);
    };
    tokio::select! {
        _ = job() => {},
        _ = lease.lost() => tracing::warn!(key, "lease lost while its job was running; stopping the job"),
    }
    Ok(true)
}

pub fn for_node(node: &Arc<ClusterNode>, quorium: Option<&Arc<Quorium>>) -> Arc<dyn Leases> {
    match quorium {
        Some(quorium) => {
            QuoriumLeases::new(Arc::clone(quorium), node.node_id(), node.watch_standing())
        }
        None => PostgresLeases::new(
            Arc::clone(node.db()),
            node.node_id(),
            Arc::clone(node) as Arc<dyn Liveness>,
            node.watch_standing(),
            node.heartbeat_interval(),
        ),
    }
}

#[cfg(test)]
pub mod conformance {
    use super::*;

    pub struct Subject {
        pub first: Arc<dyn Leases>,
        pub second: Arc<dyn Leases>,
        pub kill_first: Box<dyn Fn() -> futures::future::BoxFuture<'static, ()> + Send + Sync>,
        pub patience: Duration,
    }

    async fn eventually_acquire(leases: &Arc<dyn Leases>, key: &str, patience: Duration) -> Lease {
        let deadline = tokio::time::Instant::now() + patience;
        loop {
            if let Some(lease) = leases.try_acquire(key).await.unwrap() {
                return lease;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "never acquired {key}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn a_lease_has_one_holder(subject: &Subject) {
        let lease = subject.first.try_acquire("one").await.unwrap().unwrap();
        assert!(subject.second.try_acquire("one").await.unwrap().is_none());
        let again = subject.first.try_acquire("one").await.unwrap().unwrap();
        assert_eq!(again.token(), lease.token());
        assert!(!lease.is_lost());
        drop(again);
        drop(lease);
    }

    pub async fn dropping_a_lease_frees_it_for_a_higher_token(subject: &Subject) {
        let lease = subject.first.try_acquire("two").await.unwrap().unwrap();
        let first_token = lease.token();
        drop(lease);
        let next = eventually_acquire(&subject.second, "two", subject.patience).await;
        assert!(next.token() > first_token);
    }

    pub async fn a_dead_holder_loses_its_lease(subject: &Subject) {
        let lease = subject.first.try_acquire("three").await.unwrap().unwrap();
        (subject.kill_first)().await;
        let next = eventually_acquire(&subject.second, "three", subject.patience).await;
        assert!(next.token() > lease.token());
        tokio::time::timeout(subject.patience, lease.lost())
            .await
            .expect("the old holder learns that it lost the lease");
    }

    pub async fn run_all(subject: &Subject) {
        a_lease_has_one_holder(subject).await;
        dropping_a_lease_frees_it_for_a_higher_token(subject).await;
        a_dead_holder_loses_its_lease(subject).await;
    }
}

#[cfg(test)]
mod tests {
    use super::conformance::*;
    use super::*;
    use quorium::{Command, MemberRecord, QuoriumConfig, Tuning};

    struct Alive(Mutex<Vec<String>>);

    #[async_trait]
    impl Liveness for Alive {
        async fn live_node_ids(&self) -> Result<Vec<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn postgres_leases_conform() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = Arc::new(test_db.db.clone());
        let alive = Arc::new(Alive(Mutex::new(vec!["a".into(), "b".into()])));
        let (_standing_tx, standing) = watch::channel(NodeStanding::Confirmed);
        let interval = Duration::from_millis(50);
        let first = PostgresLeases::new(
            Arc::clone(&db),
            "a",
            Arc::clone(&alive) as Arc<dyn Liveness>,
            standing.clone(),
            interval,
        );
        let second = PostgresLeases::new(
            Arc::clone(&db),
            "b",
            Arc::clone(&alive) as Arc<dyn Liveness>,
            standing,
            interval,
        );
        let killer = Arc::clone(&alive);
        let subject = Subject {
            first,
            second,
            kill_first: Box::new(move || {
                let killer = Arc::clone(&killer);
                Box::pin(async move {
                    killer.0.lock().unwrap().retain(|id| id != "a");
                })
            }),
            patience: Duration::from_secs(5),
        };
        run_all(&subject).await;
    }

    fn member(id: &str) -> MemberRecord {
        MemberRecord {
            node_id: id.into(),
            internal_url: format!("http://{id}"),
            version: "test".into(),
            draining: false,
            joined_at_ms: 0,
            features: quorium::FEATURES.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quorium_leases_conform() {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let quorium = Quorium::start(QuoriumConfig {
            data_dir: dir.path().to_path_buf(),
            advertise_url: url,
            heartbeat_interval: Duration::from_millis(50),
            election_timeout: Duration::from_millis(300),
            session_ttl: Duration::from_millis(800),
            http: reqwest::Client::new(),
            admit_without_token: false,
            tuning: Tuning::default(),
        })
        .await
        .unwrap();
        let router = quorium::api::router(Arc::clone(&quorium));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        quorium.initialize().await.unwrap();
        for id in ["a", "b"] {
            quorium
                .write(Command::Register { member: member(id) })
                .await
                .unwrap();
        }
        let (_standing_tx, standing) = watch::channel(NodeStanding::Confirmed);
        let killer = Arc::clone(&quorium);
        let subject = Subject {
            first: QuoriumLeases::new(Arc::clone(&quorium), "a", standing.clone()),
            second: QuoriumLeases::new(Arc::clone(&quorium), "b", standing),
            kill_first: Box::new(move || {
                let killer = Arc::clone(&killer);
                Box::pin(async move {
                    killer
                        .write(Command::Withdraw {
                            node_id: "a".into(),
                        })
                        .await
                        .unwrap();
                })
            }),
            patience: Duration::from_secs(5),
        };
        run_all(&subject).await;
        quorium.shutdown().await;
    }

    struct Always(Option<u64>);

    #[async_trait]
    impl Leases for Always {
        async fn try_acquire(&self, key: &str) -> Result<Option<Lease>> {
            Ok(self.0.map(|token| Lease {
                key: key.to_string(),
                token,
                lost: watch::channel(false).1,
                release: Mutex::new(None),
            }))
        }
    }

    #[tokio::test]
    async fn run_exclusive_runs_only_with_the_lease() {
        let ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = Arc::clone(&ran);
        assert!(!run_exclusive(&Always(None), "k", || async move {
            flag.store(true, std::sync::atomic::Ordering::SeqCst)
        })
        .await
        .unwrap());
        assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));
        let flag = Arc::clone(&ran);
        assert!(run_exclusive(&Always(Some(1)), "k", || async move {
            flag.store(true, std::sync::atomic::Ordering::SeqCst)
        })
        .await
        .unwrap());
        assert!(ran.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_fenced_node_acquires_nothing_and_loses_what_it_holds() {
        let (standing_tx, standing) = watch::channel(NodeStanding::Confirmed);
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let quorium = Quorium::start(QuoriumConfig {
            data_dir: dir.path().to_path_buf(),
            advertise_url: url,
            heartbeat_interval: Duration::from_millis(50),
            election_timeout: Duration::from_millis(300),
            session_ttl: Duration::from_millis(800),
            http: reqwest::Client::new(),
            admit_without_token: false,
            tuning: Tuning::default(),
        })
        .await
        .unwrap();
        let router = quorium::api::router(Arc::clone(&quorium));
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        quorium.initialize().await.unwrap();
        quorium
            .write(Command::Register {
                member: member("a"),
            })
            .await
            .unwrap();
        let leases = QuoriumLeases::new(Arc::clone(&quorium), "a", standing);
        let lease = leases.try_acquire("job").await.unwrap().unwrap();
        standing_tx.send(NodeStanding::Fenced).unwrap();
        tokio::time::timeout(Duration::from_secs(2), lease.lost())
            .await
            .unwrap();
        assert!(leases.try_acquire("other").await.unwrap().is_none());
        quorium.shutdown().await;
    }
}
