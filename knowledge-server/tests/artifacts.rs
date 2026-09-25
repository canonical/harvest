use harvest_db::Db;
use std::sync::Arc;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{get as route_get, post as route_post},
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
    artifacts::handlers::{self as artifact_handlers, ArtifactState},
    auth::{self, jwt},
    llm::{
        LlmProvider,
        types::{LlmResponse, Message, ModelInfo, ToolDefinition, Usage},
    },
    machines::MachineRegistry,

    projects::handlers::{create_artifact_route, create_project, list_artifacts, ProjectState},
};

use async_trait::async_trait;
use anyhow::Result;

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

const JWT_SECRET: &str = "test-artifacts-secret";

fn artifacts_app(db: Arc<Db>) -> Router {
    let secret   = Arc::new(JWT_SECRET.to_string());
    let llm: Arc<dyn LlmProvider> = FixedTextLlm::new("stub");
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        db:                      Arc::clone(&db),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::new(knowledge_server::skills::SkillStore::new(Arc::clone(&db))),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let project_state  = Arc::new(ProjectState::new(Arc::clone(&db), agent, builder, Arc::clone(&llm) as Arc<dyn LlmProvider>, Arc::new(vec![]), None, Arc::new(knowledge_server::cost::PricingTable::default())));
    let artifact_state = Arc::new(ArtifactState { db: Arc::clone(&db) });

    let project_router = Router::new()
        .route("/projects",                    route_post(create_project))
        .route("/projects/:pid/artifacts",     route_get(list_artifacts).post(create_artifact_route))
        .with_state(project_state);

    let artifact_router = Router::new()
        .route("/artifacts/:id",          route_get(artifact_handlers::get_artifact).delete(artifact_handlers::delete_artifact))
        .route("/artifacts/:id/download", route_get(artifact_handlers::download_artifact))
        .with_state(artifact_state);

    project_router
        .merge(artifact_router)
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

struct RawResponse {
    status:       StatusCode,
    content_type: Option<String>,
    disposition:  Option<String>,
    bytes:        Vec<u8>,
}

async fn send_raw(app: Router, req: Request<Body>) -> RawResponse {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let content_type = resp.headers().get("content-type")
        .map(|v| v.to_str().unwrap().to_string());
    let disposition = resp.headers().get("content-disposition")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    RawResponse { status, content_type, disposition, bytes }
}

macro_rules! db {
    ($c:ident, $db:ident) => {
        let $c = TestDb::new().await;
        let $db = Arc::new($c.db.clone());
    };
}

async fn seed_project(app: &Router, token: &str, group_id: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_post("/projects", token, json!({"name":"Test Project","group_id":group_id})),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_artifacts_empty_for_new_project() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/artifacts"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_markdown_artifact_returns_201_with_fields() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "Deploy report",
            "kind": "markdown",
            "content": "# Deploy report\n\nAll good."
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());
    assert_eq!(body["title"], "Deploy report");
    assert_eq!(body["kind"], "markdown");
    assert!(body["created_at"].is_string());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_artifact_rejects_invalid_kind() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "Bad kind",
            "kind": "docx",
            "content": "whatever"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_artifact_requires_title() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "",
            "kind": "markdown",
            "content": "whatever"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_artifact_by_id_returns_full_content() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let content = "# Report\n\nDetails here.";
    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "Report", "kind": "pdf", "content": content
        })),
    ).await;
    let aid = create_body["id"].as_str().unwrap();

    let (status, body) = send(app, req_get(&format!("/artifacts/{aid}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], aid);
    assert_eq!(body["title"], "Report");
    assert_eq!(body["kind"], "pdf");
    assert_eq!(body["content"], content);
    assert_eq!(body["project_id"], pid);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn non_member_cannot_access_artifact() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, outsider_tok) = make_user(&db, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "Secret", "kind": "markdown", "content": "shh"
        })),
    ).await;
    let aid = create_body["id"].as_str().unwrap();

    let (status, _) = send(app, req_get(&format!("/artifacts/{aid}"), &outsider_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn admin_can_access_any_artifact() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "Report", "kind": "markdown", "content": "content"
        })),
    ).await;
    let aid = create_body["id"].as_str().unwrap();

    let (status, body) = send(app, req_get(&format!("/artifacts/{aid}"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], aid);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn download_markdown_artifact_returns_text_with_attachment_header() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let content = "# Hello\n\nWorld.";
    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "My Report!", "kind": "markdown", "content": content
        })),
    ).await;
    let aid = create_body["id"].as_str().unwrap();

    let resp = send_raw(app, req_get(&format!("/artifacts/{aid}/download"), &tok)).await;
    assert_eq!(resp.status, StatusCode::OK);
    assert!(resp.content_type.unwrap().starts_with("text/markdown"));
    let disposition = resp.disposition.unwrap();
    assert!(disposition.contains("attachment"));
    assert!(disposition.ends_with(".md\""), "disposition was: {disposition}");
    assert_eq!(String::from_utf8(resp.bytes).unwrap(), content);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn download_pdf_artifact_returns_pdf_bytes() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "Report", "kind": "pdf", "content": "# Report\n\nSome content."
        })),
    ).await;
    let aid = create_body["id"].as_str().unwrap();

    let resp = send_raw(app, req_get(&format!("/artifacts/{aid}/download"), &tok)).await;
    assert_eq!(resp.status, StatusCode::OK);
    assert_eq!(resp.content_type.unwrap(), "application/pdf");
    let disposition = resp.disposition.unwrap();
    assert!(disposition.ends_with(".pdf\""), "disposition was: {disposition}");
    assert!(resp.bytes.starts_with(b"%PDF-"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn delete_artifact_removes_it() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "To delete", "kind": "markdown", "content": "bye"
        })),
    ).await;
    let aid = create_body["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_del(&format!("/artifacts/{aid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/artifacts/{aid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_artifacts_after_create_returns_summary_without_content() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = artifacts_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid).await;

    send(
        app.clone(),
        req_post(&format!("/projects/{pid}/artifacts"), &tok, json!({
            "title": "First", "kind": "markdown", "content": "Some content"
        })),
    ).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/artifacts"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "First");
    assert_eq!(arr[0]["kind"], "markdown");
    assert!(arr[0].get("content").is_none(), "list should not include content");
}
