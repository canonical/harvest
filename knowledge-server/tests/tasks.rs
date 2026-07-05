use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{delete as route_delete, get as route_get, post as route_post},
    Router,
};
use http_body_util::BodyExt as _;
use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j};
use serde_json::{json, Value};
use tower::ServiceExt as _;
use uuid::Uuid;

use knowledge_server::{
    agent::Agent,
    api::ProjectAgentBuilder,
    auth::{self, jwt},
    llm::{
        LlmProvider,
        types::{LlmResponse, Message, ToolDefinition},
    },
    machines::MachineRegistry,
    neo4j::Neo4jClient,
    projects::handlers::{
        ProjectState,
        create_project,
        list_tasks, create_task, delete_task, run_task, get_task_logs,
    },
};

struct FixedTextLlm(String);
impl FixedTextLlm {
    fn new(t: impl Into<String>) -> Arc<Self> { Arc::new(Self(t.into())) }
}
#[async_trait]
impl LlmProvider for FixedTextLlm {
    async fn chat(&self, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone() })
    }
}

const JWT_SECRET: &str = "test-tasks-secret";

fn tasks_app(neo4j: Arc<Neo4jClient>) -> Router {
    let secret   = Arc::new(JWT_SECRET.to_string());
    let llm: Arc<dyn LlmProvider> = FixedTextLlm::new("task output");
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        neo4j:                      Arc::clone(&neo4j),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::new(knowledge_server::skills::SkillRegistry::new()),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let state = Arc::new(ProjectState::new(neo4j, agent, builder));

    Router::new()
        .route("/projects",                                 route_post(create_project))
        .route("/projects/:pid/tasks",                      route_get(list_tasks).post(create_task))
        .route("/projects/:pid/tasks/:tid",                 route_delete(delete_task))
        .route("/projects/:pid/tasks/:tid/run",             route_post(run_task))
        .route("/projects/:pid/tasks/:tid/logs",            route_get(get_task_logs))
        .with_state(state)
        .layer(from_fn_with_state(secret, auth::require_auth))
}

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT project_id IF NOT EXISTS FOR (p:Project) REQUIRE p.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT task_id IF NOT EXISTS FOR (t:Task) REQUIRE t.id IS UNIQUE").await.unwrap();
}

async fn make_user(neo4j: &Neo4jClient, email: &str, name: &str, role: &str) -> (String, String) {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "CREATE (:User {id:$id,email:$email,name:$name,role:$role,\
                        provider:'password',created_at:$now}) RETURN 1",
        json!({"id":id,"email":email,"name":name,"role":role,"now":now}),
    ).await.unwrap();
    let token = jwt::issue(JWT_SECRET, &id, email, name, role).unwrap();
    (id, token)
}

async fn make_group(neo4j: &Neo4jClient, name: &str) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "CREATE (:Group {id:$id,name:$name,description:'',created_at:$now}) RETURN 1",
        json!({"id":id,"name":name,"now":now}),
    ).await.unwrap();
    id
}

async fn join_group(neo4j: &Neo4jClient, user_id: &str, group_id: &str) {
    neo4j.query_read(
        "MATCH (u:User{id:$uid}),(g:Group{id:$gid}) MERGE (u)-[:MEMBER_OF]->(g) RETURN 1",
        json!({"uid":user_id,"gid":group_id}),
    ).await.unwrap();
}

fn cookie(token: &str) -> String { format!("token={token}") }

fn req_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri)
        .header("Cookie", cookie(token)).body(Body::empty()).unwrap()
}

fn req_post(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder().method("POST").uri(uri)
        .header("Cookie", cookie(token))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap())).unwrap()
}

fn req_post_empty(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("POST").uri(uri)
        .header("Cookie", cookie(token))
        .body(Body::empty()).unwrap()
}

fn req_del(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("DELETE").uri(uri)
        .header("Cookie", cookie(token)).body(Body::empty()).unwrap()
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp   = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes  = resp.into_body().collect().await.unwrap().to_bytes();
    let json   = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn send_raw(app: Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp   = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes  = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

macro_rules! neo4j {
    ($c:ident, $neo4j:ident) => {
        let $c = Neo4j::default().start().await;
        let uri  = $c.image().bolt_uri_ipv4();
        let user = $c.image().user().unwrap_or("neo4j");
        let pass = $c.image().password().unwrap_or("neo");
        let $neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());
        setup_constraints(&$neo4j).await;
    };
}

async fn seed_project(app: &Router, token: &str, group_id: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_post("/projects", token, json!({"name":"Test Project","group_id":group_id})),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

async fn seed_task(app: &Router, token: &str, pid: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/tasks"), token, json!({
            "name": "My task",
            "prompt": "Summarise the project"
        })),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

