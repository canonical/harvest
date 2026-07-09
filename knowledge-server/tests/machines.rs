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
async fn agents_install_script_route_not_shadowed_by_proxy_route() {
    use knowledge_server::machines::handlers::machines_protected_router;

    let state = make_state(MachineRegistry::new());
    let app = machines_router(Arc::clone(&state)).merge(machines_protected_router(Arc::clone(&state)));

    let install_req = Request::builder()
        .uri("/agents/some-project/install.sh")
        .body(Body::empty()).unwrap();
    let install_resp = app.clone().oneshot(install_req).await.unwrap();
    assert_eq!(
        install_resp.status(), StatusCode::SERVICE_UNAVAILABLE,
        "install.sh must still resolve to the installer, not the proxy"
    );

    let proxy_req = Request::builder()
        .uri("/agents/some-agent/some-route")
        .body(Body::empty()).unwrap();
    let proxy_resp = app.oneshot(proxy_req).await.unwrap();
    assert_eq!(
        proxy_resp.status(), StatusCode::INTERNAL_SERVER_ERROR,
        "an arbitrary route name must fall through to the proxy handler (500 here because \
         Extension<Claims> isn't populated without the auth middleware layer in this test)"
    );
}

#[tokio::test]
async fn agents_binary_route_not_shadowed_by_proxy_route() {
    use knowledge_server::machines::handlers::machines_protected_router;

    let state = make_state(MachineRegistry::new());
    let app = machines_router(Arc::clone(&state)).merge(machines_protected_router(Arc::clone(&state)));

    let req = Request::builder()
        .uri("/agents/binary/harvest-agent")
        .body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "binary route must still resolve to the binary handler");
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

    use axum::{extract::Request, middleware::{from_fn, Next}};
    use knowledge_server::{auth::jwt::Claims, machines::handlers::machines_protected_router, lxd::LxdClient};

    async fn inject_admin_claims(mut req: Request, next: Next) -> axum::response::Response {
        req.extensions_mut().insert(Claims {
            sub:   "test-admin".into(),
            email: "admin@example.com".into(),
            name:  "Test Admin".into(),
            role:  "admin".into(),
            exp:   9_999_999_999,
        });
        next.run(req).await
    }

    async fn spawn_protected_server(neo4j: Arc<Neo4jClient>, lxd: Option<Arc<LxdClient>>) -> std::net::SocketAddr {
        let state = Arc::new(MachineState {
            registry:    MachineRegistry::new(),
            neo4j:       Some(neo4j),
            binary_path: None,
            server_url:  "http://localhost".into(),
            lxd,
        });
        let app = machines_protected_router(state).layer(from_fn(inject_admin_claims));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app).into_future());
        addr
    }

    async fn seed_machine(neo4j: &Neo4jClient, project_id: &str, agent_id: &str, provider: Option<&str>) {
        neo4j.query_read(
            "MATCH (p:Project {id: $pid})
             CREATE (m:Machine {
                 id: $aid, project_id: $pid, hostname: 'seeded-host',
                 provider: $provider, lxd_instance: 'seeded-instance',
                 created_at: '2026-01-01', last_seen: '2026-01-01'
             })",
            json!({ "pid": project_id, "aid": agent_id, "provider": provider }),
        ).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn start_agent_returns_404_for_missing_agent() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "start-404-tok").await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/nonexistent/start", addr.port()))
            .send().await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn start_agent_returns_400_for_non_lxd_agent() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "start-400-tok").await;
        seed_machine(&neo4j, &project_id, "manual-agent", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/manual-agent/start", addr.port()))
            .send().await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn restart_agent_returns_503_when_lxd_not_configured() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "restart-503-tok").await;
        seed_machine(&neo4j, &project_id, "lxd-agent", Some("lxd")).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/lxd-agent/restart", addr.port()))
            .send().await.unwrap();
        assert_eq!(resp.status(), 503);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn create_port_forward_returns_created_forward_with_url() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-create-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .json(&json!({ "port": 8080, "route_name": "app" }))
            .send().await.unwrap();
        assert_eq!(resp.status(), 200);

        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["port"], 8080);
        assert_eq!(body["route_name"], "app");
        assert!(body["url"].as_str().unwrap().ends_with("/agents/agent-1/app"));
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn create_port_forward_rejects_invalid_port() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-invalid-port-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .json(&json!({ "port": 99999, "route_name": "app" }))
            .send().await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn create_port_forward_rejects_duplicate_route_name() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-dup-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port());
        client.post(&url).json(&json!({ "port": 8080, "route_name": "app" })).send().await.unwrap();
        let resp = client.post(&url).json(&json!({ "port": 9090, "route_name": "app" })).send().await.unwrap();
        assert_eq!(resp.status(), 409);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn list_port_forwards_returns_only_this_agents_forwards() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-list-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        seed_machine(&neo4j, &project_id, "agent-2", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .json(&json!({ "port": 8080, "route_name": "app" })).send().await.unwrap();
        client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-2/port-forwards", addr.port()))
            .json(&json!({ "port": 9090, "route_name": "other" })).send().await.unwrap();

        let resp = client.get(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await.unwrap();
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["route_name"], "app");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn update_port_forward_changes_fields() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-update-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let create_resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .json(&json!({ "port": 8080, "route_name": "app" })).send().await.unwrap();
        let created: Value = create_resp.json().await.unwrap();
        let forward_id = created["id"].as_str().unwrap();

        let resp = client.put(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards/{forward_id}", addr.port()))
            .json(&json!({ "port": 9090 }))
            .send().await.unwrap();
        assert_eq!(resp.status(), 200);
        let updated: Value = resp.json().await.unwrap();
        assert_eq!(updated["port"], 9090);
        assert_eq!(updated["route_name"], "app");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn update_port_forward_returns_404_for_unknown_id() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-update-404-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.put(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards/nonexistent", addr.port()))
            .json(&json!({ "port": 9090 }))
            .send().await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn delete_port_forward_removes_it() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-delete-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let create_resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .json(&json!({ "port": 8080, "route_name": "app" })).send().await.unwrap();
        let created: Value = create_resp.json().await.unwrap();
        let forward_id = created["id"].as_str().unwrap();

        let del_resp = client.delete(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards/{forward_id}", addr.port()))
            .send().await.unwrap();
        assert_eq!(del_resp.status(), 200);

        let list_resp = client.get(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .send().await.unwrap();
        let body: Value = list_resp.json().await.unwrap();
        assert!(body.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forwards_rejects_reserved_route_name() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-reserved-tok").await;
        seed_machine(&neo4j, &project_id, "agent-1", None).await;
        let addr = spawn_protected_server(Arc::clone(&neo4j), None).await;

        let client = reqwest::Client::new();
        let resp = client.post(format!("http://127.0.0.1:{}/projects/{project_id}/agents/agent-1/port-forwards", addr.port()))
            .json(&json!({ "port": 8080, "route_name": "install.sh" }))
            .send().await.unwrap();
        assert_eq!(resp.status(), 400);
    }

    async fn spawn_console_server(neo4j: Arc<Neo4jClient>) -> std::net::SocketAddr {
        let state = Arc::new(MachineState {
            registry:    MachineRegistry::new(),
            neo4j:       Some(neo4j),
            binary_path: None,
            server_url:  "http://localhost".into(),
            lxd:         None,
        });
        let app = machines_router(Arc::clone(&state))
            .merge(machines_protected_router(Arc::clone(&state)).layer(from_fn(inject_admin_claims)));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app).into_future());
        addr
    }

    async fn connect_ws(
        url: &str,
        bearer: Option<&str>,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = url.into_client_request().unwrap();
        if let Some(token) = bearer {
            request.headers_mut().insert(
                "Authorization",
                format!("Bearer {token}").parse().unwrap(),
            );
        }
        let (stream, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        stream
    }

    async fn next_sse_event<S, B, E>(buf: &mut String, stream: &mut S) -> String
    where
        S: futures_util::Stream<Item = Result<B, E>> + Unpin,
        B: AsRef<[u8]>,
    {
        use futures_util::StreamExt as _;
        loop {
            if let Some(pos) = buf.find("\n\n") {
                let ev = buf[..pos].to_string();
                buf.drain(..pos + 2);
                return ev;
            }
            let Some(Ok(chunk)) = stream.next().await else { panic!("SSE stream ended unexpectedly") };
            buf.push_str(&String::from_utf8_lossy(chunk.as_ref()));
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn console_relay_bridges_browser_and_agent_bidirectionally() {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "console-relay-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());
        let ws_base = format!("ws://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let sse_resp = http.get(format!("{base}/agent/events?hostname=console-relay-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();

        let registered_event = next_sse_event(&mut buf, &mut stream).await;
        let registered_data = registered_event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let registered: Value = serde_json::from_str(registered_data).unwrap();
        let perm_token = registered["agent_token"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'console-relay-host'}) RETURN m.id AS id",
            json!({}),
        ).await.unwrap();
        let agent_id = rows[0]["id"].as_str().unwrap().to_string();

        let browser_task = tokio::spawn({
            let ws_base = ws_base.clone();
            async move {
                connect_ws(
                    &format!("{ws_base}/projects/{project_id}/agents/{agent_id}/console?cols=80&rows=24"),
                    None,
                ).await
            }
        });

        let open_shell_event = next_sse_event(&mut buf, &mut stream).await;
        let open_shell_data = open_shell_event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let open_shell: Value = serde_json::from_str(open_shell_data).unwrap();
        assert_eq!(open_shell["type"], "open_shell");
        assert_eq!(open_shell["cols"], 80);
        assert_eq!(open_shell["rows"], 24);
        let session_id = open_shell["session_id"].as_str().unwrap().to_string();

        let mut agent_ws = connect_ws(
            &format!("{ws_base}/agent/console/{session_id}"),
            Some(&perm_token),
        ).await;

        agent_ws.send(WsMessage::text(r#"{"type":"ready"}"#)).await.unwrap();

        let mut browser_ws = browser_task.await.unwrap();

        let first = browser_ws.next().await.unwrap().unwrap();
        assert_eq!(first, WsMessage::text(r#"{"type":"ready"}"#));

        browser_ws.send(WsMessage::binary(b"echo hi\n".to_vec())).await.unwrap();
        let got = agent_ws.next().await.unwrap().unwrap();
        assert_eq!(got, WsMessage::binary(b"echo hi\n".to_vec()));

        agent_ws.send(WsMessage::binary(b"hi\n".to_vec())).await.unwrap();
        let got = browser_ws.next().await.unwrap().unwrap();
        assert_eq!(got, WsMessage::binary(b"hi\n".to_vec()));

        browser_ws.send(WsMessage::text(r#"{"type":"resize","cols":100,"rows":40}"#)).await.unwrap();
        let got = agent_ws.next().await.unwrap().unwrap();
        assert_eq!(got, WsMessage::text(r#"{"type":"resize","cols":100,"rows":40}"#));

        browser_ws.close(None).await.unwrap();
        let closed = agent_ws.next().await;
        assert!(
            matches!(closed, None | Some(Ok(WsMessage::Close(_)))),
            "expected agent side to observe closure, got {closed:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn console_open_rejects_unauthorized_project_access() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());

        let client = reqwest::Client::new();
        let resp = client.get(format!("{base}/projects/nonexistent-project/agents/nonexistent-agent/console"))
            .send().await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn console_claim_rejects_wrong_agent_token() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "console-claim-mismatch-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let sse_resp = http.get(format!("{base}/agent/events?hostname=console-claim-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        use futures_util::StreamExt as _;
        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();
        loop {
            let chunk = stream.next().await.unwrap().unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.find("\n\n").is_some() { break; }
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'console-claim-host'}) RETURN m.id AS id",
            json!({}),
        ).await.unwrap();
        let agent_id = rows[0]["id"].as_str().unwrap().to_string();

        let _browser_task = tokio::spawn({
            let ws_base = format!("ws://127.0.0.1:{}", addr.port());
            async move {
                connect_ws(
                    &format!("{ws_base}/projects/{project_id}/agents/{agent_id}/console"),
                    None,
                ).await
            }
        });

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let resp = http.get(format!("{base}/agent/console/wrong-session-id"))
            .header("Authorization", "Bearer completely-different-token")
            .send().await.unwrap();
        assert_eq!(resp.status(), 401);
    }

    async fn spawn_stand_in_http_server() -> std::net::SocketAddr {
        async fn echo_path(req: axum::extract::Request) -> String {
            req.uri().path().to_string()
        }
        let app = axum::Router::new().fallback(echo_path);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(axum::serve(listener, app).into_future());
        addr
    }

    async fn simulate_agent_tunnel(ws_base: &str, session_id: &str, agent_token: &str, target_port: u16) {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let mut agent_ws = connect_ws(&format!("{ws_base}/agent/tunnel/{session_id}"), Some(agent_token)).await;

        let tcp = match tokio::net::TcpStream::connect(("127.0.0.1", target_port)).await {
            Ok(t) => t,
            Err(e) => {
                let _ = agent_ws.send(WsMessage::text(format!(r#"{{"type":"error","message":"{e}"}}"#))).await;
                return;
            }
        };

        if agent_ws.send(WsMessage::text(r#"{"type":"connected"}"#)).await.is_err() {
            return;
        }

        let (mut ws_tx, mut ws_rx) = agent_ws.split();
        let (mut tcp_read, mut tcp_write) = tcp.into_split();

        let writer = tokio::spawn(async move {
            let mut buf = [0u8; 8192];
            loop {
                match tcp_read.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if ws_tx.send(WsMessage::binary(buf[..n].to_vec())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        while let Some(Ok(msg)) = ws_rx.next().await {
            match msg {
                WsMessage::Binary(data) => {
                    if tcp_write.write_all(&data).await.is_err() {
                        break;
                    }
                }
                WsMessage::Close(_) => break,
                _ => {}
            }
        }

        writer.abort();
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forward_proxy_forwards_request_to_agent_port() {

        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "pf-proxy-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());
        let ws_base = format!("ws://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let sse_resp = http.get(format!("{base}/agent/events?hostname=pf-proxy-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();
        let registered_event = next_sse_event(&mut buf, &mut stream).await;
        let registered_data = registered_event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let registered: Value = serde_json::from_str(registered_data).unwrap();
        let perm_token = registered["agent_token"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'pf-proxy-host'}) RETURN m.id AS id",
            json!({}),
        ).await.unwrap();
        let agent_id = rows[0]["id"].as_str().unwrap().to_string();

        let stand_in_addr = spawn_stand_in_http_server().await;
        http.post(format!("{base}/projects/{project_id}/agents/{agent_id}/port-forwards"))
            .json(&json!({ "port": stand_in_addr.port(), "route_name": "app" }))
            .send().await.unwrap();

        let agent_task = tokio::spawn({
            let ws_base = ws_base.clone();
            let perm_token = perm_token.clone();
            async move {
                let open_tunnel_event = next_sse_event(&mut buf, &mut stream).await;
                let data = open_tunnel_event.lines()
                    .find(|l| l.starts_with("data: "))
                    .and_then(|l| l.strip_prefix("data: ")).unwrap();
                let val: Value = serde_json::from_str(data).unwrap();
                assert_eq!(val["type"], "open_tunnel");
                let session_id = val["session_id"].as_str().unwrap().to_string();
                simulate_agent_tunnel(&ws_base, &session_id, &perm_token, stand_in_addr.port()).await;
            }
        });

        let resp = http.get(format!("{base}/agents/{agent_id}/app")).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "/");

        agent_task.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forward_proxy_forwards_subpath() {

        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "pf-subpath-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());
        let ws_base = format!("ws://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let sse_resp = http.get(format!("{base}/agent/events?hostname=pf-subpath-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();
        let registered_event = next_sse_event(&mut buf, &mut stream).await;
        let registered_data = registered_event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let registered: Value = serde_json::from_str(registered_data).unwrap();
        let perm_token = registered["agent_token"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'pf-subpath-host'}) RETURN m.id AS id",
            json!({}),
        ).await.unwrap();
        let agent_id = rows[0]["id"].as_str().unwrap().to_string();

        let stand_in_addr = spawn_stand_in_http_server().await;
        http.post(format!("{base}/projects/{project_id}/agents/{agent_id}/port-forwards"))
            .json(&json!({ "port": stand_in_addr.port(), "route_name": "app" }))
            .send().await.unwrap();

        let agent_task = tokio::spawn({
            let ws_base = ws_base.clone();
            let perm_token = perm_token.clone();
            async move {
                let open_tunnel_event = next_sse_event(&mut buf, &mut stream).await;
                let data = open_tunnel_event.lines()
                    .find(|l| l.starts_with("data: "))
                    .and_then(|l| l.strip_prefix("data: ")).unwrap();
                let val: Value = serde_json::from_str(data).unwrap();
                let session_id = val["session_id"].as_str().unwrap().to_string();
                simulate_agent_tunnel(&ws_base, &session_id, &perm_token, stand_in_addr.port()).await;
            }
        });

        let resp = http.get(format!("{base}/agents/{agent_id}/app/foo/bar")).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "/foo/bar");

        agent_task.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forward_proxy_returns_502_when_agent_not_connected() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-offline-tok").await;
        seed_machine(&neo4j, &project_id, "offline-agent", None).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        http.post(format!("{base}/projects/{project_id}/agents/offline-agent/port-forwards"))
            .json(&json!({ "port": 8080, "route_name": "app" }))
            .send().await.unwrap();

        let resp = http.get(format!("{base}/agents/offline-agent/app")).send().await.unwrap();
        assert_eq!(resp.status(), 502);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forward_proxy_forwards_request_with_trailing_slash() {
        use futures_util::{SinkExt as _, StreamExt as _};

        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "pf-trailing-slash-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());
        let ws_base = format!("ws://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let sse_resp = http.get(format!("{base}/agent/events?hostname=pf-trailing-slash-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();
        let registered_event = next_sse_event(&mut buf, &mut stream).await;
        let registered_data = registered_event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let registered: Value = serde_json::from_str(registered_data).unwrap();
        let perm_token = registered["agent_token"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'pf-trailing-slash-host'}) RETURN m.id AS id",
            json!({}),
        ).await.unwrap();
        let agent_id = rows[0]["id"].as_str().unwrap().to_string();

        let stand_in_addr = spawn_stand_in_http_server().await;
        http.post(format!("{base}/projects/{project_id}/agents/{agent_id}/port-forwards"))
            .json(&json!({ "port": stand_in_addr.port(), "route_name": "app" }))
            .send().await.unwrap();

        let agent_task = tokio::spawn({
            let ws_base = ws_base.clone();
            let perm_token = perm_token.clone();
            async move {
                let open_tunnel_event = next_sse_event(&mut buf, &mut stream).await;
                let data = open_tunnel_event.lines()
                    .find(|l| l.starts_with("data: "))
                    .and_then(|l| l.strip_prefix("data: ")).unwrap();
                let val: Value = serde_json::from_str(data).unwrap();
                let session_id = val["session_id"].as_str().unwrap().to_string();
                simulate_agent_tunnel(&ws_base, &session_id, &perm_token, stand_in_addr.port()).await;
            }
        });

        let resp = http.get(format!("{base}/agents/{agent_id}/app/")).send().await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.text().await.unwrap(), "/");

        agent_task.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forward_proxy_returns_404_for_unknown_route() {
        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let project_id = seed_project(&neo4j, "pf-404-tok").await;
        seed_machine(&neo4j, &project_id, "some-agent", None).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let resp = http.get(format!("{base}/agents/some-agent/nope")).send().await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    async fn port_forward_proxy_returns_502_when_agent_side_connection_refused() {

        let container = Neo4j::default().start().await;
        let uri  = container.image().bolt_uri_ipv4();
        let user = container.image().user().unwrap_or("neo4j");
        let pass = container.image().password().unwrap_or("neo");
        let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

        let install_token = "pf-refused-install-tok";
        let project_id = seed_project(&neo4j, install_token).await;

        let addr = spawn_console_server(Arc::clone(&neo4j)).await;
        let base = format!("http://127.0.0.1:{}", addr.port());
        let ws_base = format!("ws://127.0.0.1:{}", addr.port());

        let http = reqwest::Client::new();
        let sse_resp = http.get(format!("{base}/agent/events?hostname=pf-refused-host"))
            .header("Authorization", format!("Bearer {install_token}"))
            .send().await.unwrap();

        let mut stream = sse_resp.bytes_stream();
        let mut buf = String::new();
        let registered_event = next_sse_event(&mut buf, &mut stream).await;
        let registered_data = registered_event.lines()
            .find(|l| l.starts_with("data: "))
            .and_then(|l| l.strip_prefix("data: ")).unwrap();
        let registered: Value = serde_json::from_str(registered_data).unwrap();
        let perm_token = registered["agent_token"].as_str().unwrap().to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let rows = neo4j.query_read(
            "MATCH (m:Machine {hostname: 'pf-refused-host'}) RETURN m.id AS id",
            json!({}),
        ).await.unwrap();
        let agent_id = rows[0]["id"].as_str().unwrap().to_string();

        let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed_listener.local_addr().unwrap().port();
        drop(closed_listener);

        http.post(format!("{base}/projects/{project_id}/agents/{agent_id}/port-forwards"))
            .json(&json!({ "port": closed_port, "route_name": "app" }))
            .send().await.unwrap();

        let agent_task = tokio::spawn({
            let ws_base = ws_base.clone();
            let perm_token = perm_token.clone();
            async move {
                let open_tunnel_event = next_sse_event(&mut buf, &mut stream).await;
                let data = open_tunnel_event.lines()
                    .find(|l| l.starts_with("data: "))
                    .and_then(|l| l.strip_prefix("data: ")).unwrap();
                let val: Value = serde_json::from_str(data).unwrap();
                let session_id = val["session_id"].as_str().unwrap().to_string();
                simulate_agent_tunnel(&ws_base, &session_id, &perm_token, closed_port).await;
            }
        });

        let resp = http.get(format!("{base}/agents/{agent_id}/app")).send().await.unwrap();
        assert_eq!(resp.status(), 502);

        agent_task.await.unwrap();
    }
}
