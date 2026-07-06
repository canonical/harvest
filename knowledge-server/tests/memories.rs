use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{get as route_get, post as route_post},
    Router,
};
use http_body_util::BodyExt as _;
use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j};
use serde_json::{json, Value};
use tower::ServiceExt as _;
use uuid::Uuid;

use tokio::sync::Mutex;

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
    projects::{
        handlers::{
            ProjectState,
            create_memory, delete_memory, get_memory, list_memories, update_memory,
            create_project,
        },
        memory_gen::maybe_generate_memory,
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

const JWT_SECRET: &str = "test-memories-secret";

fn memories_app(neo4j: Arc<Neo4jClient>) -> Router {
    let secret   = Arc::new(JWT_SECRET.to_string());
    let llm: Arc<dyn knowledge_server::llm::LlmProvider> = FixedTextLlm::new("stub");
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        neo4j:                      Arc::clone(&neo4j),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::new(knowledge_server::skills::SkillStore::new(Arc::clone(&neo4j))),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let state = Arc::new(ProjectState::new(neo4j, agent, builder));

    Router::new()
        .route("/projects",                          route_post(create_project))
        .route("/projects/:pid/memories",            route_get(list_memories).post(create_memory))
        .route("/projects/:pid/memories/:mid",
               route_get(get_memory).put(update_memory).delete(delete_memory))
        .with_state(state)
        .layer(from_fn_with_state(secret, auth::require_auth))
}

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT project_id IF NOT EXISTS FOR (p:Project) REQUIRE p.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT memory_id IF NOT EXISTS FOR (m:Memory) REQUIRE m.id IS UNIQUE").await.unwrap();
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

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp   = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes  = resp.into_body().collect().await.unwrap().to_bytes();
    let json   = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
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

async fn seed_project(_neo4j: &Arc<Neo4jClient>, app: &Router, token: &str, group_id: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_post("/projects", token, json!({"name":"Test Project","group_id":group_id})),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_memories_empty_for_new_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/memories"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_memory_returns_201_with_fields() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/memories"), &tok, json!({
            "title": "Deploy issue fixed",
            "content": "## 2026-06-07\n\nThe deploy failed due to missing env var. Fixed by adding SECRET_KEY."
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());
    assert_eq!(body["title"], "Deploy issue fixed");
    assert!(body["created_at"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_memory_returns_full_content() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let content = "## Memory content\n\nSome detailed notes here.";
    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/memories"), &tok, json!({
            "title": "Test memory",
            "content": content
        })),
    ).await;
    let mid = create_body["id"].as_str().unwrap();

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/memories/{mid}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], mid);
    assert_eq!(body["title"], "Test memory");
    assert_eq!(body["content"], content);
    assert!(body["created_at"].is_string());
    assert!(body["updated_at"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_memories_after_create_returns_summary() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    send(
        app.clone(),
        req_post(&format!("/projects/{pid}/memories"), &tok, json!({
            "title": "First memory",
            "content": "Content here"
        })),
    ).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/memories"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["title"], "First memory");
    assert!(arr[0]["id"].is_string());
    assert!(arr[0]["created_at"].is_string());
    assert!(arr[0].get("content").is_none(), "list should not include content");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_memory_changes_title_and_content() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/memories"), &tok, json!({
            "title": "Original title",
            "content": "Original content"
        })),
    ).await;
    let mid = create_body["id"].as_str().unwrap();

    let (status, _) = send(
        app.clone(),
        req_put(&format!("/projects/{pid}/memories/{mid}"), &tok, json!({
            "title": "Updated title",
            "content": "Updated content"
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app, req_get(&format!("/projects/{pid}/memories/{mid}"), &tok)).await;
    assert_eq!(body["title"], "Updated title");
    assert_eq!(body["content"], "Updated content");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_memory_removes_it() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (_, create_body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/memories"), &tok, json!({
            "title": "To be deleted",
            "content": "Some content"
        })),
    ).await;
    let mid = create_body["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_del(&format!("/projects/{pid}/memories/{mid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/memories/{mid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn non_member_cannot_access_memories() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, outsider_tok) = make_user(&neo4j, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/memories"), &outsider_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_memory_requires_title() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/memories"), &tok, json!({
            "title": "",
            "content": "Some content"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn admin_can_access_any_project_memories() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/memories"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

struct JsonLlm(Value);
impl JsonLlm {
    fn new(v: Value) -> Arc<Self> { Arc::new(Self(v)) }
}
#[async_trait]
impl LlmProvider for JsonLlm {
    async fn chat(&self, _: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.to_string() })
    }
}

struct CapturingLlm {
    response: Value,
    prompts:  Arc<Mutex<Vec<String>>>,
}
impl CapturingLlm {
    fn new(response: Value) -> (Arc<Self>, Arc<Mutex<Vec<String>>>) {
        let prompts = Arc::new(Mutex::new(vec![]));
        (Arc::new(Self { response, prompts: Arc::clone(&prompts) }), prompts)
    }
}
#[async_trait]
impl LlmProvider for CapturingLlm {
    async fn chat(&self, messages: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        use knowledge_server::llm::types::MessageContent;
        let system_text = messages.iter().find_map(|m| {
            if let MessageContent::Text(t) = &m.content { Some(t.clone()) } else { None }
        }).unwrap_or_default();
        self.prompts.lock().await.push(system_text);
        Ok(LlmResponse::Message { text: self.response.to_string() })
    }
}

async fn count_memories(neo4j: &Arc<Neo4jClient>, project_id: &str) -> usize {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_MEMORY]->(m:Memory) RETURN count(m) AS n",
        json!({ "pid": project_id }),
    ).await.unwrap_or_default();
    rows.first().and_then(|r| r["n"].as_u64()).unwrap_or(0) as usize
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn auto_generate_creates_memory_when_llm_says_yes() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let llm = JsonLlm::new(json!({
        "create": true,
        "title": "Deploy issue fixed",
        "content": "## 2026-06-07\n\nFixed deploy by adding missing env var."
    }));

    maybe_generate_memory(&neo4j, &*llm, &pid, "How to fix the deploy?", "Add the SECRET_KEY env var.").await;

    assert_eq!(count_memories(&neo4j, &pid).await, 1);

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/memories"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr[0]["title"], "Deploy issue fixed");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn auto_generate_skips_when_llm_says_no() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let llm = JsonLlm::new(json!({ "create": false }));

    maybe_generate_memory(&neo4j, &*llm, &pid, "What is 2 + 2?", "4.").await;

    assert_eq!(count_memories(&neo4j, &pid).await, 0);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn auto_generate_skips_on_malformed_llm_response() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let llm = FixedTextLlm::new("this is not JSON at all");

    maybe_generate_memory(&neo4j, &*llm, &pid, "Any question", "Any answer.").await;

    assert_eq!(count_memories(&neo4j, &pid).await, 0);
    let _ = tok;
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn auto_generate_passes_existing_memories_to_llm() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let mid = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (p:Project {id:$pid})
         CREATE (m:Memory {id:$id,title:$title,content:$content,created_by:'system',created_at:$now,updated_at:$now})
         CREATE (p)-[:HAS_MEMORY]->(m)",
        json!({"pid":&pid,"id":mid,"title":"Known issue: auth timeout","content":"Auth times out after 30s.","now":now}),
    ).await.unwrap();

    let (llm, captured_prompts) = CapturingLlm::new(json!({ "create": false }));

    maybe_generate_memory(&neo4j, &*llm, &pid, "Why does auth timeout?", "It's a known issue.").await;

    let prompts = captured_prompts.lock().await;
    assert!(!prompts.is_empty(), "LLM should have been called");
    assert!(
        prompts[0].contains("Known issue: auth timeout"),
        "existing memory title should appear in LLM prompt"
    );
    assert_eq!(count_memories(&neo4j, &pid).await, 1);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn auto_generate_does_not_create_duplicate_when_topic_covered() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = memories_app(Arc::clone(&neo4j));
    let pid = seed_project(&neo4j, &app, &tok, &gid).await;

    let llm_yes = JsonLlm::new(json!({
        "create": true,
        "title": "Auth timeout root cause",
        "content": "## 2026-06-07\n\nAuth times out because the session store is unreachable."
    }));
    maybe_generate_memory(&neo4j, &*llm_yes, &pid, "Why auth timeout?", "Session store unreachable.").await;
    assert_eq!(count_memories(&neo4j, &pid).await, 1);

    let llm_no = JsonLlm::new(json!({ "create": false }));
    maybe_generate_memory(&neo4j, &*llm_no, &pid, "Auth still timing out?", "Same root cause.").await;
    assert_eq!(count_memories(&neo4j, &pid).await, 1);

    let _ = tok;
}
