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
        .body(axum::body::Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)))
        .unwrap()
}

async fn start_mock_llm() -> (String, Arc<MockLlm>) {
    let mock = Arc::new(MockLlm { chunk_delay_ms: AtomicU64::new(5), chunks: AtomicUsize::new(5), requests: AtomicUsize::new(0) });
    let router = Router::new()
        .route("/chat/completions", post(chat_completions))
        .route("/models", get(|| async { Json(json!({ "data": [{ "id": "mock" }] })) }))
        .with_state(Arc::clone(&mock));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{addr}"), mock)
}

fn free_port() -> u16 {
    StdListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

struct Node {
    name: String,
    child: Option<Child>,
    base: String,
    config_path: PathBuf,
    log_path: PathBuf,
}

impl Node {
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn terminate(&mut self) {
        if let Some(child) = &self.child {
            let _ = Command::new("kill").args(["-TERM", &child.id().to_string()]).status();
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

fn start_node(name: &str, db_url: &str, llm_url: &str, dir: &std::path::Path) -> Node {
    let port = free_port();
    let internal_port = free_port();
    let config = format!(r#"
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
"#);
    let config_path = dir.join(format!("{name}.toml"));
    let log_path = dir.join(format!("{name}.log"));
    std::fs::write(&config_path, config).unwrap();
    let log = std::fs::File::create(&log_path).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_knowledge-server"))
        .arg("--config")
        .arg(&config_path)
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("starting knowledge-server");
    Node { name: name.to_string(), child: Some(child), base: format!("http://127.0.0.1:{port}"), config_path, log_path }
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

struct Cluster {
    _db: TestDb,
    db_url: String,
    nodes: Vec<Node>,
    http: reqwest::Client,
    token: String,
    project_id: String,
    mock: Arc<MockLlm>,
    _dir: tempfile::TempDir,
}

impl Cluster {
    async fn start(count: usize) -> Self {
        let db = TestDb::new().await;
        let db_url = db.url.clone();
        let (llm_url, mock) = start_mock_llm().await;
        let dir = tempfile::tempdir().unwrap();
        let nodes: Vec<Node> = (0..count)
            .map(|i| start_node(&format!("node-{}", (b'a' + i as u8) as char), &db_url, &llm_url, dir.path()))
            .collect();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap();
        for node in &nodes {
            wait_ready(&http, node).await;
        }

        let response = http.post(nodes[0].url("/auth/register"))
            .json(&json!({ "email": "admin@example.com", "name": "Admin", "password": "correct-horse" }))
            .send().await.unwrap();
        assert_eq!(response.status(), 200, "register failed");
        let cookie = response.headers().get("set-cookie").unwrap().to_str().unwrap().to_string();
        let token = cookie.split(';').next().unwrap().trim_start_matches("token=").to_string();

        let group: Value = http.post(nodes[1 % count].url("/admin/groups")).bearer_auth(&token)
            .json(&json!({ "name": "team", "description": "" }))
            .send().await.unwrap().json().await.unwrap();
        let group_id = group["id"].as_str().expect("group id").to_string();
        let project: Value = http.post(nodes[2 % count].url("/projects")).bearer_auth(&token)
            .json(&json!({ "name": "ha", "group_id": group_id }))
            .send().await.unwrap().json().await.unwrap();
        let project_id = project["id"].as_str().expect("project id").to_string();

        Self { _db: db, db_url, nodes, http, token, project_id, mock, _dir: dir }
    }

    async fn new_conversation(&self, node: usize) -> String {
        let conv: Value = self.http.post(self.nodes[node].url(&format!("/projects/{}/conversations", self.project_id)))
            .bearer_auth(&self.token).json(&json!({})).send().await.unwrap().json().await.unwrap();
        conv["id"].as_str().unwrap().to_string()
    }

    async fn send(&self, node: usize, conv: &str, query: &str) -> reqwest::StatusCode {
        self.http.post(self.nodes[node].url(&format!("/projects/{}/query/stream", self.project_id)))
            .bearer_auth(&self.token)
            .json(&json!({ "query": query, "conversation_id": conv }))
            .send().await.unwrap().status()
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
    let response = reqwest::Client::new().get(url).bearer_auth(token).send().await.unwrap();
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

async fn wait_for(rx: &mut mpsc::UnboundedReceiver<Value>, timeout: Duration, predicate: impl Fn(&Value) -> bool) -> Vec<Value> {
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
    events.iter()
        .filter(|e| e["conv_id"] == conv && e["type"] == "text_delta")
        .map(|e| e["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn expected_answer(chunks: usize) -> String {
    (0..chunks).map(|i| format!("chunk{i} ")).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn three_nodes_serve_one_cluster() {
    let cluster = Cluster::start(3).await;
    let nodes: Value = cluster.http.get(cluster.nodes[0].url("/version")).send().await.unwrap().json().await.unwrap();
    assert_eq!(nodes["schema_version"], harvest_db::LATEST_SCHEMA_VERSION);
    let db = cluster.db().await;
    let alive = db.query("SELECT count(*) AS n FROM cluster_nodes WHERE heartbeat_at > now() - interval '2 seconds'", json!({})).await.unwrap();
    assert_eq!(alive[0]["n"], 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn a_turn_started_on_one_node_streams_to_watchers_on_every_node() {
    let cluster = Cluster::start(3).await;
    cluster.mock.chunks.store(8, Ordering::SeqCst);
    let conv = cluster.new_conversation(1).await;
    let mut on_b = cluster.events(1, Some(&conv)).await;
    let mut on_c = cluster.events(2, Some(&conv)).await;
    wait_for(&mut on_b, Duration::from_secs(10), |e| e["type"] == "presence").await;
    wait_for(&mut on_c, Duration::from_secs(10), |e| e["type"] == "presence").await;

    assert_eq!(cluster.send(0, &conv, "hello cluster").await, 200);

    for rx in [&mut on_b, &mut on_c] {
        let events = wait_for(rx, Duration::from_secs(30), |e| e["type"] == "done" && e["conv_id"] == conv).await;
        assert!(events.iter().any(|e| e["type"] == "user_message" && e["query"] == "hello cluster"));
        assert!(events.iter().any(|e| e["type"] == "lock" && e["conv_id"] == conv));
        assert_eq!(text_of(&events, &conv), expected_answer(8));
        let seqs: Vec<u64> = events.iter().filter(|e| e["conv_id"] == conv).filter_map(|e| e["seq"].as_u64()).collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]), "events arrive in order: {seqs:?}");
        wait_for(rx, Duration::from_secs(10), |e| e["type"] == "unlock" && e["conv_id"] == conv).await;
    }

    let saved: Value = cluster.http.get(cluster.nodes[2].url(&format!("/projects/{}/conversations/{conv}", cluster.project_id)))
        .bearer_auth(&cluster.token).send().await.unwrap().json().await.unwrap();
    let messages = saved["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["text"].as_str().unwrap().trim(), expected_answer(8).trim());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn a_conversation_lock_is_enforced_across_nodes() {
    let cluster = Cluster::start(3).await;
    cluster.mock.chunks.store(30, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(100, Ordering::SeqCst);
    let conv = cluster.new_conversation(0).await;
    let mut watch = cluster.events(2, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "first").await, 200);
    wait_for(&mut watch, Duration::from_secs(10), |e| e["type"] == "text_delta").await;
    assert_eq!(cluster.send(1, &conv, "second").await, 409, "another node must see the lock");
    assert_eq!(cluster.send(2, &conv, "third").await, 409);

    wait_for(&mut watch, Duration::from_secs(30), |e| e["type"] == "unlock" && e["conv_id"] == conv).await;
    assert_eq!(cluster.send(1, &conv, "after unlock").await, 200, "the lock is released cluster-wide");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn a_watcher_joining_mid_turn_on_another_node_catches_up_without_gaps_or_duplicates() {
    let cluster = Cluster::start(3).await;
    cluster.mock.chunks.store(40, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(50, Ordering::SeqCst);
    let conv = cluster.new_conversation(0).await;
    let mut early = cluster.events(0, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "long answer").await, 200);
    wait_for(&mut early, Duration::from_secs(10), |e| e["type"] == "text_delta" && e["text"] == "chunk10 ").await;

    let mut late = cluster.events(2, Some(&conv)).await;
    let events = wait_for(&mut late, Duration::from_secs(30), |e| e["type"] == "done" && e["conv_id"] == conv).await;
    assert!(events.iter().any(|e| e["type"] == "user_message" && e["query"] == "long answer"), "catch-up replays the question");
    assert_eq!(text_of(&events, &conv), expected_answer(40), "catch-up plus live events form the exact answer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn presence_is_shared_across_nodes() {
    let cluster = Cluster::start(2).await;
    let conv = cluster.new_conversation(0).await;
    let mut on_a = cluster.events(0, Some(&conv)).await;
    wait_for(&mut on_a, Duration::from_secs(10), |e| e["type"] == "presence").await;
    let mut on_b = cluster.events(1, None).await;
    let presence = wait_for(&mut on_b, Duration::from_secs(10), |e| e["type"] == "presence").await;
    let users = presence.last().unwrap()["users"].as_array().unwrap().clone();
    assert_eq!(users.len(), 1, "presence lists each user once: {users:?}");
    wait_for(&mut on_a, Duration::from_secs(10), |e| e["type"] == "user_join").await;
    let db = cluster.db().await;
    let rows = db.query("SELECT DISTINCT node_id FROM project_presence", json!({})).await.unwrap();
    assert_eq!(rows.len(), 2, "each node records its own watchers in shared presence");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn a_crashed_node_aborts_its_turn_and_releases_the_lock() {
    let mut cluster = Cluster::start(3).await;
    cluster.mock.chunks.store(200, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(100, Ordering::SeqCst);
    let conv = cluster.new_conversation(1).await;
    let mut watch = cluster.events(2, Some(&conv)).await;

    assert_eq!(cluster.send(0, &conv, "doomed").await, 200);
    wait_for(&mut watch, Duration::from_secs(10), |e| e["type"] == "text_delta").await;
    let killed_at = Instant::now();
    cluster.nodes[0].kill();

    wait_for(&mut watch, Duration::from_secs(15), |e| e["type"] == "turn_aborted" && e["conv_id"] == conv).await;
    wait_for(&mut watch, Duration::from_secs(5), |e| e["type"] == "unlock" && e["conv_id"] == conv).await;
    let detection = killed_at.elapsed();
    assert!(detection < Duration::from_secs(10), "failure detected in {detection:?}");

    cluster.mock.chunks.store(3, Ordering::SeqCst);
    cluster.mock.chunk_delay_ms.store(5, Ordering::SeqCst);
    assert_eq!(cluster.send(1, &conv, "retry on a survivor").await, 200);
    wait_for(&mut watch, Duration::from_secs(30), |e| e["type"] == "done" && e["conv_id"] == conv).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn a_drained_node_fails_readiness_refuses_turns_and_exits_cleanly() {
    let mut cluster = Cluster::start(2).await;
    let conv = cluster.new_conversation(0).await;
    let mut watch = cluster.events(1, Some(&conv)).await;
    wait_for(&mut watch, Duration::from_secs(10), |e| e["type"] == "presence").await;

    cluster.nodes[0].terminate();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = cluster.http.get(cluster.nodes[0].url("/health/ready")).send().await.map(|r| r.status().as_u16()).unwrap_or(0);
        if status == 503 {
            break;
        }
        assert!(Instant::now() < deadline, "readiness never reported draining");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(cluster.send(0, &conv, "too late").await, 503);
    assert!(cluster.nodes[0].wait_exit(Duration::from_secs(20)), "drained node must exit:\n{}", cluster.nodes[0].log());

    let db = cluster.db().await;
    let rows = db.query("SELECT count(*) AS n FROM cluster_nodes WHERE node_id LIKE 'node-a-%'", json!({})).await.unwrap();
    assert_eq!(rows[0]["n"], 0, "a drained node deregisters itself");

    assert_eq!(cluster.send(1, &conv, "served by the survivor").await, 200);
    wait_for(&mut watch, Duration::from_secs(30), |e| e["type"] == "done" && e["conv_id"] == conv).await;
}

async fn connect_fake_agent(cluster: &Cluster, node: usize) -> (String, String, mpsc::UnboundedReceiver<Value>) {
    let db = cluster.db().await;
    let rows = db.query("SELECT install_token FROM projects WHERE id = $pid", json!({ "pid": cluster.project_id })).await.unwrap();
    let install_token = rows[0]["install_token"].as_str().unwrap().to_string();
    let mut events = sse(&cluster.http, &cluster.nodes[node].url("/agent/events?hostname=fake-host"), &install_token).await;
    let first = wait_for(&mut events, Duration::from_secs(10), |e| e["type"] == "registered").await;
    let agent_token = first.last().unwrap()["agent_token"].as_str().unwrap().to_string();
    let rows = db.query("SELECT id FROM machines WHERE project_id = $pid", json!({ "pid": cluster.project_id })).await.unwrap();
    let agent_id = rows[0]["id"].as_str().unwrap().to_string();
    (agent_id, agent_token, events)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn an_agent_connected_to_one_node_runs_commands_requested_on_another() {
    let cluster = Cluster::start(3).await;
    let (agent_id, agent_token, mut agent_events) = connect_fake_agent(&cluster, 0).await;

    let agents: Value = cluster.http.get(cluster.nodes[1].url(&format!("/projects/{}/agents", cluster.project_id)))
        .bearer_auth(&cluster.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(agents[0]["online"], true, "node B sees the agent connected to node A as online");

    let http = cluster.http.clone();
    let result_url = cluster.nodes[2].url("/agent/results");
    let responder = tokio::spawn(async move {
        let events = wait_for(&mut agent_events, Duration::from_secs(20), |e| e["type"] == "execute").await;
        let execute = events.last().unwrap().clone();
        let status = http.post(result_url).bearer_auth(&agent_token)
            .json(&json!({ "request_id": execute["request_id"], "stdout": format!("ran: {}", execute["command"].as_str().unwrap()), "stderr": "", "exit_code": 0 }))
            .send().await.unwrap().status();
        assert_eq!(status, 200, "a result posted to a third node is accepted");
    });

    let response: Value = cluster.http.post(cluster.nodes[1].url(&format!("/projects/{}/agents/{agent_id}/execute", cluster.project_id)))
        .bearer_auth(&cluster.token)
        .json(&json!({ "command": "uname -a", "timeout_secs": 15 }))
        .send().await.unwrap().json().await.unwrap();
    responder.await.unwrap();
    assert_eq!(response["stdout"], "ran: uname -a");
    assert_eq!(response["exit_code"], 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL); starts three server processes"]
async fn an_agent_reconnects_to_a_survivor_when_its_node_crashes() {
    let mut cluster = Cluster::start(2).await;
    let (agent_id, _token, _events) = connect_fake_agent(&cluster, 0).await;
    cluster.nodes[0].kill();
    tokio::time::sleep(Duration::from_secs(3)).await;
    let agents: Value = cluster.http.get(cluster.nodes[1].url(&format!("/projects/{}/agents", cluster.project_id)))
        .bearer_auth(&cluster.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(agents[0]["online"], false, "an agent on a dead node is reported offline");

    let db = cluster.db().await;
    let rows = db.query("SELECT agent_token_hash FROM machines WHERE id = $id", json!({ "id": agent_id })).await.unwrap();
    assert!(rows[0]["agent_token_hash"].as_str().is_some());
    let (_same_id, _token, _events) = {
        let (id, token, events) = connect_fake_agent_with_token(&cluster, 1, &_token).await;
        (id, token, events)
    };
    let agents: Value = cluster.http.get(cluster.nodes[1].url(&format!("/projects/{}/agents", cluster.project_id)))
        .bearer_auth(&cluster.token).send().await.unwrap().json().await.unwrap();
    assert_eq!(agents[0]["online"], true, "the agent is back online through the surviving node");
}

async fn connect_fake_agent_with_token(cluster: &Cluster, node: usize, token: &str) -> (String, String, mpsc::UnboundedReceiver<Value>) {
    let mut events = sse(&cluster.http, &cluster.nodes[node].url("/agent/events?hostname=fake-host"), token).await;
    wait_for(&mut events, Duration::from_secs(10), |e| e["type"] == "hello_ack").await;
    (String::new(), token.to_string(), events)
}
