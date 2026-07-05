use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use knowledge_server::machines::{
    handlers::{generate_install_script, machines_router, MachineState},
    MachineRegistry,
};

async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
    resp.into_body().collect().await.unwrap().to_bytes().to_vec()
}

async fn body_text(resp: axum::response::Response) -> String {
    String::from_utf8_lossy(&body_bytes(resp).await).into_owned()
}

async fn body_json(resp: axum::response::Response) -> Value {
    serde_json::from_slice(&body_bytes(resp).await).unwrap()
}

fn make_state(registry: Arc<MachineRegistry>) -> Arc<MachineState> {
    Arc::new(MachineState {
        registry,
        neo4j:       None,
        binary_path: None,
        server_url:  "https://harvest.example.com".into(),
        lxd:         None,
    })
}

#[test]
fn install_script_contains_server_url() {
    let s = generate_install_script("https://harvest.example.com", "tok-abc");
    assert!(s.contains("https://harvest.example.com"), "missing server_url");
}

#[test]
fn install_script_contains_install_token() {
    let s = generate_install_script("https://harvest.example.com", "tok-abc");
    assert!(s.contains("tok-abc"), "missing install_token");
}

#[test]
fn install_script_does_not_write_project_id_to_config() {
    let s = generate_install_script("https://harvest.example.com", "tok");
    let config_start = s.find("config.toml").unwrap_or(0);
    let after = &s[config_start..];
    assert!(
        !after.contains("project_id"),
        "config block must not include project_id: {after}"
    );
}

#[test]
fn install_script_writes_server_url_and_agent_token_to_config() {
    let s = generate_install_script("https://harvest.example.com", "tok-abc");
    assert!(s.contains("server_url"), "config block missing server_url");
    assert!(s.contains("agent_token"), "config block missing agent_token");
}

#[test]
fn install_script_installs_uninstall_command() {
    let s = generate_install_script("https://harvest.example.com", "tok");
    assert!(s.contains("uninstall-harvest-agent"), "missing uninstall script");
}

#[test]
fn install_script_sets_up_systemd_service() {
    let s = generate_install_script("https://harvest.example.com", "tok");
    assert!(s.contains("systemd"), "missing systemd setup");
    assert!(s.contains("harvest-agent.service"), "missing service file name");
}

#[test]
fn install_script_exits_if_not_root() {
    let s = generate_install_script("https://harvest.example.com", "tok");
    assert!(s.contains("id -u"), "missing root check");
}

#[tokio::test]
async fn registry_execute_unknown_agent_returns_error() {
    let r = MachineRegistry::new();
    let e = r.execute("nonexistent", "echo hi".into(), 5).await.unwrap_err();
    assert!(e.contains("not connected"), "got: {e}");
}

#[tokio::test]
async fn registry_agents_for_project_empty_initially() {
    assert!(MachineRegistry::new().agents_for_project("proj-1").is_empty());
}

#[tokio::test]
async fn registry_agents_for_project_scoped_correctly() {
    use knowledge_server::machines::{ConnectedAgent, ServerToAgent};
    use tokio::sync::mpsc;
    use chrono::Utc;

    let registry = MachineRegistry::new();
    let (tx1, _) = mpsc::channel::<ServerToAgent>(8);
    let (tx2, _) = mpsc::channel::<ServerToAgent>(8);

    registry.agents.insert("a1".into(), ConnectedAgent {
        id: "a1".into(), project_id: "proj-1".into(), hostname: "h1".into(),
        connected_at: Utc::now(), sender: tx1,
    });
    registry.agents.insert("a2".into(), ConnectedAgent {
        id: "a2".into(), project_id: "proj-2".into(), hostname: "h2".into(),
        connected_at: Utc::now(), sender: tx2,
    });

    let p1 = registry.agents_for_project("proj-1");
    assert_eq!(p1.len(), 1);
    assert_eq!(p1[0]["hostname"], "h1");

    assert_eq!(registry.agents_for_project("proj-2").len(), 1);
    assert!(registry.agents_for_project("proj-3").is_empty());
}

#[tokio::test]
async fn install_script_endpoint_missing_project_returns_4xx_or_503() {
    let app = machines_router(make_state(MachineRegistry::new()));
    let req = Request::builder()
        .uri("/agents/missing-proj/install.sh")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::SERVICE_UNAVAILABLE,
        "got {}", resp.status()
    );
}

