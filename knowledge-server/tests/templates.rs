use harvest_db::Db;
use std::io::{Read, Write};
use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{get as route_get, post as route_post},
    Router,
};
use harvest_db::test_support::TestDb;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use knowledge_server::{
    agent::Agent,
    api::ProjectAgentBuilder,
    auth::{self, jwt},
    deployments::handlers::{
        create_template, delete_template, get_template, list_templates, upload_template,
    },
    llm::{LlmProvider, types::{LlmResponse, Message, ModelInfo, ToolDefinition, Usage}},
    machines::MachineRegistry,

    projects::handlers::ProjectState,
    skills::SkillStore,
};

const JWT_SECRET: &str = "test-templates-secret";

struct FixedTextLlm(String);
impl FixedTextLlm {
    fn new(t: impl Into<String>) -> Arc<dyn LlmProvider> { Arc::new(Self(t.into())) }
}
#[async_trait]
impl LlmProvider for FixedTextLlm {
    fn id(&self) -> &str { "fixed-text" }
    fn kind(&self) -> &str { "mock" }
    fn default_model(&self) -> &str { "mock-model" }
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> { Ok(vec![]) }
    async fn chat_with(&self, _model: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone(), usage: Usage::default() })
    }
}

fn cookie(token: &str) -> String { format!("token={token}") }

macro_rules! db {
    ($c:ident, $db:ident) => {
        let $c = TestDb::new().await;
        let $db = Arc::new($c.db.clone());
    };
}

async fn make_user(db: &Db, email: &str, name: &str, role: &str) -> (String, String) {
    let id  = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.query(
        "INSERT INTO users (id, email, name, role, provider, created_at)
         VALUES ($id, $email, $name, $role, 'password', $now)",
        json!({"id":id,"email":email,"name":name,"role":role,"now":now}),
    ).await.unwrap();
    let token = jwt::issue(JWT_SECRET, &id, email, name, role).unwrap();
    (id, token)
}

fn templates_app(db: Arc<Db>) -> Router {
    let secret      = Arc::new(JWT_SECRET.to_string());
    let skill_store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let llm = FixedTextLlm::new("stub");
    let agent = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder = Arc::new(ProjectAgentBuilder {
        llm: Arc::clone(&llm),
        db: Arc::clone(&db),
        registry: Arc::clone(&registry),
        skills: Arc::clone(&skill_store),
        lxd: None,
        server_url: "http://localhost".into(),
        max_iterations: 5,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last: 6,
    });
    let project_state = Arc::new(ProjectState::new(Arc::clone(&db), agent, builder, Arc::clone(&llm) as Arc<dyn LlmProvider>, Arc::new(vec![]), None, Arc::new(knowledge_server::cost::PricingTable::default())));
    Router::new()
        .route("/templates", route_get(list_templates).post(create_template))
        .route("/templates/upload", route_post(upload_template))
        .route("/templates/:tid", route_get(get_template).delete(delete_template))
        .with_state(project_state)
        .layer(from_fn_with_state(Arc::clone(&secret), auth::require_auth))
}

fn req_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri)
        .header("Cookie", cookie(token)).body(Body::empty()).unwrap()
}

fn req_post_json(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder().method("POST").uri(uri)
        .header("Cookie", cookie(token))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap()
}

fn req_post_multipart(uri: &str, token: &str, boundary: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder().method("POST").uri(uri)
        .header("Cookie", cookie(token))
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body)).unwrap()
}

fn req_delete(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("DELETE").uri(uri)
        .header("Cookie", cookie(token)).body(Body::empty()).unwrap()
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp   = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes  = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
    let json   = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn build_harvest_zip() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();

        zip.start_file("metadata.yaml", opts).unwrap();
        write!(zip, "name: Charmed Juju\ndescription: Deploy Juju-based products").unwrap();

        zip.start_file("design.md", opts).unwrap();
        write!(zip, "# 1. Introduction\n${{CUSTOMER}}").unwrap();

        zip.start_file("skills/juju.md", opts).unwrap();
        write!(zip, "---\nname: juju\ndescription: Deploy with Juju\n---\n# Juju\nJuju is an operator framework.").unwrap();

        zip.start_file("artifacts/main.tf", opts).unwrap();
        write!(zip, "resource \"null_resource\" \"x\" {{}}").unwrap();

        zip.finish().unwrap();
    }
    buf
}

fn build_harvest_zip_no_metadata() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("design.md", opts).unwrap();
        write!(zip, "# Introduction").unwrap();
        zip.start_file("skills/juju.md", opts).unwrap();
        write!(zip, "---\nname: juju\ndescription: test\n---\nbody").unwrap();
        zip.finish().unwrap();
    }
    buf
}

fn build_harvest_zip_no_design() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("metadata.yaml", opts).unwrap();
        write!(zip, "name: Charmed Juju\ndescription: test").unwrap();
        zip.start_file("skills/juju.md", opts).unwrap();
        write!(zip, "---\nname: juju\ndescription: test\n---\nbody").unwrap();
        zip.finish().unwrap();
    }
    buf
}

fn multipart_body(boundary: &str, filename: &str, content: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    write!(body, "--{boundary}\r\n").unwrap();
    write!(body, "Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n").unwrap();
    write!(body, "Content-Type: application/octet-stream\r\n\r\n").unwrap();
    body.extend_from_slice(content);
    write!(body, "\r\n--{boundary}--\r\n").unwrap();
    body
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_templates_returns_all_templates_globally() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&db));

    let _ = send(app.clone(), req_post_json("/templates", &tok, json!({
        "name": "Template A", "description": "first", "content": "{}"
    }))).await;
    let _ = send(app.clone(), req_post_json("/templates", &tok, json!({
        "name": "Template B", "description": "second", "content": "{}"
    }))).await;

    let (status, list) = send(app, req_get("/templates", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upload_harvest_creates_template_with_skills_and_artifacts() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&db));

    let zip_bytes = build_harvest_zip();
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "test.harvest", &zip_bytes);

    let (status, body) = send(app.clone(), req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());
    assert_eq!(body["name"], "Charmed Juju");

    let template_id = body["id"].as_str().unwrap().to_string();
    let (status, detail) = send(app, req_get(&format!("/templates/{template_id}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["description"], "Deploy Juju-based products");
    let content: Value = serde_json::from_str(detail["content"].as_str().unwrap()).unwrap();
    assert_eq!(content["design_template"], "# 1. Introduction\n${CUSTOMER}");
    assert!(content["skills"].is_array());
    assert_eq!(content["skills"][0]["name"], "juju");
    assert_eq!(content["skills"][0]["description"], "Deploy with Juju");
    assert!(content["artifacts"].is_array());
    assert_eq!(content["artifacts"][0]["name"], "main");
    assert_eq!(content["artifacts"][0]["kind"], "terraform");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upload_harvest_rejects_missing_metadata_yaml() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&db));

    let zip_bytes = build_harvest_zip_no_metadata();
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "bad.harvest", &zip_bytes);

    let (status, _) = send(app, req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upload_harvest_rejects_missing_design_md() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&db));

    let zip_bytes = build_harvest_zip_no_design();
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "bad.harvest", &zip_bytes);

    let (status, _) = send(app, req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upload_harvest_rejects_non_zip_file() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&db));

    let not_zip = b"this is not a zip file at all";
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "fake.harvest", not_zip);

    let (status, _) = send(app, req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn delete_template_removes_it() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&db));

    let (_, body) = send(app.clone(), req_post_json("/templates", &tok, json!({
        "name": "To Delete", "description": "", "content": "{}"
    }))).await;
    let tid = body["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(), req_delete(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}