async fn seed_task_full(
    app: &Router, token: &str, pid: &str, name: &str, prompt: &str, depends_on: &[&str],
) -> String {
    let (_, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/tasks"), token, json!({
            "name": name, "prompt": prompt, "depends_on": depends_on
        })),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

fn parse_sse_events(bytes: &[u8]) -> Vec<Value> {
    std::str::from_utf8(bytes).unwrap().lines().filter_map(|l| {
        l.trim().strip_prefix("data: ")
            .and_then(|d| serde_json::from_str(d).ok())
    }).collect()
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_tasks_empty_for_new_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/tasks"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_task_returns_201_with_fields() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/tasks"), &tok, json!({
            "name": "Deploy check",
            "prompt": "Is the deploy green?"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());
    assert_eq!(body["name"], "Deploy check");
    assert_eq!(body["prompt"], "Is the deploy green?");
    assert_eq!(body["status"], "idle");
    assert!(body["created_at"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_task_requires_name() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/tasks"), &tok, json!({
            "name": "",
            "prompt": "Do something"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_task_requires_prompt() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/tasks"), &tok, json!({
            "name": "My task",
            "prompt": ""
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_tasks_after_create_returns_summary() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    send(
        app.clone(),
        req_post(&format!("/projects/{pid}/tasks"), &tok, json!({
            "name": "Check health",
            "prompt": "Run a health check"
        })),
    ).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/tasks"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "Check health");
    assert!(arr[0]["id"].is_string());
    assert_eq!(arr[0]["status"], "idle");
    assert!(arr[0]["created_at"].is_string());
    assert!(arr[0].get("prompt").is_none(), "list should not expose prompt");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_task_removes_it() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let tid = seed_task(&app, &tok, &pid).await;

    let (status, _) = send(app.clone(), req_del(&format!("/projects/{pid}/tasks/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/tasks"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_task_logs_returns_idle_status_before_run() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let tid = seed_task(&app, &tok, &pid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/tasks/{tid}/logs"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "idle");
    assert!(body["output"].is_null() || body["output"] == "");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_task_streams_sse_and_stores_output() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let tid = seed_task(&app, &tok, &pid).await;

    let (status, body_bytes) = send_raw(
        app.clone(),
        req_post_empty(&format!("/projects/{pid}/tasks/{tid}/run"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let body_str = std::str::from_utf8(&body_bytes).unwrap();
    assert!(body_str.contains("done"), "SSE body should contain a done event");

    let (status, logs) = send(app, req_get(&format!("/projects/{pid}/tasks/{tid}/logs"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(logs["status"], "done");
    assert!(logs["output"].is_string());
    assert!(!logs["output"].as_str().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_task_returns_404_for_unknown_task() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send_raw(
        app,
        req_post_empty(&format!("/projects/{pid}/tasks/no-such-id/run"), &tok),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn non_member_cannot_list_tasks() {
    neo4j!(c, neo4j);
    let (uid, tok)   = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, outsider) = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/tasks"), &outsider)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn admin_can_access_any_project_tasks() {
    neo4j!(c, neo4j);
    let (uid, tok)   = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/tasks"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_task_with_depends_on_stores_field() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let dep_id = seed_task(&app, &tok, &pid).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/tasks"), &tok, json!({
            "name": "Dependent",
            "prompt": "Do the thing",
            "depends_on": [dep_id]
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let deps = body["depends_on"].as_array().unwrap();
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].as_str().unwrap(), dep_id.as_str());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_task_with_unknown_dep_returns_400() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/tasks"), &tok, json!({
            "name": "Bad dep",
            "prompt": "Do something",
            "depends_on": ["nonexistent-id"]
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_tasks_includes_depends_on() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let dep_id = seed_task_full(&app, &tok, &pid, "Base", "Run base", &[]).await;
    seed_task_full(&app, &tok, &pid, "Child", "Run child", &[dep_id.as_str()]).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/tasks"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    for item in arr {
        assert!(item["depends_on"].is_array(), "depends_on must be an array");
    }
    let child = arr.iter().find(|t| t["name"] == "Child").unwrap();
    assert_eq!(child["depends_on"].as_array().unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_task_no_deps_emits_task_id_and_run_done() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let tid = seed_task(&app, &tok, &pid).await;

    let (status, bytes) = send_raw(
        app, req_post_empty(&format!("/projects/{pid}/tasks/{tid}/run"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let events = parse_sse_events(&bytes);
    assert!(events.iter().any(|e| e["type"] == "task_start" && e["task_id"] == tid.as_str()));
    assert!(events.iter().any(|e| e["type"] == "done" && e["task_id"] == tid.as_str()));
    assert_eq!(events.last().unwrap()["type"], "run_done");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_task_runs_dependency_before_target() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let dep_id  = seed_task_full(&app, &tok, &pid, "Base",  "Run base", &[]).await;
    let task_id = seed_task_full(&app, &tok, &pid, "Child", "Run child", &[dep_id.as_str()]).await;

    let (status, bytes) = send_raw(
        app.clone(),
        req_post_empty(&format!("/projects/{pid}/tasks/{task_id}/run"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let events = parse_sse_events(&bytes);
    assert!(events.iter().any(|e| e["type"] == "task_start" && e["task_id"] == dep_id.as_str()));
    assert!(events.iter().any(|e| e["type"] == "done"       && e["task_id"] == dep_id.as_str()));
    assert!(events.iter().any(|e| e["type"] == "task_start" && e["task_id"] == task_id.as_str()));
    assert!(events.iter().any(|e| e["type"] == "done"       && e["task_id"] == task_id.as_str()));
    assert_eq!(events.last().unwrap()["type"], "run_done");

    let dep_start_pos  = events.iter().position(|e| e["type"] == "task_start" && e["task_id"] == dep_id.as_str()).unwrap();
    let task_start_pos = events.iter().position(|e| e["type"] == "task_start" && e["task_id"] == task_id.as_str()).unwrap();
    assert!(dep_start_pos < task_start_pos, "dependency must start before dependent");

    let (_, dep_logs)  = send(app.clone(), req_get(&format!("/projects/{pid}/tasks/{dep_id}/logs"),  &tok)).await;
    let (_, task_logs) = send(app,         req_get(&format!("/projects/{pid}/tasks/{task_id}/logs"), &tok)).await;
    assert_eq!(dep_logs["status"],  "done");
    assert_eq!(task_logs["status"], "done");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_task_three_level_chain_all_complete() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let a = seed_task_full(&app, &tok, &pid, "A", "task A", &[]).await;
    let b = seed_task_full(&app, &tok, &pid, "B", "task B", &[a.as_str()]).await;
    let c = seed_task_full(&app, &tok, &pid, "C", "task C", &[b.as_str()]).await;

    let (status, bytes) = send_raw(
        app.clone(), req_post_empty(&format!("/projects/{pid}/tasks/{c}/run"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let events = parse_sse_events(&bytes);
    for tid in [&a, &b, &c] {
        assert!(events.iter().any(|e| e["type"] == "task_start" && e["task_id"] == tid.as_str()));
        assert!(events.iter().any(|e| e["type"] == "done"       && e["task_id"] == tid.as_str()));
    }
    assert_eq!(events.last().unwrap()["type"], "run_done");

    let pos = |tid: &str, etype: &str| -> usize {
        events.iter().position(|e| e["type"] == etype && e["task_id"] == tid).unwrap()
    };
    assert!(pos(&a, "task_start") < pos(&b, "task_start"));
    assert!(pos(&b, "task_start") < pos(&c, "task_start"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_task_parallel_deps_both_complete() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = tasks_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid).await;
    let dep_a  = seed_task_full(&app, &tok, &pid, "DepA", "task dep_a", &[]).await;
    let dep_b  = seed_task_full(&app, &tok, &pid, "DepB", "task dep_b", &[]).await;
    let target = seed_task_full(&app, &tok, &pid, "Target", "task target",
                                &[dep_a.as_str(), dep_b.as_str()]).await;

    let (status, bytes) = send_raw(
        app.clone(), req_post_empty(&format!("/projects/{pid}/tasks/{target}/run"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let events = parse_sse_events(&bytes);
    for tid in [&dep_a, &dep_b, &target] {
        assert!(events.iter().any(|e| e["type"] == "task_start" && e["task_id"] == tid.as_str()));
        assert!(events.iter().any(|e| e["type"] == "done"       && e["task_id"] == tid.as_str()));
    }
    let dep_a_done = events.iter().position(|e| e["type"] == "done" && e["task_id"] == dep_a.as_str()).unwrap();
    let dep_b_done = events.iter().position(|e| e["type"] == "done" && e["task_id"] == dep_b.as_str()).unwrap();
    let target_start = events.iter().position(|e| e["type"] == "task_start" && e["task_id"] == target.as_str()).unwrap();
    assert!(dep_a_done < target_start);
    assert!(dep_b_done < target_start);
    assert_eq!(events.last().unwrap()["type"], "run_done");
}
