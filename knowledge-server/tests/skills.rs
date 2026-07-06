use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{get as route_get, post as route_post, put as route_put},
    Router,
};
use http_body_util::BodyExt as _;
use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j};
use serde_json::{json, Value};
use tower::ServiceExt as _;
use uuid::Uuid;

use knowledge_server::{
    agent::{skill_tools::{ListSkillsTool, LoadSkillTool}, tool::Tool, Agent},
    api::ProjectAgentBuilder,
    auth::{self, jwt},
    llm::{
        LlmProvider,
        types::{LlmResponse, Message, ToolDefinition},
    },
    machines::MachineRegistry,
    neo4j::Neo4jClient,
    projects::handlers::{
        create_project, create_project_skill, delete_project_skill,
        get_project_skill, list_project_skills, update_project_skill,
        ProjectState,
    },
    skills::{
        handlers::{
            create_global_skill, delete_global_skill, get_global_skill,
            list_global_skills, update_global_skill,
        },
        SkillStore,
    },
};

struct FixedTextLlm(String);
impl FixedTextLlm {
    fn new(t: impl Into<String>) -> Arc<Self> { Arc::new(Self(t.into())) }
}
#[async_trait]
impl LlmProvider for FixedTextLlm {
    async fn chat(&self, _: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone() })
    }
}

const JWT_SECRET: &str = "test-skills-secret";

fn skills_app(neo4j: Arc<Neo4jClient>) -> Router {
    let secret      = Arc::new(JWT_SECRET.to_string());
    let skill_store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));

    let global_read = Router::new()
        .route("/skills",     route_get(list_global_skills))
        .route("/skills/:id", route_get(get_global_skill))
        .with_state(Arc::clone(&skill_store))
        .layer(from_fn_with_state(Arc::clone(&secret), auth::require_auth));

    let global_admin = Router::new()
        .route("/admin/skills", route_post(create_global_skill))
        .route("/admin/skills/:id", route_put(update_global_skill)
                                    .delete(delete_global_skill))
        .with_state(Arc::clone(&skill_store))
        .layer(from_fn_with_state(Arc::clone(&secret), auth::require_admin));

    let llm: Arc<dyn LlmProvider> = FixedTextLlm::new("stub");
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        neo4j:                      Arc::clone(&neo4j),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::clone(&skill_store),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let project_state = Arc::new(ProjectState::new(Arc::clone(&neo4j), agent, builder));

    let project_routes = Router::new()
        .route("/projects", route_post(create_project))
        .route("/projects/:pid/skills",
               route_get(list_project_skills).post(create_project_skill))
        .route("/projects/:pid/skills/:sid",
               route_get(get_project_skill).put(update_project_skill).delete(delete_project_skill))
        .with_state(project_state)
        .layer(from_fn_with_state(Arc::clone(&secret), auth::require_auth));

    Router::new().merge(global_read).merge(global_admin).merge(project_routes)
}

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT project_id IF NOT EXISTS FOR (p:Project) REQUIRE p.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT skill_id   IF NOT EXISTS FOR (s:Skill)   REQUIRE s.id IS UNIQUE").await.unwrap();
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

async fn make_project_raw(neo4j: &Neo4jClient, name: &str) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "CREATE (:Project {id:$id, name:$name, description:'', group_id:'g', created_by:'system', created_at:$now})",
        json!({"id": id, "name": name, "now": now}),
    ).await.unwrap();
    id
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

async fn seed_project(app: &Router, token: &str, group_id: &str, name: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_post("/projects", token, json!({"name": name, "group_id": group_id})),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

#[allow(clippy::too_many_arguments)]
async fn seed_skill_raw(
    neo4j: &Neo4jClient,
    name: &str,
    description: &str,
    content: &str,
    is_global: bool,
    project_id: Option<&str>,
) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "CREATE (s:Skill {id:$id, name:$name, description:$description, content:$content,
                           is_global:$is_global, created_by:'system', created_at:$now, updated_at:$now})",
        json!({
            "id": id, "name": name, "description": description, "content": content,
            "is_global": is_global, "now": now,
        }),
    ).await.unwrap();
    if let Some(pid) = project_id {
        neo4j.query_read(
            "MATCH (p:Project {id:$pid}), (s:Skill {id:$sid}) CREATE (p)-[:HAS_SKILL]->(s)",
            json!({"pid": pid, "sid": id}),
        ).await.unwrap();
    }
    id
}

