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
    overview::handlers::{get_overview, overview_events, regenerate_overview, OverviewState},
};

// ── stub LLM ──────────────────────────────────────────────────────────────────

struct FixedTextLlm(String);
#[async_trait]
impl LlmProvider for FixedTextLlm {
    async fn chat(&self, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone() })
    }
}

fn stub_llm(text: impl Into<String>) -> Arc<dyn LlmProvider> {
    Arc::new(FixedTextLlm(text.into()))
}

const JWT_SECRET: &str = "test-overview-secret";

// ── app builder ───────────────────────────────────────────────────────────────

fn overview_app(neo4j: Arc<Neo4jClient>, llm: Arc<dyn LlmProvider>) -> Router {
    let secret   = Arc::new(JWT_SECRET.to_string());
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        neo4j:                      Arc::clone(&neo4j),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::new(knowledge_server::skills::SkillRegistry::new()),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let agent = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let state = Arc::new(OverviewState {
        neo4j:         Arc::clone(&neo4j),
        llm:           Arc::clone(&llm),
        agent_builder: Arc::clone(&builder),
        agent:         Arc::clone(&agent),
        generating:    Arc::new(dashmap::DashMap::new()),
    });

    Router::new()
        .route("/projects/:pid/overview",            route_get(get_overview))
        .route("/projects/:pid/overview/events",     route_get(overview_events))
        .route("/projects/:pid/overview/regenerate", route_post(regenerate_overview))
        .with_state(state)
        .layer(from_fn_with_state(secret, auth::require_auth))
}

// ── seed helpers ──────────────────────────────────────────────────────────────

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT project_id    IF NOT EXISTS FOR (p:Project)      REQUIRE p.id IS UNIQUE").await.unwrap();
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

async fn make_project(neo4j: &Neo4jClient, group_id: &str, user_id: &str, name: &str) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (g:Group {id:$gid})
         CREATE (p:Project {id:$id,name:$name,description:'',\
                             group_id:$gid,created_by:$uid,created_at:$now,\
                             install_token:$tok})
         CREATE (g)-[:HAS_PROJECT]->(p) RETURN 1",
        json!({"gid":group_id,"id":id,"name":name,"uid":user_id,
               "now":now,"tok":Uuid::new_v4().to_string()}),
    ).await.unwrap();
    id
}

async fn join_group(neo4j: &Neo4jClient, user_id: &str, group_id: &str) {
    neo4j.query_read(
        "MATCH (u:User{id:$uid}),(g:Group{id:$gid}) MERGE (u)-[:MEMBER_OF]->(g) RETURN 1",
        json!({"uid":user_id,"gid":group_id}),
    ).await.unwrap();
}

async fn add_conversation(neo4j: &Neo4jClient, project_id: &str) {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let msgs = serde_json::to_string(&json!([
        {"role":"user","text":"What services are running?","attachments":[]},
        {"role":"assistant","text":"The nginx service is running on port 80.","sources":[],"tool_calls":[],"tool_calls_made":0},
    ])).unwrap();
    neo4j.query_read(
        "MATCH (p:Project {id:$pid})
         CREATE (c:Conversation {id:$id,title:'Services',messages:$msgs,
                                  message_count:2,created_at:$now,updated_at:$now})
         CREATE (p)-[:HAS_CONVERSATION]->(c) RETURN 1",
        json!({"pid":project_id,"id":id,"msgs":msgs,"now":now}),
    ).await.unwrap();
}

// ── request helpers ───────────────────────────────────────────────────────────

fn cookie(token: &str) -> String { format!("token={token}") }

fn req_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("GET").uri(uri)
        .header("Cookie", cookie(token)).body(Body::empty()).unwrap()
}

