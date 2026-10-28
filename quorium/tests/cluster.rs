use std::sync::Arc;
use std::time::Duration;

use quorium::{Command, KeepAlive, MemberRecord, Quorium, QuoriumConfig};
use tokio::net::TcpListener;

struct Server {
    quorium: Arc<Quorium>,
    serving: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

impl Server {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let dir = tempfile::tempdir().unwrap();
        let quorium = Quorium::start(QuoriumConfig {
            data_dir: dir.path().to_path_buf(),
            advertise_url: url,
            heartbeat_interval: Duration::from_millis(50),
            election_timeout: Duration::from_millis(300),
            session_ttl: Duration::from_millis(800),
            http: reqwest::Client::new(),
        })
        .await
        .unwrap();
        let router = quorium::api::router(Arc::clone(&quorium));
        let serving = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            quorium,
            serving,
            _dir: dir,
        }
    }

    async fn stop(self) {
        self.serving.abort();
        self.quorium.shutdown().await;
    }
}

fn member(id: &str) -> MemberRecord {
    MemberRecord {
        node_id: id.into(),
        internal_url: format!("http://{id}"),
        version: "test".into(),
        draining: false,
        joined_at_ms: 0,
    }
}

async fn eventually<F: Fn() -> bool>(what: &str, timeout: Duration, check: F) {
    let deadline = tokio::time::Instant::now() + timeout;
    while !check() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn three_voters() -> Vec<Server> {
    let first = Server::start().await;
    first.quorium.initialize().await.unwrap();
    let mut servers = vec![first];
    for _ in 0..2 {
        let token = servers[0]
            .quorium
            .issue_token(Duration::from_secs(60))
            .await
            .unwrap();
        let server = Server::start().await;
        server
            .quorium
            .join(&token, Duration::from_secs(5))
            .await
            .unwrap();
        servers.push(server);
    }
    let first = Arc::clone(&servers[0].quorium);
    eventually("three voters", Duration::from_secs(10), || {
        first.servers().iter().filter(|s| s.voter).count() == 3
    })
    .await;
    servers
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_server_bootstraps_and_serves_members() {
    let server = Server::start().await;
    server.quorium.initialize().await.unwrap();
    assert!(!server.quorium.cluster_id().is_empty());
    server
        .quorium
        .write(Command::Register {
            member: member("n1"),
        })
        .await
        .unwrap();
    assert_eq!(
        server.quorium.keepalive("n1").await.unwrap(),
        KeepAlive::Alive
    );
    assert_eq!(
        server.quorium.members_linearizable().await.unwrap().len(),
        1
    );
    assert!(server.quorium.health().quorum);
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_admit_servers_once_and_learners_are_promoted() {
    let servers = three_voters().await;
    let reused = servers[0]
        .quorium
        .issue_token(Duration::from_secs(60))
        .await
        .unwrap();
    let late = Server::start().await;
    late.quorium
        .join(&reused, Duration::from_secs(5))
        .await
        .unwrap();
    let intruder = Server::start().await;
    assert!(intruder
        .quorium
        .join(&reused, Duration::from_secs(1))
        .await
        .is_err());
    let first = Arc::clone(&servers[0].quorium);
    eventually("four servers, three voters", Duration::from_secs(5), || {
        let all = first.servers();
        all.len() == 4 && all.iter().filter(|s| s.voter).count() == 3
    })
    .await;
    late.stop().await;
    intruder.stop().await;
    for server in servers {
        server.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_from_followers_reach_the_leader_and_replicate_everywhere() {
    let servers = three_voters().await;
    let follower = servers.iter().find(|s| !s.quorium.is_leader()).unwrap();
    follower
        .quorium
        .write(Command::Register {
            member: member("n1"),
        })
        .await
        .unwrap();
    assert_eq!(
        follower.quorium.keepalive("n1").await.unwrap(),
        KeepAlive::Alive
    );
    for server in &servers {
        let q = Arc::clone(&server.quorium);
        eventually("replicated member", Duration::from_secs(5), || {
            q.members().len() == 1
        })
        .await;
    }
    assert_eq!(
        follower.quorium.members_linearizable().await.unwrap().len(),
        1
    );
    for server in servers {
        server.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn silent_members_are_evicted_for_good() {
    let server = Server::start().await;
    server.quorium.initialize().await.unwrap();
    server
        .quorium
        .write(Command::Register {
            member: member("quiet"),
        })
        .await
        .unwrap();
    server
        .quorium
        .write(Command::Register {
            member: member("chatty"),
        })
        .await
        .unwrap();
    let q = Arc::clone(&server.quorium);
    let keeper = tokio::spawn(async move {
        loop {
            let _ = q.keepalive("chatty").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let q = Arc::clone(&server.quorium);
    eventually("eviction", Duration::from_secs(5), || {
        let ids: Vec<String> = q.members().into_iter().map(|m| m.node_id).collect();
        ids == vec!["chatty".to_string()]
    })
    .await;
    assert_eq!(
        server.quorium.keepalive("quiet").await.unwrap(),
        KeepAlive::Evicted
    );
    assert_eq!(
        server
            .quorium
            .write(Command::Register {
                member: member("quiet")
            })
            .await
            .unwrap(),
        quorium::Response::Evicted
    );
    keeper.abort();
    server.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cluster_survives_losing_its_leader() {
    let mut servers = three_voters().await;
    servers[0]
        .quorium
        .write(Command::Register {
            member: member("n1"),
        })
        .await
        .unwrap();
    let leader_index = servers.iter().position(|s| s.quorium.is_leader()).unwrap();
    let leader = servers.remove(leader_index);
    let old_leader_id = leader.quorium.id();
    leader.stop().await;

    let survivors: Vec<Arc<Quorium>> = servers.iter().map(|s| Arc::clone(&s.quorium)).collect();
    eventually("a new leader", Duration::from_secs(10), || {
        survivors
            .iter()
            .any(|q| q.is_leader() && q.id() != old_leader_id)
    })
    .await;
    let survivor = &survivors[0];
    survivor
        .write(Command::Register {
            member: member("n2"),
        })
        .await
        .unwrap();
    assert_eq!(survivor.keepalive("n2").await.unwrap(), KeepAlive::Alive);
    assert!(survivors.iter().all(|q| {
        let h = q.health();
        h.leader.is_some()
    }));
    for server in servers {
        server.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_minority_loses_quorum() {
    let mut servers = three_voters().await;
    let a = servers.remove(0);
    let b = servers.remove(0);
    let lonely = servers.remove(0);
    a.stop().await;
    b.stop().await;
    let q = Arc::clone(&lonely.quorium);
    eventually("lost quorum", Duration::from_secs(10), || {
        !q.health().quorum
    })
    .await;
    assert!(tokio::time::timeout(
        Duration::from_secs(5),
        lonely.quorium.write(Command::Register {
            member: member("x")
        })
    )
    .await
    .map_or(true, |r| r.is_err()));
    lonely.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_removed_server_leaves_the_voters() {
    let servers = three_voters().await;
    let victim = servers
        .iter()
        .find(|s| !s.quorium.is_leader())
        .unwrap()
        .quorium
        .id();
    servers[0].quorium.remove_server(victim).await.unwrap();
    let first = Arc::clone(&servers[0].quorium);
    eventually("two servers", Duration::from_secs(5), || {
        first.servers().iter().all(|s| s.id != victim)
    })
    .await;
    for server in servers {
        server.stop().await;
    }
}
