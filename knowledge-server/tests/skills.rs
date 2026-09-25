use harvest_db::Db;
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
use harvest_db::test_support::TestDb;
use serde_json::{json, Value};
use tower::ServiceExt as _;
use uuid::Uuid;

use knowledge_server::{
    agent::{skill_tools::{ListSkillsTool, LoadSkillTool}, tool::Tool, Agent},
    api::ProjectAgentBuilder,
    auth::{self, jwt},
    llm::{
        LlmProvider,
        types::{LlmResponse, Message, ModelInfo, ToolDefinition, Usage},
    },
    machines::MachineRegistry,

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
    fn id(&self) -> &str { "fixed-text" }
    fn kind(&self) -> &str { "mock" }
    fn default_model(&self) -> &str { "mock-model" }
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> { Ok(vec![]) }
    async fn chat_with(&self, _model: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone(), usage: Usage::default() })
    }
}

const JWT_SECRET: &str = "test-skills-secret";

fn skills_app(db: Arc<Db>) -> Router {
    let secret      = Arc::new(JWT_SECRET.to_string());
    let skill_store = Arc::new(SkillStore::new(Arc::clone(&db)));

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
        db:                      Arc::clone(&db),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::clone(&skill_store),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             2,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let project_state = Arc::new(ProjectState::new(Arc::clone(&db), agent, builder, Arc::clone(&llm) as Arc<dyn LlmProvider>, Arc::new(vec![]), None, Arc::new(knowledge_server::cost::PricingTable::default())));

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

macro_rules! db {
    ($c:ident, $db:ident) => {
        let $c = TestDb::new().await;
        let $db = Arc::new($c.db.clone());
    };
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

async fn make_project_raw(db: &Db, name: &str) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.query(
        "WITH g AS (
             INSERT INTO groups (id, name) VALUES ('g', 'g') ON CONFLICT (id) DO UPDATE SET name = groups.name
             RETURNING id
         )
         INSERT INTO projects (id, name, description, group_id, created_by, created_at)
         SELECT $id, $name, '', g.id, 'system', $now::timestamptz FROM g",
        json!({"id": id, "name": name, "now": now}),
    ).await.unwrap();
    id
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

async fn seed_project(app: &Router, token: &str, group_id: &str, name: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_post("/projects", token, json!({"name": name, "group_id": group_id})),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

#[allow(clippy::too_many_arguments)]
async fn seed_skill_raw(
    db: &Db,
    name: &str,
    description: &str,
    content: &str,
    is_global: bool,
    project_id: Option<&str>,
) -> String {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.query(
        "INSERT INTO skills (id, project_id, name, description, content, created_by, created_at, updated_at)
                           VALUES ($id, $pid, $name, $description, $content, 'system', $now, $now)",
        json!({
            "id": id, "name": name, "description": description, "content": content,
            "pid": if is_global { None } else { project_id }, "now": now,
        }),
    ).await.unwrap();
    id
}

async fn count_skills(db: &Db) -> usize {
    let rows = db.query("SELECT count(*) AS n FROM skills", json!({})).await.unwrap();
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_global_skills_empty_initially() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&db));

    let (status, body) = send(app, req_get("/skills", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_global_skill_returns_201_with_fields() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_global_skill_rejected_for_non_admin() {
    db!(c, db);
    let (_, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&db));

    let (status, _) = send(
        app,
        req_post("/admin/skills", &tok, json!({
            "name": "juju", "description": "Juju guide", "content": "body"
        })),
    ).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_global_skill_rejected_unauthenticated() {
    db!(c, db);
    let app = skills_app(Arc::clone(&db));

    let req = Request::builder().method("POST").uri("/admin/skills")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&json!({
            "name": "juju", "description": "d", "content": "c"
        })).unwrap())).unwrap();
    let (status, _) = send(app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_global_skill_requires_name() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

    let (status, _) = send(
        app,
        req_post("/admin/skills", &admin_tok, json!({
            "name": "", "description": "d", "content": "c"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_global_skill_duplicate_name_returns_409() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

    send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;

    let (status, _) = send(app, req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "other", "content": "other"
    }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_global_skill_returns_full_content() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let (_, tok)       = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_global_skills_omits_content() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn update_global_skill_changes_fields() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn update_global_skill_rejected_for_non_admin() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let (_, tok)       = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn update_global_skill_duplicate_name_returns_409() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn delete_global_skill_removes_it() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let app = skills_app(Arc::clone(&db));

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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn delete_global_skill_rejected_for_non_admin() {
    db!(c, db);
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let (_, tok)       = make_user(&db, "a@x.com", "Alice", "regular").await;
    let app = skills_app(Arc::clone(&db));

    let (_, created) = send(app.clone(), req_post("/admin/skills", &admin_tok, json!({
        "name": "juju", "description": "d", "content": "c"
    }))).await;
    let id = created["id"].as_str().unwrap();

    let (status, _) = send(app, req_del(&format!("/admin/skills/{id}"), &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ---- SkillStore direct tests ----

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn skill_store_list_for_project_includes_global_and_own_skills() {
    db!(c, db);
    let store = SkillStore::new(Arc::clone(&db));
    let pid = make_project_raw(&db, "proj-a").await;

    seed_skill_raw(&db, "juju", "juju guide", "juju body", true, None).await;
    seed_skill_raw(&db, "proj-a-only", "custom", "custom body", false, Some(&pid)).await;

    let list = store.list_for_project(&pid).await;
    let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"juju"), "global skill missing from list: {names:?}");
    assert!(names.contains(&"proj-a-only"), "project's own skill missing from list: {names:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn skill_store_list_for_project_excludes_other_projects_skills() {
    db!(c, db);
    let store = SkillStore::new(Arc::clone(&db));
    let pid_a = make_project_raw(&db, "proj-a").await;
    let pid_b = make_project_raw(&db, "proj-b").await;

    seed_skill_raw(&db, "proj-a-only", "custom", "custom body", false, Some(&pid_a)).await;

    let list_b = store.list_for_project(&pid_b).await;
    let names: Vec<&str> = list_b.iter().map(|s| s.name.as_str()).collect();
    assert!(!names.contains(&"proj-a-only"), "project B should not see project A's skill: {names:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn skill_store_load_content_resolves_global_skill_by_name() {
    db!(c, db);
    let store = SkillStore::new(Arc::clone(&db));
    let pid = make_project_raw(&db, "proj-a").await;
    seed_skill_raw(&db, "juju", "juju guide", "juju body content", true, None).await;

    let content = store.load_content("juju", &pid).await;
    assert_eq!(content.as_deref(), Some("juju body content"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn skill_store_load_content_resolves_project_skill_by_name() {
    db!(c, db);
    let store = SkillStore::new(Arc::clone(&db));
    let pid = make_project_raw(&db, "proj-a").await;
    seed_skill_raw(&db, "proj-a-only", "custom", "custom body content", false, Some(&pid)).await;

    let content = store.load_content("proj-a-only", &pid).await;
    assert_eq!(content.as_deref(), Some("custom body content"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn skill_store_load_content_project_skill_not_resolvable_from_other_project() {
    db!(c, db);
    let store = SkillStore::new(Arc::clone(&db));
    let pid_a = make_project_raw(&db, "proj-a").await;
    let pid_b = make_project_raw(&db, "proj-b").await;
    seed_skill_raw(&db, "proj-a-only", "custom", "custom body content", false, Some(&pid_a)).await;

    let content = store.load_content("proj-a-only", &pid_b).await;
    assert_eq!(content, None);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn skill_store_load_content_unknown_name_returns_none() {
    db!(c, db);
    let store = SkillStore::new(Arc::clone(&db));
    let pid = make_project_raw(&db, "proj-a").await;

    let content = store.load_content("nonexistent", &pid).await;
    assert_eq!(content, None);
}

// ---- Seeding ----

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn seed_defaults_if_needed_seeds_five_skills_once() {
    db!(c, db);
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    assert_eq!(count_skills(&db).await, 5);

    let rows = db.query(
        "SELECT name FROM skills WHERE project_id IS NULL ORDER BY name",
        json!({}),
    ).await.unwrap();
    let names: Vec<&str> = rows.iter().filter_map(|r| r["name"].as_str()).collect();
    for expected in ["juju", "lxd", "ceph", "canonical-k8s", "landscape"] {
        assert!(names.contains(&expected), "missing seeded skill '{expected}': {names:?}");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn seed_defaults_if_needed_is_idempotent_second_call_no_duplicates() {
    db!(c, db);
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    assert_eq!(count_skills(&db).await, 5);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn seed_defaults_if_needed_does_not_reseed_after_deletion() {
    db!(c, db);
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    assert_eq!(count_skills(&db).await, 5);

    db.execute("DELETE FROM skills", json!({})).await.unwrap();
    assert_eq!(count_skills(&db).await, 0);

    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    assert_eq!(count_skills(&db).await, 0, "deleted skills must not reappear after re-seeding attempt");
}

// ---- Project-scoped skill CRUD ----

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_project_skills_empty_for_new_project() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/skills"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_skill_returns_201_with_fields() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_project_skill_returns_full_content() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_project_skills_after_create_returns_summary() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn update_project_skill_changes_fields() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn delete_project_skill_removes_it() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn non_member_cannot_access_project_skills() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, outsider_tok) = make_user(&db, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/skills"), &outsider_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn admin_can_access_any_project_skills() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/skills"), &admin_tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_skill_requires_name() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
    let pid = seed_project(&app, &tok, &gid, "Test Project").await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/skills"), &tok, json!({
        "name": "", "description": "d", "content": "c"
    }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_skill_duplicate_name_within_project_returns_409() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_skill_same_name_as_global_returns_409() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let (_, admin_tok) = make_user(&db, "admin@x.com", "Admin", "admin").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn create_project_skill_same_name_in_different_project_is_allowed() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn project_skill_not_visible_from_other_project() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_project_skills_never_includes_other_projects_skills_even_with_same_name() {
    db!(c, db);
    let (uid, tok) = make_user(&db, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&db, "eng").await;
    join_group(&db, &uid, &gid).await;
    let app = skills_app(Arc::clone(&db));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_skills_definition_has_correct_name() {
    db!(c, db);
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = ListSkillsTool { store, project_id: "any".into() };
    assert_eq!(tool.definition().name, "list_skills");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn load_skill_definition_has_correct_name() {
    db!(c, db);
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = LoadSkillTool { store, project_id: "any".into() };
    assert_eq!(tool.definition().name, "load_skill");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_skills_returns_json_array_with_seeded_skill() {
    db!(c, db);
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    let pid = make_project_raw(&db, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = ListSkillsTool { store, project_id: pid };

    let result = tool.execute(json!({})).await.unwrap();
    let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
    let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
    assert!(names.contains(&"juju"), "missing seeded skill in tool output: {names:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_skills_each_item_has_name_and_description() {
    db!(c, db);
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    let pid = make_project_raw(&db, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn load_skill_returns_content_for_known_skill() {
    db!(c, db);
    knowledge_server::skills::seed_defaults_if_needed(&db).await.unwrap();
    let pid = make_project_raw(&db, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = LoadSkillTool { store, project_id: pid };

    let result = tool.execute(json!({ "name": "juju" })).await.unwrap();
    assert!(!result.is_empty());
    assert!(!result.contains("name: juju"), "frontmatter must not appear in output");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn load_skill_missing_name_param_returns_error() {
    db!(c, db);
    let pid = make_project_raw(&db, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = LoadSkillTool { store, project_id: pid };

    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn load_skill_unknown_name_returns_error() {
    db!(c, db);
    let pid = make_project_raw(&db, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = LoadSkillTool { store, project_id: pid };

    let result = tool.execute(json!({ "name": "nonexistent" })).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("nonexistent"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn load_skill_preview_is_markdown_envelope() {
    db!(c, db);
    let pid = make_project_raw(&db, "proj-a").await;
    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool = LoadSkillTool { store, project_id: pid };

    let preview = tool.preview("# Heading\nsome text");
    let parsed: Value = serde_json::from_str(&preview).expect("preview must be valid JSON");
    assert_eq!(parsed["__type"].as_str(), Some("markdown"), "__type must be 'markdown'");
    assert!(parsed["content"].as_str().map(|s| s.contains("# Heading")).unwrap_or(false));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn load_skill_scoped_to_project_via_agent_tool() {
    db!(c, db);
    let pid_a = make_project_raw(&db, "proj-a").await;
    let pid_b = make_project_raw(&db, "proj-b").await;
    seed_skill_raw(&db, "proj-a-only", "custom", "custom body", false, Some(&pid_a)).await;

    let store = Arc::new(SkillStore::new(Arc::clone(&db)));
    let tool_b = LoadSkillTool { store, project_id: pid_b };

    let result = tool_b.execute(json!({ "name": "proj-a-only" })).await;
    assert!(result.is_err(), "project B's LoadSkillTool must not resolve project A's skill");
}
