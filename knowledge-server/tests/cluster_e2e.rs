use std::net::TcpListener as StdListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use harvest_db::test_support::TestDb;
use serde_json::{json, Value};
use tokio::sync::mpsc;

const SECRET: &str = "e2e-cluster-secret";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Postgres,
    Quorium,
}

impl Backend {
    fn smallest_cluster(self, wanted: usize) -> usize {
        match self {
            Backend::Postgres => wanted,
            Backend::Quorium => wanted.max(3),
        }
    }
}

enum Bootstrap {
    Init,
    Join(String),
}

impl Bootstrap {
    fn settings(&self, data_dir: &std::path::Path) -> String {
        let (mode, token) = match self {
            Bootstrap::Init => ("init", String::new()),
            Bootstrap::Join(token) => ("join", format!("join_token = \"{token}\"\n")),
        };
        format!(
            "backend = \"quorium\"\ndata_dir = \"{}\"\nbootstrap = \"{mode}\"\nelection_timeout_ms = 300\n{token}",
            data_dir.display()
        )
    }
}
const JWT_SECRET: &str = "e2e-jwt-secret-that-is-long-enough-for-hs256";

struct MockLlm {
    chunk_delay_ms: AtomicU64,
    chunks: AtomicUsize,
    requests: AtomicUsize,
}

async fn chat_completions(State(mock): State<Arc<MockLlm>>, Json(body): Json<Value>) -> Response {
    mock.requests.fetch_add(1, Ordering::SeqCst);
    let streaming = body["stream"].as_bool().unwrap_or(false);
    let chunks = mock.chunks.load(Ordering::SeqCst);
    let delay = Duration::from_millis(mock.chunk_delay_ms.load(Ordering::SeqCst));
    if !streaming {
        return Json(json!({
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "mock title" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 1, "completion_tokens": 1 },
        })).into_response();
    }
    let (tx, rx) = mpsc::channel::<Result<String, std::convert::Infallible>>(8);
    tokio::spawn(async move {
        for i in 0..chunks {
            let chunk = json!({ "choices": [{ "index": 0, "delta": { "content": format!("chunk{i} ") } }] });
            if tx.send(Ok(format!("data: {chunk}\n\n"))).await.is_err() {
                return;
            }
            tokio::time::sleep(delay).await;
        }
        let end = json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } });
        let _ = tx.send(Ok(format!("data: {end}\n\n"))).await;
        let _ = tx.send(Ok("data: [DONE]\n\n".to_string())).await;
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(axum::body::Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        ))
        .unwrap()
}

