use harvest_db::Db;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{delete as route_delete, get as route_get, post as route_post, put as route_put},
    Router,
};
use http_body_util::BodyExt as _;
use harvest_db::test_support::TestDb;
use serde_json::{json, Value};
use tower::ServiceExt as _;
use uuid::Uuid;

use knowledge_server::{
    agent::Agent,
    api::ProjectAgentBuilder,
    auth::{self, jwt},
    llm::{
        LlmProvider,
        types::{LlmResponse, Message, ModelInfo, ToolCall, ToolDefinition, Usage},
    },
    machines::MachineRegistry,

    projects::handlers::{
        ProjectState,
        create_conversation, create_project, delete_conversation, delete_project,
        get_conversation, get_project, list_conversations, list_projects,
        project_query, project_query_stream, resume_confirm_action,
        update_conversation, update_project,
    },
};

struct FixedTextLlm(String);
impl FixedTextLlm {
    fn new(t: impl Into<String>) -> Arc<Self> { Arc::new(Self(t.into())) }
}
#[async_trait]
impl LlmProvider for FixedTextLlm {
    fn id(&self) -> &str { "fixed-text" }
    fn kind(&self) -> &str { "mock" }
    fn default_model(&self) -> &str { "mock-model" }
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> { Ok(vec![]) }
    async fn chat_with(&self, _model: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone(), usage: Usage::default() })
    }
}

struct ScriptedLlm(std::sync::Mutex<std::collections::VecDeque<LlmResponse>>);
impl ScriptedLlm {
    fn new(responses: Vec<LlmResponse>) -> Arc<Self> {
        Arc::new(Self(std::sync::Mutex::new(responses.into())))
    }
}
#[async_trait]
impl LlmProvider for ScriptedLlm {
    fn id(&self) -> &str { "scripted" }
    fn kind(&self) -> &str { "mock" }
    fn default_model(&self) -> &str { "mock-model" }
    async fn list_models(&self) -> Result<Vec<ModelInfo>> { Ok(vec![]) }
    async fn chat_with(&self, _model: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
        self.0.lock().unwrap().pop_front()
            .ok_or_else(|| anyhow::anyhow!("ScriptedLlm: no more responses"))
    }
}

const JWT_SECRET: &str = "test-projects-secret";

fn projects_app(db: Arc<Db>) -> Router {
    projects_app_with_llm(db, FixedTextLlm::new("stub answer"))
}

fn projects_app_with_llm(db: Arc<Db>, llm: Arc<dyn knowledge_server::llm::LlmProvider>) -> Router {
    let secret   = Arc::new(JWT_SECRET.to_string());
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 4));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        db:                      Arc::clone(&db),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::new(knowledge_server::skills::SkillStore::new(Arc::clone(&db))),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             4,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let state = Arc::new(ProjectState::new(db, agent, builder, Arc::clone(&llm) as Arc<dyn LlmProvider>, Arc::new(vec![]), None, Arc::new(knowledge_server::cost::PricingTable::default())));

    Router::new()
        .route("/projects",     route_get(list_projects).post(create_project))
        .route("/projects/:pid",
               route_get(get_project).put(update_project).delete(delete_project))
        .route("/projects/:pid/conversations",
               route_get(list_conversations).post(create_conversation))
        .route("/projects/:pid/conversations/:cid",
               route_get(get_conversation).put(update_conversation).delete(delete_conversation))
        .route("/projects/:pid/conversations/:cid/confirm-action/resume",
               route_post(resume_confirm_action))
        .route("/projects/:pid/query",        route_post(project_query))
        .route("/projects/:pid/query/stream", route_post(project_query_stream))
        .with_state(state)
        .layer(from_fn_with_state(secret, auth::require_auth))
}