async fn count_skills(neo4j: &Neo4jClient) -> usize {
    let rows = neo4j.query_read("MATCH (s:Skill) RETURN count(s) AS n", json!({})).await.unwrap();
    rows.first().and_then(|r| r["n"].as_u64()).unwrap_or(0) as usize
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

// ---- Global skill CRUD ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_global_skills_empty_initially() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (status, body) = send(app, req_get("/skills", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_global_skill_returns_201_with_fields() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (status, body) = send(
        app,
        req_post("/admin/skills", &admin_tok, json!({
            "name": "juju", "description": "Juju guide", "content": "# Juju\nbody"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());
    assert_eq!(body["name"], "juju");
    assert!(body["created_at"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_global_skill_rejected_for_non_admin() {
    neo4j!(c, neo4j);
    let (_, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (status, _) = send(
        app,
        req_post("/admin/skills", &tok, json!({
            "name": "juju", "description": "Juju guide", "content": "body"
        })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_global_skill_rejected_unauthenticated() {
    neo4j!(c, neo4j);
    let app = skills_app(Arc::clone(&neo4j));

    let req = Request::builder().method("POST").uri("/admin/skills")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({
            "name": "juju", "description": "d", "content": "c"
        })).unwrap())).unwrap();
    let (status, _) = send(app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_global_skill_requires_name() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (status, _) = send(
        app,
        req_post("/admin/skills", &admin_tok, json!({
            "name": "", "description": "d", "content": "c"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_global_skill_duplicate_name_returns_409() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;

    let (status, _) = send(app, req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "other", "content": "other"
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_global_skill_returns_full_content() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let (_, tok)       = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "Juju guide", "content": "# Juju\nfull body"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, body) = send(app, req_get(&format!("/skills/{id}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "juju");
    assert_eq!(body["content"], "# Juju\nfull body");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_global_skills_omits_content() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "secret body"
    }))).await;

    let (status, body) = send(app, req_get("/skills", &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert!(arr[0].get("content").is_none(), "list should not include content");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_global_skill_changes_fields() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "old", "content": "old body"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_put(&format!("/admin/skills/{id}"), &admin_tok, json!({
        "description": "new", "content": "new body"
    }))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app, req_get(&format!("/skills/{id}"), &admin_tok)).await;
    assert_eq!(body["description"], "new");
    assert_eq!(body["content"], "new body");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_global_skill_rejected_for_non_admin() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let (_, tok)       = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(app, req_put(&format!("/admin/skills/{id}"), &tok, json!({
        "description": "new"
    }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_global_skill_duplicate_name_returns_409() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;
    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "lxd", "description": "d", "content": "c"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(app, req_put(&format!("/admin/skills/{id}"), &admin_tok, json!({
        "name": "juju"
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_global_skill_removes_it() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_del(&format!("/admin/skills/{id}"), &admin_tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/skills/{id}"), &admin_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_global_skill_rejected_for_non_admin() {
    neo4j!(c, neo4j);
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let (_, tok)       = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&neo4j));

    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(app, req_del(&format!("/admin/skills/{id}"), &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---- SkillStore direct tests ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn skill_store_list_for_project_includes_global_and_own_skills() {
    neo4j!(c, neo4j);
    let store = SkillStore::new(Arc::clone(&neo4j));
    let pid = make_project_raw(&neo4j, "proj-a").await;

    seed_skill_raw(&neo4j, "juju", "juju guide", "juju body", true, None).await;
    seed_skill_raw(&neo4j, "proj-a-only", "custom", "custom body", false, Some(&pid)).await;

    let list = store.list_for_project(&pid).await;
    let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"juju"), "global skill missing from list: {names:?}");
    assert!(names.contains(&"proj-a-only"), "project's own skill missing from list: {names:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn skill_store_list_for_project_excludes_other_projects_skills() {
    neo4j!(c, neo4j);
    let store = SkillStore::new(Arc::clone(&neo4j));
    let pid_a = make_project_raw(&neo4j, "proj-a").await;
    let pid_b = make_project_raw(&neo4j, "proj-b").await;

    seed_skill_raw(&neo4j, "proj-a-only", "custom", "custom body", false, Some(&pid_a)).await;

    let list_b = store.list_for_project(&pid_b).await;
    let names: Vec<&str> = list_b.iter().map(|s| s.name.as_str()).collect();
    assert!(!names.contains(&"proj-a-only"), "project B should not see project A's skill: {names:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn skill_store_load_content_resolves_global_skill_by_name() {
    neo4j!(c, neo4j);
    let store = SkillStore::new(Arc::clone(&neo4j));
    let pid = make_project_raw(&neo4j, "proj-a").await;
    seed_skill_raw(&neo4j, "juju", "juju guide", "juju body content", true, None).await;

    let content = store.load_content("juju", &pid).await;
    assert_eq!(content.as_deref(), Some("juju body content"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn skill_store_load_content_resolves_project_skill_by_name() {
    neo4j!(c, neo4j);
    let store = SkillStore::new(Arc::clone(&neo4j));
    let pid = make_project_raw(&neo4j, "proj-a").await;
    seed_skill_raw(&neo4j, "proj-a-only", "custom", "custom body content", false, Some(&pid)).await;

    let content = store.load_content("proj-a-only", &pid).await;
    assert_eq!(content.as_deref(), Some("custom body content"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn skill_store_load_content_project_skill_not_resolvable_from_other_project() {
    neo4j!(c, neo4j);
    let store = SkillStore::new(Arc::clone(&neo4j));
    let pid_a = make_project_raw(&neo4j, "proj-a").await;
    let pid_b = make_project_raw(&neo4j, "proj-b").await;
    seed_skill_raw(&neo4j, "proj-a-only", "custom", "custom body content", false, Some(&pid_a)).await;

    let content = store.load_content("proj-a-only", &pid_b).await;
    assert_eq!(content, None);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn skill_store_load_content_unknown_name_returns_none() {
    neo4j!(c, neo4j);
    let store = SkillStore::new(Arc::clone(&neo4j));
    let pid = make_project_raw(&neo4j, "proj-a").await;

    let content = store.load_content("nonexistent", &pid).await;
    assert_eq!(content, None);
}

// ---- Seeding ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn seed_defaults_if_needed_seeds_five_skills_once() {
    neo4j!(c, neo4j);
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    assert_eq!(count_skills(&neo4j).await, 5);

    let rows = neo4j.query_read(
        "MATCH (s:Skill {is_global: true}) RETURN s.name AS name ORDER BY s.name",
        json!({}),
    ).await.unwrap();
    let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
    for expected in ["juju", "lxd", "ceph", "canonical-k8s", "landscape"] {
        assert!(names.contains(&expected), "missing seeded skill '{expected}': {names:?}");
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn seed_defaults_if_needed_is_idempotent_second_call_no_duplicates() {
    neo4j!(c, neo4j);
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    assert_eq!(count_skills(&neo4j).await, 5);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn seed_defaults_if_needed_does_not_reseed_after_deletion() {
    neo4j!(c, neo4j);
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    assert_eq!(count_skills(&neo4j).await, 5);

    neo4j.run("MATCH (s:Skill) DETACH DELETE s").await.unwrap();
    assert_eq!(count_skills(&neo4j).await, 0);

    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    assert_eq!(count_skills(&neo4j).await, 0, "deleted skills must not reappear after re-seeding attempt");
}

// ---- Project-scoped skill CRUD ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_project_skills_empty_for_new_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/skills"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_skill_returns_201_with_fields() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/skills"), &tok, json!({
            "name": "runbook", "description": "Team runbook", "content": "# Runbook\nsteps"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());
    assert_eq!(body["name"], "runbook");
    assert!(body["created_at"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_project_skill_returns_full_content() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (_, created) = send(app.clone(), req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "runbook", "description": "Team runbook", "content": "full body"
    }))).await;
    let sid = created["id"].as_str().unwrap();

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/skills/{sid}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "runbook");
    assert_eq!(body["content"], "full body");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_project_skills_after_create_returns_summary() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    send(app.clone(), req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "runbook", "description": "d", "content": "secret content"
    }))).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/skills"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "runbook");
    assert!(arr[0].get("content").is_none(), "list should not include content");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_project_skill_changes_fields() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (_, created) = send(app.clone(), req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "runbook", "description": "old", "content": "old body"
    }))).await;
    let sid = created["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_put(&format!("/projects/{pid}/skills/{sid}"), &tok, json!({
        "description": "new", "content": "new body"
    }))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app, req_get(&format!("/projects/{pid}/skills/{sid}"), &tok)).await;
    assert_eq!(body["description"], "new");
    assert_eq!(body["content"], "new body");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_project_skill_removes_it() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (_, created) = send(app.clone(), req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "runbook", "description": "d", "content": "c"
    }))).await;
    let sid = created["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_del(&format!("/projects/{pid}/skills/{sid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/skills/{sid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn non_member_cannot_access_project_skills() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, outsider_tok) = make_user(&neo4j, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/skills"), &outsider_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn admin_can_access_any_project_skills() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/skills"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_skill_requires_name() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "", "description": "d", "content": "c"
    }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_skill_duplicate_name_within_project_returns_409() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    send(app.clone(), req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "runbook", "description": "d", "content": "c"
    }))).await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "runbook", "description": "other", "content": "other"
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_skill_same_name_as_global_returns_409() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&neo4j, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "juju", "description": "other", "content": "other"
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_project_skill_same_name_in_different_project_is_allowed() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid_a = seed_project(&app, &tok, &gid, "Project A").await;
    let pid_b = seed_project(&app, &tok, &gid, "Project B").await;

    let (status_a, _) = send(app.clone(), req_post(&format!("/projects/{pid_a}/skills"), &tok, json!({
        "name": "runbook", "description": "d", "content": "c"
    }))).await;
    assert_eq!(status_a, StatusCode::CREATED);

    let (status_b, _) = send(app, req_post(&format!("/projects/{pid_b}/skills"), &tok, json!({
        "name": "runbook", "description": "d2", "content": "c2"
    }))).await;
    assert_eq!(status_b, StatusCode::CREATED);
}

// ---- Cross-project isolation ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn project_skill_not_visible_from_other_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid_a = seed_project(&app, &tok, &gid, "Project A").await;
    let pid_b = seed_project(&app, &tok, &gid, "Project B").await;

    let (_, created) = send(app.clone(), req_post(&format!("/projects/{pid_a}/skills"), &tok, json!({
        "name": "foo", "description": "d", "content": "c"
    }))).await;
    let sid = created["id"].as_str().unwrap();

    let (_, list_b) = send(app.clone(), req_get(&format!("/projects/{pid_b}/skills"), &tok)).await;
    assert_eq!(list_b, json!([]), "project B's list must not include project A's skill");

    let (status, _) = send(app, req_get(&format!("/projects/{pid_b}/skills/{sid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "project A's skill must 404 via project B's URL");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_project_skills_never_includes_other_projects_skills_even_with_same_name() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let app = skills_app(Arc::clone(&neo4j));
    let pid_a = seed_project(&app, &tok, &gid, "Project A").await;
    let pid_b = seed_project(&app, &tok, &gid, "Project B").await;

    let (_, created_a) = send(app.clone(), req_post(&format!("/projects/{pid_a}/skills"), &tok, json!({
        "name": "foo", "description": "a-desc", "content": "a-content"
    }))).await;
    let (_, created_b) = send(app.clone(), req_post(&format!("/projects/{pid_b}/skills"), &tok, json!({
        "name": "foo", "description": "b-desc", "content": "b-content"
    }))).await;
    assert_ne!(created_a["id"], created_b["id"]);

    let (_, list_a) = send(app.clone(), req_get(&format!("/projects/{pid_a}/skills"), &tok)).await;
    let arr_a = list_a.as_array().unwrap();
    assert_eq!(arr_a.len(), 1);
    assert_eq!(arr_a[0]["id"], created_a["id"]);

    let (_, list_b) = send(app, req_get(&format!("/projects/{pid_b}/skills"), &tok)).await;
    let arr_b = list_b.as_array().unwrap();
    assert_eq!(arr_b.len(), 1);
    assert_eq!(arr_b[0]["id"], created_b["id"]);
}

// ---- Agent tools (list_skills / load_skill) ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_skills_definition_has_correct_name() {
    neo4j!(c, neo4j);
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = ListSkillsTool { store, project_id: "any".into() };
    assert_eq!(tool.definition().name, "list_skills");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn load_skill_definition_has_correct_name() {
    neo4j!(c, neo4j);
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = LoadSkillTool { store, project_id: "any".into() };
    assert_eq!(tool.definition().name, "load_skill");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_skills_returns_json_array_with_seeded_skill() {
    neo4j!(c, neo4j);
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    let pid = make_project_raw(&neo4j, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = ListSkillsTool { store, project_id: pid };

    let result = tool.execute(json!({})).await.unwrap();
    let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
    let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"juju"), "missing seeded skill in tool output: {names:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_skills_each_item_has_name_and_description() {
    neo4j!(c, neo4j);
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    let pid = make_project_raw(&neo4j, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = ListSkillsTool { store, project_id: pid };

    let result = tool.execute(json!({})).await.unwrap();
    let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
    assert!(!arr.is_empty());
    for item in &arr {
        assert!(item["name"].is_string(),        "item missing name: {item}");
        assert!(item["description"].is_string(), "item missing description: {item}");
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn load_skill_returns_content_for_known_skill() {
    neo4j!(c, neo4j);
    knowledge_server::skills::seed_defaults_if_needed(&neo4j).await.unwrap();
    let pid = make_project_raw(&neo4j, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = LoadSkillTool { store, project_id: pid };

    let result = tool.execute(json!({ "name": "juju" })).await.unwrap();
    assert!(!result.is_empty());
    assert!(!result.contains("name: juju"), "frontmatter must not appear in output");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn load_skill_missing_name_param_returns_error() {
    neo4j!(c, neo4j);
    let pid = make_project_raw(&neo4j, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = LoadSkillTool { store, project_id: pid };

    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn load_skill_unknown_name_returns_error() {
    neo4j!(c, neo4j);
    let pid = make_project_raw(&neo4j, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = LoadSkillTool { store, project_id: pid };

    let result = tool.execute(json!({ "name": "nonexistent" })).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn load_skill_preview_is_markdown_envelope() {
    neo4j!(c, neo4j);
    let pid = make_project_raw(&neo4j, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool = LoadSkillTool { store, project_id: pid };

    let preview = tool.preview("# Heading\nsome text");
    let parsed: Value = serde_json::from_str(&preview).expect("preview must be valid JSON");
    assert_eq!(parsed["__type"].as_str(), Some("markdown"), "__type must be 'markdown'");
    assert!(parsed["content"].as_str().map(|s| s.contains("# Heading")).unwrap_or(false));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn load_skill_scoped_to_project_via_agent_tool() {
    neo4j!(c, neo4j);
    let pid_a = make_project_raw(&neo4j, "proj-a").await;
    let pid_b = make_project_raw(&neo4j, "proj-b").await;
    seed_skill_raw(&neo4j, "proj-a-only", "custom", "custom body", false, Some(&pid_a)).await;

    let store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let tool_b = LoadSkillTool { store, project_id: pid_b };

    let result = tool_b.execute(json!({ "name": "proj-a-only" })).await;
    assert!(result.is_err(), "project B's LoadSkillTool must not resolve project A's skill");
}