fn req_post(uri: &str, token: &str) -> Request<Body> {
    Request::builder().method("POST").uri(uri)
        .header("Cookie", cookie(token))
        .header("content-type", "application/json")
        .body(Body::from("{}")).unwrap()
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp   = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes  = resp.into_body().collect().await.unwrap().to_bytes();
    let json   = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

// ── Neo4j container macro (matches pattern in projects.rs) ───────────────────

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

// ── auth guard tests (no Docker needed) ──────────────────────────────────────

mod auth_guards {
    use super::*;
    use axum::http::StatusCode;

    fn stub_router() -> Router {
        let secret = Arc::new(JWT_SECRET.to_string());
        Router::new()
            .route("/projects/:pid/overview",
                   axum::routing::get(|| async { StatusCode::OK }))
            .route("/projects/:pid/overview/regenerate",
                   axum::routing::post(|| async { StatusCode::ACCEPTED }))
            .layer(from_fn_with_state(secret, auth::require_auth))
    }

    #[tokio::test]
    async fn get_overview_requires_auth() {
        let (status, _) = send(stub_router(), Request::builder()
            .method("GET").uri("/projects/pid/overview")
            .body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn regenerate_requires_auth() {
        let (status, _) = send(stub_router(), Request::builder()
            .method("POST").uri("/projects/pid/overview/regenerate")
            .header("content-type", "application/json")
            .body(Body::from("{}")).unwrap()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

// ── Docker integration tests ──────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_overview_empty_returns_null_status() {
    neo4j!(c, neo4j);
    let app = overview_app(Arc::clone(&neo4j), stub_llm("Generated overview content"));

    let (uid, token) = make_user(&neo4j, "alice@example.com", "Alice", "regular").await;
    let gid          = make_group(&neo4j, "team").await;
    join_group(&neo4j, &uid, &gid).await;
    let pid          = make_project(&neo4j, &gid, &uid, "proj").await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/overview"), &token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["current_status"].is_null(), "expected null current_status, got: {body}");
    assert!(body["current_status_updated_at"].is_null());
    assert_eq!(body["has_conversations"], json!(false));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_overview_with_conversations_shows_has_conversations_true() {
    neo4j!(c, neo4j);
    let app = overview_app(Arc::clone(&neo4j), stub_llm("Generated overview"));

    let (uid, token) = make_user(&neo4j, "bob@example.com", "Bob", "regular").await;
    let gid          = make_group(&neo4j, "team2").await;
    join_group(&neo4j, &uid, &gid).await;
    let pid          = make_project(&neo4j, &gid, &uid, "proj2").await;
    add_conversation(&neo4j, &pid).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/overview"), &token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["has_conversations"], json!(true));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_overview_requires_project_access() {
    neo4j!(c, neo4j);
    let app = overview_app(Arc::clone(&neo4j), stub_llm("stub"));

    let (uid1, _)     = make_user(&neo4j, "owner@example.com", "Owner", "regular").await;
    let (_uid2, tok2) = make_user(&neo4j, "other@example.com", "Other", "regular").await;
    let gid           = make_group(&neo4j, "owners").await;
    join_group(&neo4j, &uid1, &gid).await;
    let pid           = make_project(&neo4j, &gid, &uid1, "private-proj").await;

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/overview"), &tok2)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn regenerate_streams_done_event() {
    neo4j!(c, neo4j);
    let app = overview_app(Arc::clone(&neo4j), stub_llm("# Status\nAll systems operational."));

    let (uid, token) = make_user(&neo4j, "carol@example.com", "Carol", "regular").await;
    let gid          = make_group(&neo4j, "team3").await;
    join_group(&neo4j, &uid, &gid).await;
    let pid          = make_project(&neo4j, &gid, &uid, "proj3").await;
    add_conversation(&neo4j, &pid).await;

    let resp = app.oneshot(req_post(&format!("/projects/{pid}/overview/regenerate"), &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body_str = std::str::from_utf8(&bytes).unwrap();
    assert!(body_str.contains("overview_done"), "expected overview_done in SSE body: {body_str}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn regenerate_requires_project_access() {
    neo4j!(c, neo4j);
    let app = overview_app(Arc::clone(&neo4j), stub_llm("stub"));

    let (uid1, _)     = make_user(&neo4j, "o2@example.com", "O2", "regular").await;
    let (_uid2, tok2) = make_user(&neo4j, "x2@example.com", "X2", "regular").await;
    let gid           = make_group(&neo4j, "grp4").await;
    join_group(&neo4j, &uid1, &gid).await;
    let pid           = make_project(&neo4j, &gid, &uid1, "p4").await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/overview/regenerate"), &tok2)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_overview_returns_content_after_regenerate() {
    neo4j!(c, neo4j);
    let status_text = "# Current Status\n- nginx: running\n- postgres: running";
    let llm = stub_llm(status_text);

    let (uid, token) = make_user(&neo4j, "dave@example.com", "Dave", "regular").await;
    let gid          = make_group(&neo4j, "team5").await;
    join_group(&neo4j, &uid, &gid).await;
    let pid          = make_project(&neo4j, &gid, &uid, "proj5").await;
    add_conversation(&neo4j, &pid).await;

    // Trigger regeneration and read the full SSE stream until it closes
    // (the stream ends after overview_done is sent and the channel drops)
    let app1 = overview_app(Arc::clone(&neo4j), Arc::clone(&llm));
    let resp = app1.oneshot(req_post(&format!("/projects/{pid}/overview/regenerate"), &token)).await.unwrap();
    let _ = resp.into_body().collect().await.unwrap().to_bytes();

    // Pipeline is complete — fetch the overview
    let app2 = overview_app(Arc::clone(&neo4j), Arc::clone(&llm));
    let (status, body) = send(app2, req_get(&format!("/projects/{pid}/overview"), &token)).await;
    assert_eq!(status, StatusCode::OK);
    let cs = body["current_status"].as_str().unwrap_or("");
    assert!(!cs.is_empty(), "expected non-empty current_status after regenerate, body: {body}");
    assert!(body["current_status_updated_at"].is_string());
}