async fn make_user(db: &Db, email: &str, name: &str, role: &str) -> (String, String) {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.query(
        "INSERT INTO users (id, email, name, role, provider, created_at)
                        VALUES ($id, $email, $name, $role, 'password', $now)",
        json!({"id":id,"email":email,"name":name,"role":role,"now":now}),
    ).await.unwrap();
    let token = jwt::issue(JWT_SECRET, &id, email, name, role).unwrap();
    (id, token)
}

async fn make_group(db: &Db, name: &str) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.query(
        "INSERT INTO groups (id, name, description, created_at) VALUES ($id, $name, '', $now)",
        json!({"id":id,"name":name,"now":now}),
    ).await.unwrap();
    id
}

async fn join_group(db: &Db, user_id: &str, group_id: &str) {
    db.query(
        "INSERT INTO user_groups (user_id, group_id) VALUES ($uid, $gid) ON CONFLICT DO NOTHING",
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

fn req_put(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder().method("PUT").uri(uri)
        .header("Cookie", cookie(token))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap())).unwrap()
}

fn req_del(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("DELETE").uri(uri)
        .header("Cookie", cookie(token)).body(Body::empty()).unwrap()
}

fn req_get_anon(uri: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri).body(Body::empty()).unwrap()
}

fn req_post_anon(uri: &str, body: Value) -> Request<Body> {
    Request::builder().method("POST").uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap())).unwrap()
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp   = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes  = resp.into_body().collect().await.unwrap().to_bytes();
    let json   = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn wait_for_message_count(app: Router, uri: &str, token: &str, expected: usize) -> Value {
    for _ in 0..40 {
        let (_, conv) = send(app.clone(), req_get(uri, token)).await;
        if conv["messages"].as_array().map(|m| m.len()).unwrap_or(0) >= expected {
            return conv;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("conversation at {uri} never reached {expected} messages");
}

async fn wait_for_message_text(app: Router, uri: &str, token: &str, expected_text: &str) -> Value {
    for _ in 0..40 {
        let (_, conv) = send(app.clone(), req_get(uri, token)).await;
        if conv["messages"].as_array()
            .and_then(|m| m.last())
            .and_then(|m| m["text"].as_str())
            .map(|t| t == expected_text)
            .unwrap_or(false)
        {
            return conv;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("conversation at {uri} never reached text {expected_text:?}");
}

mod auth_guards {
    use super::*;

    fn stub_router() -> Router {
        use axum::{Json, http::StatusCode};
        use axum::middleware::from_fn_with_state;
        let secret = Arc::new(JWT_SECRET.to_string());
        Router::new()
            .route("/projects",      axum::routing::get(|| async { (StatusCode::OK, Json(json!([]))) }))
            .route("/projects/:pid", axum::routing::get(|| async { (StatusCode::OK, Json(json!({}))) }))
            .route("/projects",      axum::routing::post(|| async { (StatusCode::CREATED, Json(json!({}))) }))
            .layer(from_fn_with_state(secret, auth::require_auth))
    }

    #[tokio::test]
    async fn list_projects_without_token_returns_401() {
        let (status, _) = send(stub_router(), req_get_anon("/projects")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_project_without_token_returns_401() {
        let (status, _) = send(stub_router(), req_get_anon("/projects/any-id")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn create_project_without_token_returns_401() {
        let (status, _) = send(stub_router(),
            req_post_anon("/projects", json!({"name":"x","group_id":"g"}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

macro_rules! db {
    ($c:ident, $db:ident) => {
        let $c = TestDb::new().await;
        let $db = Arc::new($c.db.clone());
    };
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_projects_empty_for_new_user() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (status, body) = send(projects_app(db), req_get("/projects", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_succeeds_for_group_member() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let (status, body) = send(projects_app(db),
        req_post("/projects", &tok, json!({"name":"My Project","group_id":gid,"description":"desc"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["name"], "My Project");
    assert_eq!(body["description"], "desc");
    assert_eq!(body["group_id"], gid);
    assert!(body["id"].is_string());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_blocked_for_non_member() {
    db!(c, db);
    let (_, tok) = make_user(&db, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&db, "eng").await;

    let (status, _) = send(projects_app(db),
        req_post("/projects", &tok, json!({"name":"Sneaky","group_id":gid}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_returns_400_for_empty_name() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let (status, body) = send(projects_app(db),
        req_post("/projects", &tok, json!({"name":"   ","group_id":gid}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap_or("").contains("name"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_returns_404_for_unknown_group() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;

    let (status, _) = send(projects_app(db),
        req_post("/projects", &tok, json!({"name":"X","group_id":"nonexistent"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_projects_shows_only_own_group_projects() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let g_eng  = make_group(&db, "eng").await;
    let g_data = make_group(&db, "data").await;
    join_group(&db, &alice_id, &g_eng).await;
    join_group(&db, &bob_id,   &g_data).await;

    let app = projects_app(Arc::clone(&db));
    send(app.clone(), req_post("/projects", &alice_tok, json!({"name":"Eng Project","group_id":g_eng}))).await;
    send(app.clone(), req_post("/projects", &bob_tok,   json!({"name":"Data Project","group_id":g_data}))).await;

    let (status, body) = send(app, req_get("/projects", &alice_tok)).await;
    assert_eq!(status, StatusCode::OK);
    let projects = body.as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "Eng Project");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_project_returns_404_for_non_member() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, bob_tok)          = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, created) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"Secret","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app, req_get(&format!("/projects/{pid}"), &bob_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn update_project_succeeds_for_any_group_member() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;
    join_group(&db, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, created) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"Original","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(),
        req_put(&format!("/projects/{pid}"), &bob_tok, json!({"name":"Renamed"}))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app, req_get(&format!("/projects/{pid}"), &alice_tok)).await;
    assert_eq!(body["name"], "Renamed");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn delete_project_removes_it_for_group_member() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, created) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"Temp","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(), req_del(&format!("/projects/{pid}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(app, req_get(&format!("/projects/{pid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn admin_can_list_all_projects() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok)        = make_user(&db, "z@x.com", "Admin", "admin").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    send(app.clone(), req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;

    let (status, body) = send(app, req_get("/projects", &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn admin_can_access_project_in_any_group() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok)        = make_user(&db, "z@x.com", "Admin", "admin").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, created) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app, req_get(&format!("/projects/{pid}"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_project_conversations_returns_404_for_non_member() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, bob_tok)          = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, _) = send(app,
        req_get(&format!("/projects/{pid}/conversations"), &bob_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_conversation_visible_to_all_group_members() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;
    join_group(&db, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok,
                 json!({"title":"Team Chat"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conv["id"].as_str().unwrap().to_string();

    let (status, list) = send(app.clone(),
        req_get(&format!("/projects/{pid}/conversations"), &bob_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.as_array().unwrap().iter().any(|c| c["id"] == cid));

    let (status, detail) = send(app,
        req_get(&format!("/projects/{pid}/conversations/{cid}"), &bob_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["title"], "Team Chat");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn project_conversation_stores_created_by() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();

    let (_, list) = send(app,
        req_get(&format!("/projects/{pid}/conversations"), &alice_tok)).await;
    let found = list.as_array().unwrap().iter().find(|c| c["id"] == cid).unwrap().clone();
    assert_eq!(found["created_by"], alice_id);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn any_group_member_can_update_project_conversation() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;
    join_group(&db, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();

    let messages = json!([{"role":"user","content":"Hello team","username":"Bob"}]);
    let (status, _) = send(app.clone(),
        req_put(&format!("/projects/{pid}/conversations/{cid}"), &bob_tok,
                json!({"title":"Updated","messages":messages}))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, detail) = send(app,
        req_get(&format!("/projects/{pid}/conversations/{cid}"), &alice_tok)).await;
    assert_eq!(detail["title"], "Updated");
    let msgs = detail["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["username"], "Bob");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn only_creator_can_delete_project_conversation() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;
    join_group(&db, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(),
        req_del(&format!("/projects/{pid}/conversations/{cid}"), &bob_tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = send(app.clone(),
        req_del(&format!("/projects/{pid}/conversations/{cid}"), &alice_tok)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(app,
        req_get(&format!("/projects/{pid}/conversations/{cid}"), &alice_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn admin_can_delete_any_project_conversation() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok)        = make_user(&db, "z@x.com", "Admin", "admin").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();

    let (status, _) = send(app,
        req_del(&format!("/projects/{pid}/conversations/{cid}"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn project_query_returns_404_for_non_member() {
    db!(c, db);
    let (alice_id, alice_tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, bob_tok)          = make_user(&db, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, _) = send(app,
        req_post(&format!("/projects/{pid}/query"), &bob_tok, json!({"query":"hello"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn project_query_succeeds_for_group_member() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, body) = send(app,
        req_post(&format!("/projects/{pid}/query"), &tok,
                 json!({"query":"what is the meaning of life?"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answer"], "stub answer");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn second_turn_preserves_first_turns_sources_and_chain() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let llm = FixedTextLlm::new("see [myrepo:v1.0:src/lib.rs:42]");
    let app = projects_app_with_llm(Arc::clone(&db), llm);
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();
    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();
    let conv_uri = format!("/projects/{pid}/conversations/{cid}");

    let (status, _) = send(app.clone(),
        req_post(&format!("/projects/{pid}/query/stream"), &tok,
                 json!({"query": "first question", "conversation_id": cid}))).await;
    assert_eq!(status, StatusCode::OK);
    wait_for_message_count(app.clone(), &conv_uri, &tok, 2).await;

    let (status, _) = send(app.clone(),
        req_post(&format!("/projects/{pid}/query/stream"), &tok,
                 json!({"query": "second question", "conversation_id": cid}))).await;
    assert_eq!(status, StatusCode::OK);
    let conv = wait_for_message_count(app, &conv_uri, &tok, 4).await;

    let messages = conv["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    let first_assistant = &messages[1];
    assert_eq!(first_assistant["role"], "assistant");
    assert!(
        !first_assistant["sources"].as_array().unwrap().is_empty(),
        "first turn's sources must survive being an older turn: {first_assistant}"
    );
    assert_eq!(first_assistant["sources"][0]["file"], "src/lib.rs");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn tool_call_chain_persists_with_preview() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let llm = ScriptedLlm::new(vec![
        LlmResponse::ToolCalls {
            calls: vec![ToolCall {
                id: "tc1".into(), name: "list_agents".into(),
                input: json!({}), thought_signature: None,
            }],
            preamble: "Checking connected agents".into(),
        usage: Usage::default(),
        },
        LlmResponse::Message { text: "No agents are connected.".into(), usage: Usage::default() },
    ]);
    let app = projects_app_with_llm(Arc::clone(&db), llm);
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();
    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();
    let conv_uri = format!("/projects/{pid}/conversations/{cid}");

    let (status, _) = send(app.clone(),
        req_post(&format!("/projects/{pid}/query/stream"), &tok,
                 json!({"query": "any agents?", "conversation_id": cid}))).await;
    assert_eq!(status, StatusCode::OK);

    let conv = wait_for_message_count(app, &conv_uri, &tok, 2).await;
    let assistant = &conv["messages"].as_array().unwrap()[1];
    let chain = assistant["chain"].as_array().unwrap();
    let tool_call = chain.iter().find(|c| c["type"] == "tool_call")
        .expect("expected a tool_call chain entry");
    assert_eq!(tool_call["name"], "list_agents");
    assert_eq!(tool_call["status"], "done");
    assert!(!tool_call["preview"].is_null(), "preview must be captured: {tool_call}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn ask_user_question_persists_across_reload() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let llm = ScriptedLlm::new(vec![
        LlmResponse::ToolCalls {
            calls: vec![ToolCall {
                id: "tc1".into(), name: "ask_user".into(),
                input: json!({
                    "question": "Which environment?",
                    "choices": ["staging", "production"],
                }),
                thought_signature: None,
            }],
            preamble: String::new(),
        usage: Usage::default(),
        },
    ]);
    let app = projects_app_with_llm(Arc::clone(&db), llm);
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();
    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();
    let conv_uri = format!("/projects/{pid}/conversations/{cid}");

    let (status, _) = send(app.clone(),
        req_post(&format!("/projects/{pid}/query/stream"), &tok,
                 json!({"query": "deploy this", "conversation_id": cid}))).await;
    assert_eq!(status, StatusCode::OK);

    let conv = wait_for_message_count(app, &conv_uri, &tok, 2).await;
    let assistant = &conv["messages"].as_array().unwrap()[1];
    assert_eq!(assistant["question"]["question"], "Which environment?");
    assert_eq!(assistant["question"]["choices"], json!(["staging", "production"]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn confirm_action_persists_pending_then_resume_continues_the_turn() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let llm = ScriptedLlm::new(vec![
        LlmResponse::ToolCalls {
            calls: vec![ToolCall {
                id: "tc1".into(), name: "delete_agent".into(),
                input: json!({"agent_id": "bogus-agent"}),
                thought_signature: None,
            }],
            preamble: String::new(),
        usage: Usage::default(),
        },
        LlmResponse::Message { text: "Deleted bogus-agent as requested".into(), usage: Usage::default() },
    ]);
    let app = projects_app_with_llm(Arc::clone(&db), llm);
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();
    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();
    let conv_uri = format!("/projects/{pid}/conversations/{cid}");

    let (status, _) = send(app.clone(),
        req_post(&format!("/projects/{pid}/query/stream"), &tok,
                 json!({"query": "delete it", "conversation_id": cid}))).await;
    assert_eq!(status, StatusCode::OK);

    let conv = wait_for_message_count(app.clone(), &conv_uri, &tok, 2).await;
    let assistant = &conv["messages"].as_array().unwrap()[1];
    let chain = assistant["chain"].as_array().unwrap();
    let actions: Vec<&Value> = chain.iter().filter(|c| c["type"] == "confirm_action").collect();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["name"], "delete_agent");
    assert_eq!(actions[0]["input"]["agent_id"], "bogus-agent");
    assert_eq!(actions[0]["status"], "pending");
    let tool_call_id = actions[0]["id"].as_str().unwrap().to_string();

    let resume_uri = format!("/projects/{pid}/conversations/{cid}/confirm-action/resume");
    let (status, body) = send(app.clone(),
        req_post(&resume_uri, &tok, json!({"results": [
            {"tool_call_id": tool_call_id, "status": "done", "result_text": "Agent deleted."},
        ]}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resumed"], true);

    let conv = wait_for_message_text(app, &conv_uri, &tok, "Deleted bogus-agent as requested").await;
    let assistant = &conv["messages"].as_array().unwrap()[1];
    let chain = assistant["chain"].as_array().unwrap();
    let confirm_entry = chain.iter().find(|c| c["type"] == "confirm_action").unwrap();
    assert_eq!(confirm_entry["status"], "done");
    assert_eq!(confirm_entry["result_text"], "Agent deleted.");
    assert_eq!(assistant["text"], "Deleted bogus-agent as requested");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn resume_confirm_action_404s_without_pending_action() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;

    let app = projects_app(Arc::clone(&db));
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();
    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();
    let conv_uri = format!("/projects/{pid}/conversations/{cid}");

    let (status, _) = send(app.clone(),
        req_post(&format!("/projects/{pid}/query/stream"), &tok,
                 json!({"query": "hello", "conversation_id": cid}))).await;
    assert_eq!(status, StatusCode::OK);
    wait_for_message_count(app.clone(), &conv_uri, &tok, 2).await;

    let (status, _) = send(app,
        req_post(&format!("/projects/{pid}/conversations/{cid}/confirm-action/resume"), &tok,
                  json!({"results": [{"tool_call_id": "nope", "status": "done", "result_text": "x"}]}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