async fn start_mock_llm() -> (String, Arc<MockLlm>) {
    let mock = Arc::new(MockLlm {
        chunk_delay_ms: AtomicU64::new(5),
        chunks: AtomicUsize::new(5),
        requests: AtomicUsize::new(0),
    });
    let router = Router::new()
        .route("/chat/completions", post(chat_completions))
        .route(
            "/models",
            get(|| async { Json(json!({ "data": [{ "id": "mock" }] })) }),
        )
        .with_state(Arc::clone(&mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn free_port() -> u16 {
    StdListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Node {
    name: String,
    child: Option<Child>,
    base: String,
    internal: String,
    config_path: PathBuf,
    log_path: PathBuf,
}

impl Node {
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn internal_url(&self, path: &str) -> String {
        format!("{}{}", self.internal, path)
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn signal(&self, signal: &str) {
        if let Some(child) = &self.child {
            let _ = Command::new("kill")
                .args([signal, &child.id().to_string()])
                .status();
        }
    }

    fn pause(&self) {
        self.signal("-STOP");
    }

    fn resume(&self) {
        self.signal("-CONT");
    }

    fn terminate(&mut self) {
        if let Some(child) = &self.child {
            let _ = Command::new("kill")
                .args(["-TERM", &child.id().to_string()])
                .status();
        }
    }

    fn wait_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(child) = self.child.as_mut() {
                if let Ok(Some(_)) = child.try_wait() {
                    self.child = None;
                    return true;
                }
            } else {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    fn rewrite_config(&self, edit: impl Fn(&str) -> String) {
        let config = std::fs::read_to_string(&self.config_path).unwrap();
        std::fs::write(&self.config_path, edit(&config)).unwrap();
    }

    fn start_again(&mut self) {
        self.child = Some(spawn_server(&self.config_path, &self.log_path, &[]));
    }

    fn recover(&self) -> std::process::ExitStatus {
        spawn_server(&self.config_path, &self.log_path, &["recover-cluster"])
            .wait()
            .unwrap()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_file(&self.config_path);
    }
}

fn start_node(
    name: &str,
    db_url: &str,
    llm_url: &str,
    dir: &std::path::Path,
    bootstrap: Option<Bootstrap>,
) -> Node {
    let port = free_port();
    let internal_port = free_port();
    let quorium = bootstrap
        .map(|b| b.settings(&dir.join(format!("{name}-quorium"))))
        .unwrap_or_default();
    let config = format!(
        r#"
[server]
host = "127.0.0.1"
port = {port}

[database]
url = "{db_url}"
pool_size = 6

[cluster]
node_name = "{name}"
internal_listen = "127.0.0.1:{internal_port}"
shared_secret = "{SECRET}"
heartbeat_interval_ms = 200
node_timeout_ms = 1500
bus_batch_window_ms = 10
drain_grace_secs = 1
drain_timeout_secs = 5
{quorium}

[auth]
jwt_secret = "{JWT_SECRET}"
allow_local_login = true

[agent]
max_iterations = 3

[[llm]]
provider = "openai-compatible"
base_url = "{llm_url}"
api_key  = "mock-key"
model    = "mock"
id       = "mock"
max_retries = 0
"#
    );
    let config_path = dir.join(format!("{name}.toml"));
    let log_path = dir.join(format!("{name}.log"));
    std::fs::write(&config_path, config).unwrap();
    let child = spawn_server(&config_path, &log_path, &[]);
    Node {
        name: name.to_string(),
        child: Some(child),
        base: format!("http://127.0.0.1:{port}"),
        internal: format!("http://127.0.0.1:{internal_port}"),
        config_path,
        log_path,
    }
}

fn spawn_server(config_path: &std::path::Path, log_path: &std::path::Path, args: &[&str]) -> Child {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .unwrap();
    Command::new(env!("CARGO_BIN_EXE_knowledge-server"))
        .arg("--config")
        .arg(config_path)
        .args(args)
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("starting knowledge-server")
}

fn node_name(index: usize) -> String {
    format!("node-{}", (b'a' + index as u8) as char)
}

async fn join_token(http: &reqwest::Client, seed: &Node) -> String {
    let response: Value = http
        .post(seed.internal_url("/internal/cluster/tokens"))
        .header("x-harvest-cluster-secret", SECRET)
        .json(&json!({ "ttl_secs": 600 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    response["token"].as_str().expect("join token").to_string()
}

fn expected_voters(servers: usize) -> usize {
    let odd = if servers % 2 == 0 {
        servers - 1
    } else {
        servers
    };
    odd.clamp(1, 5)
}

async fn wait_for_voters(http: &reqwest::Client, node: &Node, voters: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status = cluster_status(http, node).await;
        let promoted = status["quorium"]["servers"]
            .as_array()
            .map_or(0, |servers| {
                servers.iter().filter(|s| s["voter"] == true).count()
            });
        if promoted >= voters {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "only {promoted} of {voters} voters after 30s: {status}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn cluster_status(http: &reqwest::Client, node: &Node) -> Value {
    http.get(node.internal_url("/internal/cluster/status"))
        .header("x-harvest-cluster-secret", SECRET)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn wait_ready(http: &reqwest::Client, node: &Node) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Ok(response) = http.get(node.url("/health/ready")).send().await {
            if response.status() == 200 {
                return;
            }
        }
        if Instant::now() > deadline {
            panic!("node {} never became ready:\n{}", node.name, node.log());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

static ONE_CLUSTER_AT_A_TIME: std::sync::LazyLock<Arc<tokio::sync::Mutex<()>>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Mutex::new(())));

struct Cluster {
    db_url: String,
    nodes: Vec<Node>,
    http: reqwest::Client,
    token: String,
    project_id: String,
    mock: Arc<MockLlm>,
    llm_url: String,
    _dir: tempfile::TempDir,
    _db: TestDb,
    _exclusive: tokio::sync::OwnedMutexGuard<()>,
}

impl Cluster {
    async fn start(backend: Backend, wanted: usize) -> Self {
        let exclusive = Arc::clone(&ONE_CLUSTER_AT_A_TIME).lock_owned().await;
        let count = backend.smallest_cluster(wanted);
        let db = TestDb::new().await;
        let db_url = db.url.clone();
        let (llm_url, mock) = start_mock_llm().await;
        let dir = tempfile::tempdir().unwrap();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap();
        let nodes = match backend {
            Backend::Postgres => {
                let nodes: Vec<Node> = (0..count)
                    .map(|i| start_node(&node_name(i), &db_url, &llm_url, dir.path(), None))
                    .collect();
                for node in &nodes {
                    wait_ready(&http, node).await;
                }
                nodes
            }
            Backend::Quorium => {
                let seed = start_node(
                    &node_name(0),
                    &db_url,
                    &llm_url,
                    dir.path(),
                    Some(Bootstrap::Init),
                );
                wait_ready(&http, &seed).await;
                let mut nodes = vec![seed];
                for i in 1..count {
                    let token = join_token(&http, &nodes[0]).await;
                    let node = start_node(
                        &node_name(i),
                        &db_url,
                        &llm_url,
                        dir.path(),
                        Some(Bootstrap::Join(token)),
                    );
                    wait_ready(&http, &node).await;
                    nodes.push(node);
                }
                wait_for_voters(&http, &nodes[0], expected_voters(count)).await;
                nodes
            }
        };

        let response = http.post(nodes[0].url("/auth/register"))
            .json(&json!({ "email": "admin@example.com", "name": "Admin", "password": "correct-horse" }))
            .send().await.unwrap();
        assert_eq!(response.status(), 200, "register failed");
        let cookie = response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let token = cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("token=")
            .to_string();

        let group: Value = http
            .post(nodes[1 % count].url("/admin/groups"))
            .bearer_auth(&token)
            .json(&json!({ "name": "team", "description": "" }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let group_id = group["id"].as_str().expect("group id").to_string();
        let project: Value = http
            .post(nodes[2 % count].url("/projects"))
            .bearer_auth(&token)
            .json(&json!({ "name": "ha", "group_id": group_id }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let project_id = project["id"].as_str().expect("project id").to_string();

        Self {
            _db: db,
            db_url,
            nodes,
            http,
            token,
            project_id,
            mock,
            llm_url,
            _dir: dir,
            _exclusive: exclusive,
        }
    }

    async fn new_conversation(&self, node: usize) -> String {
        let conv: Value = self
            .http
            .post(self.nodes[node].url(&format!("/projects/{}/conversations", self.project_id)))
            .bearer_auth(&self.token)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        conv["id"]
            .as_str()
            .unwrap_or_else(|| panic!("creating a conversation failed: {conv}"))
            .to_string()
    }

    async fn send(&self, node: usize, conv: &str, query: &str) -> reqwest::StatusCode {
        self.http
            .post(self.nodes[node].url(&format!("/projects/{}/query/stream", self.project_id)))
            .bearer_auth(&self.token)
            .json(&json!({ "query": query, "conversation_id": conv }))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn events(&self, node: usize, conv: Option<&str>) -> mpsc::UnboundedReceiver<Value> {
        let path = match conv {
            Some(c) => format!("/projects/{}/events?conv={c}", self.project_id),
            None => format!("/projects/{}/events", self.project_id),
        };
        sse(&self.http, &self.nodes[node].url(&path), &self.token).await
    }

    async fn db(&self) -> harvest_db::Db {
        harvest_db::Db::connect_without_migrating(&self.db_url).unwrap()
    }
}

async fn sse(http: &reqwest::Client, url: &str, token: &str) -> mpsc::UnboundedReceiver<Value> {
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "subscribing to {url}");
    let _ = http;
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(end) = buffer.find("\n\n") {
                let block: String = buffer.drain(..end + 2).collect();
                for line in block.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        if let Ok(value) = serde_json::from_str::<Value>(data.trim()) {
                            if tx.send(value).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
    });
    rx
}

async fn wait_for(
    rx: &mut mpsc::UnboundedReceiver<Value>,
    timeout: Duration,
    predicate: impl Fn(&Value) -> bool,
) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut seen = Vec::new();
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(event)) => {
                let done = predicate(&event);
                seen.push(event);
                if done {
                    return seen;
                }
            }
            Ok(None) => panic!("event stream closed; saw {seen:?}"),
            Err(_) => panic!("timed out waiting for event; saw {seen:?}"),
        }
    }
}

fn text_of(events: &[Value], conv: &str) -> String {
    events
        .iter()
        .filter(|e| e["conv_id"] == conv && e["type"] == "text_delta")
        .map(|e| e["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn expected_answer(chunks: usize) -> String {
    (0..chunks).map(|i| format!("chunk{i} ")).collect()
}

async fn three_nodes_serve_one_cluster(backend: Backend) {
    let cluster = Cluster::start(backend, 3).await;
    let nodes: Value = cluster
        .http
        .get(cluster.nodes[0].url("/version"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(nodes["schema_version"], harvest_db::LATEST_SCHEMA_VERSION);
    for node in &cluster.nodes {
        let status = cluster_status(&cluster.http, node).await;
        assert_eq!(status["members"].as_array().unwrap().len(), 3, "{status}");
        assert_eq!(status["membership"]["quorum"], true);
    }
}

async fn a_turn_started_on_one_node_streams_to_watchers_on_every_node(backend: Backend) {
    let cluster = Cluster::start(backend, 3).await;
    cluster.mock.chunks.store(8, Ordering::SeqCst);
    let conv = cluster.new_conversation(1).await;
    let mut on_b = cluster.events(1, Some(&conv)).await;
    let mut on_c = cluster.events(2, Some(&conv)).await;
    wait_for(&mut on_b, Duration::from_secs(10), |e| {
        e["type"] == "presence"
    })
    .await;
    wait_for(&mut on_c, Duration::from_secs(10), |e| {
        e["type"] == "presence"
    })
    .await;

    assert_eq!(cluster.send(0, &conv, "hello cluster").await, 200);

    for rx in [&mut on_b, &mut on_c] {
        let events = wait_for(rx, Duration::from_secs(30), |e| {
            e["type"] == "done" && e["conv_id"] == conv
        })
        .await;
        assert!(events
            .iter()
            .any(|e| e["type"] == "user_message" && e["query"] == "hello cluster"));
        assert!(events
            .iter()
            .any(|e| e["type"] == "lock" && e["conv_id"] == conv));
        assert_eq!(text_of(&events, &conv), expected_answer(8));
        let seqs: Vec<u64> = events
            .iter()
            .filter(|e| e["conv_id"] == conv)
            .filter_map(|e| e["seq"].as_u64())
            .collect();
        assert!(
            seqs.windows(2).all(|w| w[0] < w[1]),
            "events arrive in order: {seqs:?}"
        );
        wait_for(rx, Duration::from_secs(10), |e| {
            e["type"] == "unlock" && e["conv_id"] == conv
        })
        .await;
    }

    let saved: Value = cluster
        .http
        .get(cluster.nodes[2].url(&format!(
            "/projects/{}/conversations/{conv}",
            cluster.project_id
        )))
        .bearer_auth(&cluster.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = saved["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(
        messages[1]["text"].as_str().unwrap().trim(),
        expected_answer(8).trim()
    );
}

async fn a_conversation_lock_is_enforced_across_nodes(backend: Backend) {
    let cluster = Cluster::start(backend, 3).await;
    cluster.mock.chunks.store(30, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(100, Ordering::SeqCst);
    let conv = cluster.new_conversation(0).await;
    let mut watch = cluster.events(2, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "first").await, 200);
    wait_for(&mut watch, Duration::from_secs(10), |e| {
        e["type"] == "text_delta"
    })
    .await;
    assert_eq!(
        cluster.send(1, &conv, "second").await,
        409,
        "another node must see the lock"
    );
    assert_eq!(cluster.send(2, &conv, "third").await, 409);

    wait_for(&mut watch, Duration::from_secs(30), |e| {
        e["type"] == "unlock" && e["conv_id"] == conv
    })
    .await;
    assert_eq!(
        cluster.send(1, &conv, "after unlock").await,
        200,
        "the lock is released cluster-wide"
    );
}

async fn a_watcher_joining_mid_turn_on_another_node_catches_up_without_gaps_or_duplicates(
    backend: Backend,
) {
    let cluster = Cluster::start(backend, 3).await;
    cluster.mock.chunks.store(40, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(50, Ordering::SeqCst);
    let conv = cluster.new_conversation(0).await;
    let mut early = cluster.events(0, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "long answer").await, 200);
    wait_for(&mut early, Duration::from_secs(10), |e| {
        e["type"] == "text_delta" && e["text"] == "chunk10 "
    })
    .await;

    let mut late = cluster.events(2, Some(&conv)).await;
    let events = wait_for(&mut late, Duration::from_secs(30), |e| {
        e["type"] == "done" && e["conv_id"] == conv
    })
    .await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "user_message" && e["query"] == "long answer"),
        "catch-up replays the question"
    );
    assert_eq!(
        text_of(&events, &conv),
        expected_answer(40),
        "catch-up plus live events form the exact answer"
    );
}

async fn presence_is_shared_across_nodes(backend: Backend) {
    let cluster = Cluster::start(backend, 2).await;
    let conv = cluster.new_conversation(0).await;
    let mut on_a = cluster.events(0, Some(&conv)).await;
    wait_for(&mut on_a, Duration::from_secs(10), |e| {
        e["type"] == "presence"
    })
    .await;
    let mut on_b = cluster.events(1, None).await;
    let presence = wait_for(&mut on_b, Duration::from_secs(10), |e| {
        e["type"] == "presence"
    })
    .await;
    let users = presence.last().unwrap()["users"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(users.len(), 1, "presence lists each user once: {users:?}");
    wait_for(&mut on_a, Duration::from_secs(10), |e| {
        e["type"] == "user_join"
    })
    .await;
    let db = cluster.db().await;
    let rows = db
        .query("SELECT DISTINCT node_id FROM project_presence", json!({}))
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "each node records its own watchers in shared presence"
    );
}

async fn a_crashed_node_aborts_its_turn_and_releases_the_lock(backend: Backend) {
    let mut cluster = Cluster::start(backend, 3).await;
    cluster.mock.chunks.store(200, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(100, Ordering::SeqCst);
    let conv = cluster.new_conversation(1).await;
    let mut watch = cluster.events(2, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "doomed").await, 200);
    wait_for(&mut watch, Duration::from_secs(10), |e| {
        e["type"] == "text_delta"
    })
    .await;
    let killed_at = Instant::now();
    cluster.nodes[0].kill();

    wait_for(&mut watch, Duration::from_secs(15), |e| {
        e["type"] == "turn_aborted" && e["conv_id"] == conv
    })
    .await;
    wait_for(&mut watch, Duration::from_secs(5), |e| {
        e["type"] == "unlock" && e["conv_id"] == conv
    })
    .await;
    let detection = killed_at.elapsed();
    assert!(
        detection < Duration::from_secs(10),
        "failure detected in {detection:?}"
    );

    cluster.mock.chunks.store(3, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(5, Ordering::SeqCst);
    assert_eq!(cluster.send(1, &conv, "retry on a survivor").await, 200);
    wait_for(&mut watch, Duration::from_secs(30), |e| {
        e["type"] == "done" && e["conv_id"] == conv
    })
    .await;
}

async fn a_drained_node_fails_readiness_refuses_turns_and_exits_cleanly(backend: Backend) {
    let mut cluster = Cluster::start(backend, 2).await;
    let conv = cluster.new_conversation(0).await;
    let mut watch = cluster.events(1, Some(&conv)).await;
    wait_for(&mut watch, Duration::from_secs(10), |e| {
        e["type"] == "presence"
    })
    .await;

    cluster.nodes[0].terminate();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = cluster
            .http
            .get(cluster.nodes[0].url("/health/ready"))
            .send()
            .await
            .map(|r| r.status().as_u16())
            .unwrap_or(0);
        if status == 503 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "readiness never reported draining"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(cluster.send(0, &conv, "too late").await, 503);
    assert!(
        cluster.nodes[0].wait_exit(Duration::from_secs(20)),
        "drained node must exit:\n{}",
        cluster.nodes[0].log()
    );

    let status = cluster_status(&cluster.http, &cluster.nodes[1]).await;
    assert!(
        status["members"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| !m["node_id"].as_str().unwrap().starts_with("node-a-")),
        "a drained node deregisters itself: {status}"
    );

    assert_eq!(cluster.send(1, &conv, "served by the survivor").await, 200);
    wait_for(&mut watch, Duration::from_secs(30), |e| {
        e["type"] == "done" && e["conv_id"] == conv
    })
    .await;
}

async fn connect_fake_agent(
    cluster: &Cluster,
    node: usize,
) -> (String, String, mpsc::UnboundedReceiver<Value>) {
    let db = cluster.db().await;
    let rows = db
        .query(
            "SELECT install_token FROM projects WHERE id = $pid",
            json!({ "pid": cluster.project_id }),
        )
        .await
        .unwrap();
    let install_token = rows[0]["install_token"].as_str().unwrap().to_string();
    let mut events = sse(
        &cluster.http,
        &cluster.nodes[node].url("/agent/events?hostname=fake-host"),
        &install_token,
    )
    .await;
    let first = wait_for(&mut events, Duration::from_secs(10), |e| {
        e["type"] == "registered"
    })
    .await;
    let agent_token = first.last().unwrap()["agent_token"]
        .as_str()
        .unwrap()
        .to_string();
    let rows = db
        .query(
            "SELECT id FROM machines WHERE project_id = $pid",
            json!({ "pid": cluster.project_id }),
        )
        .await
        .unwrap();
    let agent_id = rows[0]["id"].as_str().unwrap().to_string();
    (agent_id, agent_token, events)
}

async fn an_agent_connected_to_one_node_runs_commands_requested_on_another(backend: Backend) {
    let cluster = Cluster::start(backend, 3).await;
    let (agent_id, agent_token, mut agent_events) = connect_fake_agent(&cluster, 0).await;

    let agents: Value = cluster
        .http
        .get(cluster.nodes[1].url(&format!("/projects/{}/agents", cluster.project_id)))
        .bearer_auth(&cluster.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        agents[0]["online"], true,
        "node B sees the agent connected to node A as online"
    );

    let http = cluster.http.clone();
    let result_url = cluster.nodes[2].url("/agent/results");
    let responder = tokio::spawn(async move {
        let events = wait_for(&mut agent_events, Duration::from_secs(20), |e| {
            e["type"] == "execute"
        })
        .await;
        let execute = events.last().unwrap().clone();
        let status = http.post(result_url).bearer_auth(&agent_token)
            .json(&json!({ "request_id": execute["request_id"], "stdout": format!("ran: {}", execute["command"].as_str().unwrap()), "stderr": "", "exit_code": 0 }))
            .send().await.unwrap().status();
        assert_eq!(status, 200, "a result posted to a third node is accepted");
    });

    let response: Value = cluster
        .http
        .post(cluster.nodes[1].url(&format!(
            "/projects/{}/agents/{agent_id}/execute",
            cluster.project_id
        )))
        .bearer_auth(&cluster.token)
        .json(&json!({ "command": "uname -a", "timeout_secs": 15 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    responder.await.unwrap();
    assert_eq!(response["stdout"], "ran: uname -a");
    assert_eq!(response["exit_code"], 0);
}

async fn an_agent_reconnects_to_a_survivor_when_its_node_crashes(backend: Backend) {
    let mut cluster = Cluster::start(backend, 2).await;
    let (agent_id, _token, _events) = connect_fake_agent(&cluster, 0).await;
    cluster.nodes[0].kill();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let agents: Value = cluster
        .http
        .get(cluster.nodes[1].url(&format!("/projects/{}/agents", cluster.project_id)))
        .bearer_auth(&cluster.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        agents[0]["online"], false,
        "an agent on a dead node is reported offline"
    );

    let db = cluster.db().await;
    let rows = db
        .query(
            "SELECT agent_token_hash FROM machines WHERE id = $id",
            json!({ "id": agent_id }),
        )
        .await
        .unwrap();
    assert!(rows[0]["agent_token_hash"].as_str().is_some());
    let (_same_id, _token, _events) = {
        let (id, token, events) = connect_fake_agent_with_token(&cluster, 1, &_token).await;
        (id, token, events)
    };
    let agents: Value = cluster
        .http
        .get(cluster.nodes[1].url(&format!("/projects/{}/agents", cluster.project_id)))
        .bearer_auth(&cluster.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        agents[0]["online"], true,
        "the agent is back online through the surviving node"
    );
}

async fn connect_fake_agent_with_token(
    cluster: &Cluster,
    node: usize,
    token: &str,
) -> (String, String, mpsc::UnboundedReceiver<Value>) {
    let mut events = sse(
        &cluster.http,
        &cluster.nodes[node].url("/agent/events?hostname=fake-host"),
        token,
    )
    .await;
    wait_for(&mut events, Duration::from_secs(10), |e| {
        e["type"] == "hello_ack"
    })
    .await;
    (String::new(), token.to_string(), events)
}

async fn wait_ready_again(http: &reqwest::Client, node: &Node, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(response) = http.get(node.url("/health/ready")).send().await {
            if response.status() == 200 {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

async fn a_frozen_node_is_fenced_and_cannot_overwrite_the_takeover(backend: Backend) {
    let mut cluster = Cluster::start(backend, 3).await;
    cluster.mock.chunks.store(150, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(100, Ordering::SeqCst);
    let conv = cluster.new_conversation(1).await;
    let mut watch = cluster.events(2, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "frozen").await, 200);
    wait_for(&mut watch, Duration::from_secs(10), |e| {
        e["type"] == "text_delta"
    })
    .await;
    cluster.nodes[0].pause();
    wait_for(&mut watch, Duration::from_secs(15), |e| {
        e["type"] == "turn_aborted" && e["conv_id"] == conv
    })
    .await;

    cluster.mock.chunks.store(3, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(5, Ordering::SeqCst);
    wait_for(&mut watch, Duration::from_secs(5), |e| {
        e["type"] == "unlock" && e["conv_id"] == conv
    })
    .await;
    assert_eq!(cluster.send(1, &conv, "taken over").await, 200);
    wait_for(&mut watch, Duration::from_secs(30), |e| {
        e["type"] == "done" && e["conv_id"] == conv
    })
    .await;

    cluster.nodes[0].resume();
    match backend {
        Backend::Quorium => assert!(
            cluster.nodes[0].wait_exit(Duration::from_secs(20)),
            "an evicted node must exit:\n{}",
            cluster.nodes[0].log()
        ),
        Backend::Postgres => assert!(
            wait_ready_again(&cluster.http, &cluster.nodes[0], Duration::from_secs(20)).await,
            "a fenced node rejoins once its heartbeat is confirmed:\n{}",
            cluster.nodes[0].log()
        ),
    }
    tokio::time::sleep(Duration::from_secs(3)).await;

    let saved: Value = cluster
        .http
        .get(cluster.nodes[1].url(&format!(
            "/projects/{}/conversations/{conv}",
            cluster.project_id
        )))
        .bearer_auth(&cluster.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let messages = saved["messages"].as_array().unwrap();
    assert_eq!(
        messages.len(),
        4,
        "the interrupted turn and the takeover are each saved once: {messages:?}"
    );
    assert_eq!(messages[0]["text"], "frozen");
    assert_eq!(messages[1]["status"], "interrupted");
    assert_eq!(messages[2]["text"], "taken over");
    assert_eq!(
        messages[3]["text"].as_str().unwrap().trim(),
        expected_answer(3).trim()
    );
}

struct LoadProfile {
    backend: Backend,
    turns: usize,
    concurrency: usize,
    chunks: usize,
    chunk_delay_ms: u64,
}

impl LoadProfile {
    fn from_env() -> Option<Self> {
        let number = |name: &str, default: usize| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        };
        let turns = number("HARVEST_LOAD_TURNS", 0);
        if turns == 0 {
            return None;
        }
        let backend = match std::env::var("HARVEST_LOAD_BACKEND").as_deref() {
            Ok("quorium") => Backend::Quorium,
            _ => Backend::Postgres,
        };
        Some(Self {
            backend,
            turns,
            concurrency: number("HARVEST_LOAD_CONCURRENCY", 8),
            chunks: number("HARVEST_LOAD_CHUNKS", 20),
            chunk_delay_ms: number("HARVEST_LOAD_CHUNK_DELAY_MS", 20) as u64,
        })
    }
}

#[derive(Default)]
struct TurnTimings {
    first_token: Vec<Duration>,
    completed: Vec<Duration>,
    rejected: usize,
    failed: usize,
}

fn percentile(samples: &mut [Duration], p: f64) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort();
    let rank = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[rank].as_millis()
}

async fn timed_turn(cluster: &Cluster, index: usize) -> Result<(Duration, Duration), u16> {
    let nodes = cluster.nodes.len();
    let conv = cluster.new_conversation(index % nodes).await;
    let mut watch = cluster.events((index + 1) % nodes, Some(&conv)).await;
    let started = Instant::now();
    let status = cluster.send(index % nodes, &conv, "load").await;
    if status != 200 {
        return Err(status.as_u16());
    }
    let mut first_token = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let event = tokio::time::timeout_at(deadline, watch.recv())
            .await
            .map_err(|_| 504u16)?
            .ok_or(502u16)?;
        if event["conv_id"] != conv {
            continue;
        }
        if event["type"] == "text_delta" && first_token.is_none() {
            first_token = Some(started.elapsed());
        }
        if event["type"] == "done" {
            return Ok((first_token.unwrap_or_default(), started.elapsed()));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "load profile: set HARVEST_LOAD_TURNS"]
async fn load_profile_reports_latency_percentiles() {
    let Some(profile) = LoadProfile::from_env() else {
        return;
    };
    let cluster = Arc::new(Cluster::start(profile.backend, 3).await);
    cluster.mock.chunks.store(profile.chunks, Ordering::SeqCst);
    cluster
        .mock
        .chunk_delay_ms
        .store(profile.chunk_delay_ms, Ordering::SeqCst);
    let permits = Arc::new(tokio::sync::Semaphore::new(profile.concurrency));
    let mut runs = tokio::task::JoinSet::new();
    let wall = Instant::now();
    for index in 0..profile.turns {
        let permit = Arc::clone(&permits).acquire_owned().await.unwrap();
        let cluster = Arc::clone(&cluster);
        runs.spawn(async move {
            let outcome = timed_turn(&cluster, index).await;
            drop(permit);
            outcome
        });
    }
    let mut timings = TurnTimings::default();
    while let Some(outcome) = runs.join_next().await {
        match outcome.unwrap() {
            Ok((first_token, completed)) => {
                timings.first_token.push(first_token);
                timings.completed.push(completed);
            }
            Err(429) | Err(503) => timings.rejected += 1,
            Err(_) => timings.failed += 1,
        }
    }
    let wall = wall.elapsed();
    let report = json!({
        "backend": format!("{:?}", profile.backend).to_lowercase(),
        "turns": profile.turns,
        "concurrency": profile.concurrency,
        "mock_stream_ms": profile.chunks as u64 * profile.chunk_delay_ms,
        "completed": timings.completed.len(),
        "rejected": timings.rejected,
        "failed": timings.failed,
        "turns_per_second": timings.completed.len() as f64 / wall.as_secs_f64(),
        "first_token_ms": {
            "p50": percentile(&mut timings.first_token, 0.50),
            "p95": percentile(&mut timings.first_token, 0.95),
            "p99": percentile(&mut timings.first_token, 0.99),
        },
        "completion_ms": {
            "p50": percentile(&mut timings.completed, 0.50),
            "p95": percentile(&mut timings.completed, 0.95),
            "p99": percentile(&mut timings.completed, 0.99),
        },
    });
    let path = std::env::var("HARVEST_LOAD_REPORT").unwrap_or_else(|_| "harvest-load.json".into());
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    println!("{report}");
    assert_eq!(timings.failed, 0, "{report}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts server processes"]
async fn a_cluster_that_lost_quorum_is_recovered_from_its_survivor() {
    let mut cluster = Cluster::start(Backend::Quorium, 3).await;
    cluster.nodes[1].kill();
    cluster.nodes[2].kill();
    let deadline = Instant::now() + Duration::from_secs(20);
    while cluster
        .http
        .get(cluster.nodes[0].url("/health/ready"))
        .send()
        .await
        .is_ok_and(|r| r.status() == 200)
    {
        assert!(
            Instant::now() < deadline,
            "a survivor without quorum must fail readiness"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    cluster.nodes[0].kill();
    assert!(
        cluster.nodes[0].recover().success(),
        "{}",
        cluster.nodes[0].log()
    );
    cluster.nodes[0].start_again();
    wait_ready(&cluster.http, &cluster.nodes[0]).await;

    for index in 1..3 {
        let token = join_token(&cluster.http, &cluster.nodes[0]).await;
        let replacement = start_node(
            &format!("node-r{index}"),
            &cluster.db_url,
            &cluster.llm_url,
            cluster._dir.path(),
            Some(Bootstrap::Join(token)),
        );
        wait_ready(&cluster.http, &replacement).await;
        cluster.nodes[index] = replacement;
    }
    wait_for_voters(&cluster.http, &cluster.nodes[0], 3).await;

    let conv = cluster.new_conversation(1).await;
    let mut watch = cluster.events(2, Some(&conv)).await;
    assert_eq!(cluster.send(1, &conv, "after recovery").await, 200);
    wait_for(&mut watch, Duration::from_secs(30), |e| {
        e["type"] == "done" && e["conv_id"] == conv
    })
    .await;
}

fn secrets_line(primary: &str, accepted: &[&str]) -> String {
    let accepted: Vec<String> = accepted.iter().map(|s| format!("\"{s}\"")).collect();
    format!(
        "shared_secret = \"{primary}\"\naccepted_secrets = [{}]",
        accepted.join(", ")
    )
}

fn with_secrets(config: &str, primary: &str, accepted: &[&str]) -> String {
    config
        .lines()
        .filter(|line| !line.starts_with("accepted_secrets"))
        .map(|line| {
            if line.starts_with("shared_secret") {
                secrets_line(primary, accepted)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn restart_one_by_one(cluster: &mut Cluster, primary: &str, accepted: &[&str]) {
    for index in 0..cluster.nodes.len() {
        cluster.nodes[index].terminate();
        assert!(cluster.nodes[index].wait_exit(Duration::from_secs(30)));
        cluster.nodes[index].rewrite_config(|config| with_secrets(config, primary, accepted));
        cluster.nodes[index].start_again();
        wait_ready(&cluster.http, &cluster.nodes[index]).await;
    }
}

async fn streams_across_nodes(cluster: &Cluster, query: &str) {
    let conv = cluster.new_conversation(0).await;
    let mut watch = cluster.events(2, Some(&conv)).await;
    assert_eq!(cluster.send(1, &conv, query).await, 200);
    let events = wait_for(&mut watch, Duration::from_secs(30), |e| {
        e["type"] == "done" && e["conv_id"] == conv
    })
    .await;
    assert!(events.iter().any(|e| e["type"] == "text_delta"));
}

async fn the_cluster_secret_rotates_without_downtime(backend: Backend) {
    let mut cluster = Cluster::start(backend, 3).await;
    let next = "e2e-rotated-secret";
    restart_one_by_one(&mut cluster, SECRET, &[next]).await;
    streams_across_nodes(&cluster, "after staging").await;
    restart_one_by_one(&mut cluster, next, &[SECRET]).await;
    streams_across_nodes(&cluster, "after committing").await;
    restart_one_by_one(&mut cluster, next, &[]).await;
    streams_across_nodes(&cluster, "after finishing").await;
    let rejected = cluster
        .http
        .get(cluster.nodes[0].internal_url("/internal/cluster/status"))
        .header("x-harvest-cluster-secret", SECRET)
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(rejected, 401, "the retired secret is no longer accepted");
}

macro_rules! on_every_backend {
    ($($scenario:ident),* $(,)?) => {
        $(
            mod $scenario {
                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts server processes"]
                async fn postgres() {
                    super::$scenario(super::Backend::Postgres).await
                }

                #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
                #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts server processes"]
                async fn quorium() {
                    super::$scenario(super::Backend::Quorium).await
                }
            }
        )*
    };
}

on_every_backend!(
    the_cluster_secret_rotates_without_downtime,
    a_frozen_node_is_fenced_and_cannot_overwrite_the_takeover,
    three_nodes_serve_one_cluster,
    a_turn_started_on_one_node_streams_to_watchers_on_every_node,
    a_conversation_lock_is_enforced_across_nodes,
    a_watcher_joining_mid_turn_on_another_node_catches_up_without_gaps_or_duplicates,
    presence_is_shared_across_nodes,
    a_crashed_node_aborts_its_turn_and_releases_the_lock,
    a_drained_node_fails_readiness_refuses_turns_and_exits_cleanly,
    an_agent_connected_to_one_node_runs_commands_requested_on_another,
    an_agent_reconnects_to_a_survivor_when_its_node_crashes,
);
