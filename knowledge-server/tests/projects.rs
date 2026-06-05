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
        create_conversation, create_project, delete_conversation, delete_project,
        get_conversation, get_project, list_conversations, list_projects,
        project_query, project_query_stream, update_conversation, update_project,
    },
};

// ── stub LLM ──────────────────────────────────────────────────────────────────

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

const JWT_SECRET: &str = "test-projects-secret";

// ── app builder ───────────────────────────────────────────────────────────────

fn projects_app(neo4j: Arc<Neo4jClient>) -> Router {
    let secret   = Arc::new(JWT_SECRET.to_string());
    let llm: Arc<dyn knowledge_server::llm::LlmProvider> = FixedTextLlm::new("stub answer");
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        neo4j:                      Arc::clone(&neo4j),
        registry:                   Arc::clone(&registry),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let state = Arc::new(ProjectState::new(neo4j, agent, builder));

    Router::new()
        .route("/projects",     route_get(list_projects).post(create_project))
        .route("/projects/:pid",
               route_get(get_project).put(update_project).delete(delete_project))
        .route("/projects/:pid/conversations",
               route_get(list_conversations).post(create_conversation))
        .route("/projects/:pid/conversations/:cid",
               route_get(get_conversation).put(update_conversation).delete(delete_conversation))
        .route("/projects/:pid/query",        route_post(project_query))
        .route("/projects/:pid/query/stream", route_post(project_query_stream))
        .with_state(state)
        .layer(from_fn_with_state(secret, auth::require_auth))
}

// ── seed helpers ──────────────────────────────────────────────────────────────

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT project_id IF NOT EXISTS FOR (p:Project) REQUIRE p.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT conversation_id IF NOT EXISTS FOR (c:Conversation) REQUIRE c.id IS UNIQUE").await.unwrap();
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

// ── request builders ──────────────────────────────────────────────────────────

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

// ── auth guard tests (no Docker needed) ──────────────────────────────────────

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

// ── Docker integration tests ──────────────────────────────────────────────────

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

// ── project CRUD ──────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_projects_empty_for_new_user() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (status, body) = send(projects_app(neo4j), req_get("/projects", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_succeeds_for_group_member() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;

    let (status, body) = send(projects_app(neo4j),
        req_post("/projects", &tok, json!({"name":"My Project","group_id":gid,"description":"desc"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["name"], "My Project");
    assert_eq!(body["description"], "desc");
    assert_eq!(body["group_id"], gid);
    assert!(body["id"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_blocked_for_non_member() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&neo4j, "eng").await;

    let (status, _) = send(projects_app(neo4j),
        req_post("/projects", &tok, json!({"name":"Sneaky","group_id":gid}))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_returns_400_for_empty_name() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;

    let (status, body) = send(projects_app(neo4j),
        req_post("/projects", &tok, json!({"name":"   ","group_id":gid}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap_or("").contains("name"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_returns_404_for_unknown_group() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;

    let (status, _) = send(projects_app(neo4j),
        req_post("/projects", &tok, json!({"name":"X","group_id":"nonexistent"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_projects_shows_only_own_group_projects() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let g_eng  = make_group(&neo4j, "eng").await;
    let g_data = make_group(&neo4j, "data").await;
    join_group(&neo4j, &alice_id, &g_eng).await;
    join_group(&neo4j, &bob_id,   &g_data).await;

    let app = projects_app(Arc::clone(&neo4j));
    send(app.clone(), req_post("/projects", &alice_tok, json!({"name":"Eng Project","group_id":g_eng}))).await;
    send(app.clone(), req_post("/projects", &bob_tok,   json!({"name":"Data Project","group_id":g_data}))).await;

    let (status, body) = send(app, req_get("/projects", &alice_tok)).await;
    assert_eq!(status, StatusCode::OK);
    let projects = body.as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["name"], "Eng Project");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_project_returns_404_for_non_member() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, bob_tok)          = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, created) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"Secret","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app, req_get(&format!("/projects/{pid}"), &bob_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_project_succeeds_for_any_group_member() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;
    join_group(&neo4j, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
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
#[ignore = "requires Docker"]
async fn delete_project_removes_it_for_group_member() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, created) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"Temp","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(), req_del(&format!("/projects/{pid}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(app, req_get(&format!("/projects/{pid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn admin_can_list_all_projects() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok)        = make_user(&neo4j, "z@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    send(app.clone(), req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;

    let (status, body) = send(app, req_get("/projects", &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!body.as_array().unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn admin_can_access_project_in_any_group() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok)        = make_user(&neo4j, "z@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, created) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = created["id"].as_str().unwrap().to_string();

    let (status, _) = send(app, req_get(&format!("/projects/{pid}"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
}

// ── project conversations ─────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_project_conversations_returns_404_for_non_member() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, bob_tok)          = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, _) = send(app,
        req_get(&format!("/projects/{pid}/conversations"), &bob_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_conversation_visible_to_all_group_members() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;
    join_group(&neo4j, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok,
                 json!({"title":"Team Chat"}))).await;
    assert_eq!(status, StatusCode::CREATED);
    let cid = conv["id"].as_str().unwrap().to_string();

    // Bob can list it
    let (status, list) = send(app.clone(),
        req_get(&format!("/projects/{pid}/conversations"), &bob_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.as_array().unwrap().iter().any(|c| c["id"] == cid));

    // Bob can fetch it directly
    let (status, detail) = send(app,
        req_get(&format!("/projects/{pid}/conversations/{cid}"), &bob_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["title"], "Team Chat");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn project_conversation_stores_created_by() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
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
#[ignore = "requires Docker"]
async fn any_group_member_can_update_project_conversation() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;
    join_group(&neo4j, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
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
#[ignore = "requires Docker"]
async fn only_creator_can_delete_project_conversation() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (bob_id,   bob_tok)   = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;
    join_group(&neo4j, &bob_id,   &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (_, conv) = send(app.clone(),
        req_post(&format!("/projects/{pid}/conversations"), &alice_tok, json!({}))).await;
    let cid = conv["id"].as_str().unwrap().to_string();

    // Bob (not creator) is rejected
    let (status, _) = send(app.clone(),
        req_del(&format!("/projects/{pid}/conversations/{cid}"), &bob_tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Alice (creator) succeeds
    let (status, _) = send(app.clone(),
        req_del(&format!("/projects/{pid}/conversations/{cid}"), &alice_tok)).await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(app,
        req_get(&format!("/projects/{pid}/conversations/{cid}"), &alice_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn admin_can_delete_any_project_conversation() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok)        = make_user(&neo4j, "z@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
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

// ── project query ─────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires Docker"]
async fn project_query_returns_404_for_non_member() {
    neo4j!(c, neo4j);
    let (alice_id, alice_tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, bob_tok)          = make_user(&neo4j, "b@x.com", "Bob",   "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &alice_id, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, project) = send(app.clone(),
        req_post("/projects", &alice_tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, _) = send(app,
        req_post(&format!("/projects/{pid}/query"), &bob_tok, json!({"query":"hello"}))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn project_query_succeeds_for_group_member() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;

    let app = projects_app(Arc::clone(&neo4j));
    let (_, project) = send(app.clone(),
        req_post("/projects", &tok, json!({"name":"P","group_id":gid}))).await;
    let pid = project["id"].as_str().unwrap().to_string();

    let (status, body) = send(app,
        req_post(&format!("/projects/{pid}/query"), &tok,
                 json!({"query":"what is the meaning of life?"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["answer"], "stub answer");
}
