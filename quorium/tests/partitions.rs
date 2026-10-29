use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use quorium::{Command, MemberRecord, Quorium, QuoriumConfig, Response as Outcome, Tuning};
use reqwest::header::{HeaderMap, HeaderValue};
use tokio::net::TcpListener;

const ORIGIN: &str = "x-test-origin";
const SERVERS: usize = 3;
const ROUNDS: usize = 16;
const ROUND: Duration = Duration::from_millis(200);

#[derive(Default)]
struct Network {
    cut: Mutex<BTreeSet<(usize, usize)>>,
}

impl Network {
    fn isolate(&self, server: usize) {
        let mut cut = self.cut.lock().unwrap();
        cut.clear();
        for other in (0..SERVERS).filter(|o| *o != server) {
            cut.insert((server, other));
            cut.insert((other, server));
        }
    }

    fn heal(&self) {
        self.cut.lock().unwrap().clear();
    }

    fn allows(&self, from: usize, to: usize) -> bool {
        !self.cut.lock().unwrap().contains(&(from, to))
    }
}

#[derive(Clone)]
struct Gate {
    network: Arc<Network>,
    server: usize,
}

async fn gate(State(gate): State<Gate>, request: Request, next: Next) -> Response {
    let from = request
        .headers()
        .get(ORIGIN)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    match from {
        Some(from) if !gate.network.allows(from, gate.server) => {
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        _ => next.run(request).await,
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

struct Server {
    quorium: Arc<Quorium>,
    serving: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

async fn start(index: usize, network: &Arc<Network>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let dir = tempfile::tempdir().unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(ORIGIN, HeaderValue::from_str(&index.to_string()).unwrap());
    let quorium = Quorium::start(QuoriumConfig {
        data_dir: dir.path().to_path_buf(),
        advertise_url: url,
        heartbeat_interval: Duration::from_millis(50),
        election_timeout: Duration::from_millis(300),
        session_ttl: Duration::from_millis(900),
        http: reqwest::Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_millis(500))
            .build()
            .unwrap(),
        admit_without_token: false,
        tuning: Tuning::default(),
    })
    .await
    .unwrap();
    let router =
        quorium::api::router(Arc::clone(&quorium)).layer(axum::middleware::from_fn_with_state(
            Gate {
                network: Arc::clone(network),
                server: index,
            },
            gate,
        ));
    let serving = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server {
        quorium,
        serving,
        _dir: dir,
    }
}

async fn cluster(network: &Arc<Network>) -> Vec<Server> {
    let first = start(0, network).await;
    first.quorium.initialize().await.unwrap();
    let mut servers = vec![first];
    for index in 1..SERVERS {
        let token = servers[0]
            .quorium
            .issue_token(Duration::from_secs(60))
            .await
            .unwrap();
        let server = start(index, network).await;
        server
            .quorium
            .join(&token, Duration::from_secs(10))
            .await
            .unwrap();
        servers.push(server);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while servers[0]
        .quorium
        .servers()
        .iter()
        .filter(|s| s.voter)
        .count()
        < SERVERS
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "voters were never promoted"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    servers
}

fn member(node_id: &str) -> MemberRecord {
    MemberRecord {
        node_id: node_id.into(),
        internal_url: String::new(),
        version: "fuzz".into(),
        draining: false,
        joined_at_ms: 0,
    }
}

#[derive(Default)]
struct History {
    leaders_by_term: BTreeMap<u64, BTreeSet<u64>>,
    violations: Vec<String>,
}

impl History {
    fn observe(&mut self, servers: &[Server]) {
        for server in servers {
            let status = server.quorium.status();
            if let Some(leader) = status.leader {
                self.leaders_by_term
                    .entry(status.term)
                    .or_default()
                    .insert(leader);
            }
            let members: BTreeSet<String> =
                status.members.iter().map(|m| m.node_id.clone()).collect();
            for evicted in server.quorium.evicted() {
                if members.contains(&evicted) {
                    self.violations.push(format!(
                        "server {} lists evicted {evicted} as a member",
                        status.server_id
                    ));
                }
            }
        }
    }

    fn assert_safe(&self, seed: u64) {
        for (term, leaders) in &self.leaders_by_term {
            assert!(
                leaders.len() <= 1,
                "seed {seed}: term {term} had leaders {leaders:?}"
            );
        }
        assert!(
            self.violations.is_empty(),
            "seed {seed}: {:?}",
            self.violations
        );
    }
}

async fn keep_alive(quorium: Arc<Quorium>, node_id: String) {
    loop {
        let _ = quorium.keepalive(&node_id).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn run_seed(seed: u64) {
    let network = Arc::new(Network::default());
    let servers = cluster(&network).await;
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut keepers: HashMap<String, tokio::task::JoinHandle<()>> = HashMap::new();
    for (index, server) in servers.iter().enumerate() {
        let node_id = format!("member-{index}");
        servers[0]
            .quorium
            .write(Command::Register {
                member: member(&node_id),
            })
            .await
            .unwrap();
        keepers.insert(
            node_id.clone(),
            tokio::spawn(keep_alive(Arc::clone(&server.quorium), node_id)),
        );
    }
    let mut history = History::default();
    let mut evicted_seen: BTreeSet<String> = BTreeSet::new();
    for _ in 0..ROUNDS {
        match rng.below(10) {
            0..=3 => {
                if let Some(leader) = servers.iter().position(|s| s.quorium.is_leader()) {
                    network.isolate(leader);
                }
            }
            4..=6 => network.isolate(rng.below(SERVERS as u64) as usize),
            _ => network.heal(),
        }
        tokio::time::sleep(ROUND + Duration::from_millis(rng.below(1_400))).await;
        history.observe(&servers);
        for server in &servers {
            evicted_seen.extend(server.quorium.evicted());
        }
    }
    network.heal();
    for id in &evicted_seen {
        let outcome = servers[0]
            .quorium
            .write(Command::Register { member: member(id) })
            .await;
        if let Ok(outcome) = outcome {
            assert_eq!(
                outcome,
                Outcome::Evicted,
                "seed {seed}: evicted {id} registered again"
            );
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let views: Vec<Vec<String>> = servers
            .iter()
            .map(|s| s.quorium.members().into_iter().map(|m| m.node_id).collect())
            .collect();
        if views.windows(2).all(|pair| pair[0] == pair[1]) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "seed {seed}: servers never converged: {views:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    history.assert_safe(seed);
    println!(
        "seed {seed}: {} terms, {} evicted members",
        history.leaders_by_term.len(),
        evicted_seen.len()
    );
    for keeper in keepers.values() {
        keeper.abort();
    }
    for server in servers {
        server.serving.abort();
        server.quorium.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn random_partitions_keep_the_cluster_safe_and_convergent() {
    let seeds: u64 = std::env::var("QUORIUM_PARTITION_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2);
    for seed in 1..=seeds {
        run_seed(seed).await;
    }
}
