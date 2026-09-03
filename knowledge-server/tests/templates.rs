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
use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j};
use serde_json::{json, Value};
use tower::ServiceExt as _;

use knowledge_server::{
    agent::Agent,
    api::ProjectAgentBuilder,
    auth::{self, jwt},
    deployments::handlers::{
        create_template, delete_template, get_template, list_templates, upload_template,
    },
    llm::{LlmProvider, types::{LlmResponse, Message, ModelInfo, ToolDefinition}},
    machines::MachineRegistry,
    neo4j::Neo4jClient,
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
        Ok(LlmResponse::Message { text: self.0.clone() })
    }
}

fn cookie(token: &str) -> String { format!("token={token}") }

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT template_id IF NOT EXISTS FOR (t:ProductTemplate) REQUIRE t.id IS UNIQUE").await.unwrap();
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

async fn make_user(neo4j: &Neo4jClient, email: &str, name: &str, role: &str) -> (String, String) {
    let id  = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "CREATE (:User {id:$id,email:$email,name:$name,role:$role,provider:'password',created_at:$now}) RETURN 1",
        json!({"id":id,"email":email,"name":name,"role":role,"now":now}),
    ).await.unwrap();
    let token = jwt::issue(JWT_SECRET, &id, email, name, role).unwrap();
    (id, token)
}

fn templates_app(neo4j: Arc<Neo4jClient>) -> Router {
    let secret      = Arc::new(JWT_SECRET.to_string());
    let skill_store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let llm = FixedTextLlm::new("stub");
    let agent = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder = Arc::new(ProjectAgentBuilder {
        llm: Arc::clone(&llm),
        neo4j: Arc::clone(&neo4j),
        registry: Arc::clone(&registry),
        skills: Arc::clone(&skill_store),
        lxd: None,
        server_url: "http://localhost".into(),
        max_iterations: 5,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last: 6,
    });
    let project_state = Arc::new(ProjectState::new(Arc::clone(&neo4j), agent, builder));
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
#[ignore = "requires Docker"]
async fn list_templates_returns_all_templates_globally() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&neo4j));

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
#[ignore = "requires Docker"]
async fn upload_harvest_creates_template_with_skills_and_artifacts() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&neo4j));

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
#[ignore = "requires Docker"]
async fn upload_harvest_rejects_missing_metadata_yaml() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&neo4j));

    let zip_bytes = build_harvest_zip_no_metadata();
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "bad.harvest", &zip_bytes);

    let (status, _) = send(app, req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn upload_harvest_rejects_missing_design_md() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&neo4j));

    let zip_bytes = build_harvest_zip_no_design();
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "bad.harvest", &zip_bytes);

    let (status, _) = send(app, req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn upload_harvest_rejects_non_zip_file() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&neo4j));

    let not_zip = b"this is not a zip file at all";
    let boundary = "----testboundary";
    let body = multipart_body(boundary, "fake.harvest", not_zip);

    let (status, _) = send(app, req_post_multipart("/templates/upload", &tok, boundary, body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_template_removes_it() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = templates_app(Arc::clone(&neo4j));

    let (_, body) = send(app.clone(), req_post_json("/templates", &tok, json!({
        "name": "To Delete", "description": "", "content": "{}"
    }))).await;
    let tid = body["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(), req_delete(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}