#[tokio::test]
async fn binary_endpoint_missing_path_returns_404() {
    let app = machines_router(make_state(MachineRegistry::new()));
    let req = Request::builder()
        .uri("/agents/binary/harvest-agent")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_events_no_token_returns_401() {
    let app = machines_router(make_state(MachineRegistry::new()));
    let req = Request::builder()
        .uri("/agent/events?hostname=test")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_ping_no_token_returns_401() {
    let app = machines_router(make_state(MachineRegistry::new()));
    let req = Request::builder()
        .method("POST")
        .uri("/agent/ping")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_results_no_token_returns_401() {
    let app = machines_router(make_state(MachineRegistry::new()));
    let req = Request::builder()
        .method("POST")
        .uri("/agent/results")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"request_id":"x","stdout":"","stderr":"","exit_code":0}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_ping_invalid_token_returns_401() {
    let app = machines_router(make_state(MachineRegistry::new()));
    let req = Request::builder()
        .method("POST")
        .uri("/agent/ping")
        .header("Authorization", "Bearer bad-token")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[cfg(test)]
mod docker_tests {
    use super::*;
    use knowledge_server::neo4j::Neo4jClient;
    use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j, Neo4jImageExt as _};
    use std::future::IntoFuture as _;
    use tokio::net::TcpListener;

    async fn spawn_server(neo4j: Arc<Neo4jClient>) -> std::net::SocketAddr {
        let state = Arc::new(MachineState {
            registry:    MachineRegistry::new(),
            neo4j:       Some(neo4j),
            binary_path: None,
            server_url:  "http://localhost".into(),
            lxd:         None,
        });
        let app      = machines_router(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr     = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app).into_future());
        addr
    }

    async fn seed_project(neo4j: &Neo4jClient, install_token: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        neo4j.query_read(
            "CREATE (p:Project {id: $id, install_token: $tok, name: 'test', group_id: 'g1', created_by: 'u1', created_at: '2026-01-01'}) RETURN p.id AS id",
            json!({ "id": id, "tok": install_token }),
        ).await.unwrap();
        id
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn sse_agent_registers_with_install_token_and_receives_permanent_token() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "test-install-token-sse";
        seed_project(&neo4j, install_token).await;

        let addr = spawn_server(neo4j).await;
        let url  = format!("http://127.0.0.1:{}/agent/events?hostname=test-host", addr.port());

        let client = reqwest::Client::new();
        let resp = client.get(&url)
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        assert_eq!(resp.status(), 200);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("text/event-stream"), "content-type: {ct}");

        use futures_util::StreamExt as _;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let event = loop {
            let chunk = stream.next().await.unwrap().unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if let Some(pos) = buf.find("\n\n") {
                break buf[..pos].to_string();
            }
        };

        let data = event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: "))
            .expect("no data line in SSE event");

        let val: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(val["type"], "registered", "got: {val}");
        let new_token = val["agent_token"].as_str().expect("missing agent_token");
        assert!(!new_token.is_empty());
        assert_ne!(new_token, install_token);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn sse_invalid_token_returns_401() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let addr = spawn_server(neo4j).await;
        let url  = format!("http://127.0.0.1:{}/agent/events?hostname=attacker", addr.port());

        let client = reqwest::Client::new();
        let resp = client.get(&url)
            .header("Authorization", "Bearer completely-wrong-token")
            .send().await.unwrap();

        assert_eq!(resp.status(), 401);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn ping_updates_last_seen_and_returns_200() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "ping-test-install-tok";
        seed_project(&neo4j, install_token).await;

        let addr = spawn_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());

        let client = reqwest::Client::new();
        use futures_util::StreamExt as _;
        let sse_resp = client.get(format!("{base}/agent/events?hostname=ping-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();
        let (event, _rest) = loop {
            let chunk = stream.next().await.unwrap().unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if let Some(pos) = buf.find("\n\n") {
                let ev = buf[..pos].to_string();
                let rest = buf[pos+2..].to_string();
                break (ev, rest);
            }
        };

        let data = event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let val: serde_json::Value = serde_json::from_str(data).unwrap();
        let perm_token = val["agent_token"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let ping_resp = client.post(format!("{base}/agent/ping"))
            .header("Authorization", format!("Bearer {perm_token}"))
            .send().await.unwrap();

        assert_eq!(ping_resp.status(), 200);

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'ping-host'}) RETURN m.last_seen AS last_seen",
            json!({}),
        ).await.unwrap();
        assert!(!rows.is_empty(), "machine not in DB");
        assert!(rows[0]["last_seen"].as_str().is_some());
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn lxd_marker_tags_new_machine_as_lxd_managed() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "lxd-marker-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        neo4j.query_read(
            "CREATE (:LxdInstance {
                 project_id: $pid, hostname: $h, lxd_project: 'harvest',
                 description: 'test agent', created_at: '2026-01-01'
             })",
            json!({ "pid": project_id, "h": "agent-lxd-host" }),
        ).await.unwrap();

        let addr = spawn_server(Arc::clone(&neo4j)).await;
        let url  = format!("http://127.0.0.1:{}/agent/events?hostname=agent-lxd-host", addr.port());

        let client = reqwest::Client::new();
        let resp = client.get(&url)
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();
        assert_eq!(resp.status(), 200);

        use futures_util::StreamExt as _;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        loop {
            let chunk = stream.next().await.unwrap().unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.find("\n\n").is_some() { break; }
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let rows = neo4j.query_read(
            "MATCH (m:Machine {project_id: $pid, hostname: $h})
             RETURN m.provider AS provider, m.lxd_instance AS lxd_instance, m.description AS description",
            json!({ "pid": project_id, "h": "agent-lxd-host" }),
        ).await.unwrap();
        let m = rows.into_iter().next().expect("machine not created");
        assert_eq!(m["provider"], "lxd");
        assert_eq!(m["lxd_instance"], "agent-lxd-host");
        assert_eq!(m["description"], "test agent");

        let marker_rows = neo4j.query_read(
            "MATCH (li:LxdInstance {project_id: $pid, hostname: $h}) RETURN li",
            json!({ "pid": project_id, "h": "agent-lxd-host" }),
        ).await.unwrap();
        assert!(marker_rows.is_empty(), "LxdInstance marker should be consumed");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn manually_installed_machine_has_no_lxd_provider() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "manual-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_server(Arc::clone(&neo4j)).await;
        let url  = format!("http://127.0.0.1:{}/agent/events?hostname=manual-host", addr.port());

        let client = reqwest::Client::new();
        let resp = client.get(&url)
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();
        assert_eq!(resp.status(), 200);

        use futures_util::StreamExt as _;
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        loop {
            let chunk = stream.next().await.unwrap().unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.find("\n\n").is_some() { break; }
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let rows = neo4j.query_read(
            "MATCH (m:Machine {project_id: $pid, hostname: $h}) RETURN m.provider AS provider",
            json!({ "pid": project_id, "h": "manual-host" }),
        ).await.unwrap();
        let m = rows.into_iter().next().expect("machine not created");
        assert!(m["provider"].is_null(), "manually-installed machine should have no provider tag");
    }
}
