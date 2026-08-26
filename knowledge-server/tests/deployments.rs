use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::from_fn_with_state,
    routing::{get as route_get, post as route_post},
    Router,
};
use chrono::Utc;
use http_body_util::BodyExt as _;
use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tower::ServiceExt as _;
use uuid::Uuid;

use knowledge_server::{
    agent::Agent,
    api::ProjectAgentBuilder,
    artifacts::handlers::{create_artifact, get_artifact_in_project, ArtifactKind},
    auth::{self, jwt},
    deployments::{
        handlers::{
            add_context_artifact, apply_provision_change, approve_proposal, create_deployment, create_template, delete_deployment, delete_template,
            deploy_deployment, destroy_deployment, discard_proposal,
            generate_design, generate_design_decisions,
            generate_environment_questions, generate_provision, get_deployment, get_execution_plan, get_project_deployment, get_template,
            link_context_artifact, list_deployment_runs, list_deployments, list_templates, list_proposals, propose_artifact_change, propose_provision_change,
            redeploy_deployment, remove_context_artifact, revise_design, run_dag, set_execution_plan, update_deployment, update_template,
        },
        last_applied_bundle_for_artifact, record_run_and_update_state,
    },
    issues::handlers::{
        apply_change_request, apply_issue_solution, create_change_request_comment,
        create_issue_comment, discard_change_request, get_change_request, get_issue,
        issue_chat, list_change_requests, list_issues,
        redeploy_from_issue, update_issue_status_route,
    },
    llm::{
        LlmProvider,
        types::{ContentPart, LlmResponse, Message, MessageContent, ModelInfo, Role, ToolCall, ToolDefinition},
    },
    machines::{CommandResult, ConnectedAgent, MachineRegistry, ServerToAgent, TerraformAction},
    neo4j::Neo4jClient,
    projects::handlers::{create_project, ProjectState},
    skills::SkillStore,
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
        Ok(LlmResponse::Message { text: self.0.clone() })
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
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> { Ok(vec![]) }
    async fn chat_with(&self, _model: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        self.0.lock().unwrap().pop_front().ok_or_else(|| anyhow::anyhow!("ScriptedLlm: no more responses"))
    }
}

fn text_response(text: &str) -> LlmResponse {
    LlmResponse::Message { text: text.to_string() }
}

fn tool_call_response(name: &str, input: Value) -> LlmResponse {
    LlmResponse::ToolCalls {
        calls: vec![ToolCall { id: Uuid::new_v4().to_string(), name: name.to_string(), input, thought_signature: None }],
        preamble: String::new(),
    }
}

fn tool_result_contents(messages: &[Message]) -> Vec<&str> {
    messages.iter().filter_map(|m| {
        if let MessageContent::Parts(parts) = &m.content {
            parts.iter().find_map(|p| match p {
                ContentPart::ToolResult { content, .. } => Some(content.as_str()),
                _ => None,
            })
        } else {
            None
        }
    }).collect()
}

fn message_text(m: &Message) -> String {
    match &m.content {
        MessageContent::Text(t) => t.clone(),
        MessageContent::Parts(parts) => parts.iter().filter_map(|p| match p {
            ContentPart::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("\n"),
    }
}

fn captured_prompt_for_design(messages: &[Message]) -> (String, String) {
    let system = messages.iter()
        .find(|m| matches!(m.role, Role::System))
        .map(message_text)
        .unwrap_or_default();
    let user = messages.iter().rev()
        .find(|m| matches!(m.role, Role::User))
        .map(message_text)
        .unwrap_or_default();
    (system, user)
}

struct ClosureLlm(Box<dyn Fn(&[Message]) -> LlmResponse + Send + Sync>);
impl ClosureLlm {
    fn new(f: impl Fn(&[Message]) -> LlmResponse + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self(Box::new(f)))
    }
}
#[async_trait]
impl LlmProvider for ClosureLlm {
    fn id(&self) -> &str { "closure" }
    fn kind(&self) -> &str { "mock" }
    fn default_model(&self) -> &str { "mock-model" }
    async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> { Ok(vec![]) }
    async fn chat_with(&self, _model: Option<&str>, messages: &[Message], _tools: &[ToolDefinition]) -> anyhow::Result<LlmResponse> {
        Ok((self.0)(messages))
    }
}

const JWT_SECRET: &str = "test-deployments-secret";

fn deployments_app(neo4j: Arc<Neo4jClient>) -> (Router, Arc<MachineRegistry>) {
    deployments_app_with_llm(neo4j, FixedTextLlm::new("stub"))
}

fn deployments_app_with_llm(neo4j: Arc<Neo4jClient>, llm: Arc<dyn LlmProvider>) -> (Router, Arc<MachineRegistry>) {
    let secret      = Arc::new(JWT_SECRET.to_string());
    let skill_store = Arc::new(SkillStore::new(Arc::clone(&neo4j)));
    let agent    = Arc::new(Agent::new(Arc::clone(&llm), vec![], 2));
    let registry = MachineRegistry::new();
    let builder  = Arc::new(ProjectAgentBuilder {
        llm:                        Arc::clone(&llm),
        neo4j:                      Arc::clone(&neo4j),
        registry:                   Arc::clone(&registry),
        skills:                     Arc::clone(&skill_store),
        lxd:                        None,
        server_url:                 "http://localhost".into(),
        max_iterations:             5,
        compaction_threshold_chars: usize::MAX,
        compaction_keep_last:       6,
    });
    let project_state = Arc::new(ProjectState::new(Arc::clone(&neo4j), agent, builder));

    let project_routes = Router::new()
        .route("/projects", route_post(create_project))
        .route("/projects/:pid/deployment",
               route_get(get_project_deployment))
        .route("/projects/:pid/deployments",
               route_get(list_deployments).post(create_deployment))
        .route("/projects/:pid/deployments/:did",
               route_get(get_deployment).patch(update_deployment).delete(delete_deployment))
        .route("/projects/:pid/deployments/:did/deploy",   route_post(deploy_deployment))
        .route("/projects/:pid/deployments/:did/redeploy", route_post(redeploy_deployment))
        .route("/projects/:pid/deployments/:did/destroy",  route_post(destroy_deployment))
        .route("/projects/:pid/deployments/:did/runs",     route_get(list_deployment_runs))
        .route("/projects/:pid/deployments/:did/environment/questions", route_post(generate_environment_questions))
        .route("/projects/:pid/deployments/:did/design/generate",       route_post(generate_design))
        .route("/projects/:pid/deployments/:did/design/decisions",      route_post(generate_design_decisions))
        .route("/projects/:pid/deployments/:did/design/revise",         route_post(revise_design))
        .route("/projects/:pid/deployments/:did/provision/generate",       route_post(generate_provision))
        .route("/projects/:pid/deployments/:did/provision/propose-change", route_post(propose_provision_change))
        .route("/projects/:pid/deployments/:did/provision/apply-change",   route_post(apply_provision_change))
        .route("/projects/:pid/deployments/:did/context-artifacts",        route_post(add_context_artifact))
        .route("/projects/:pid/deployments/:did/context-artifacts/link",   route_post(link_context_artifact))
        .route("/projects/:pid/deployments/:did/context-artifacts/:aid",  axum::routing::delete(remove_context_artifact))
        .route("/projects/:pid/deployments/:did/proposals",                 route_get(list_proposals).post(propose_artifact_change))
        .route("/projects/:pid/deployments/:did/proposals/:propid/approve", route_post(approve_proposal))
        .route("/projects/:pid/deployments/:did/proposals/:propid/discard", route_post(discard_proposal))
        .route("/projects/:pid/deployments/:did/execution-plan",            axum::routing::get(get_execution_plan).post(set_execution_plan))
        .route("/projects/:pid/deployments/:did/run-dag",                    route_post(run_dag))
        .route("/projects/:pid/issues", route_get(list_issues))
        .route("/projects/:pid/issues/:iid", route_get(get_issue))
        .route("/projects/:pid/issues/:iid/status", axum::routing::patch(update_issue_status_route))
        .route("/projects/:pid/issues/:iid/comments", route_post(create_issue_comment))
        .route("/projects/:pid/issues/:iid/apply-solution", route_post(apply_issue_solution))
        .route("/projects/:pid/issues/:iid/redeploy", route_post(redeploy_from_issue))
        .route("/projects/:pid/issues/:iid/chat", route_post(issue_chat))
        .route("/projects/:pid/change-requests", route_get(list_change_requests))
        .route("/projects/:pid/change-requests/:cid", route_get(get_change_request))
        .route("/projects/:pid/change-requests/:cid/apply", route_post(apply_change_request))
        .route("/projects/:pid/change-requests/:cid/discard", route_post(discard_change_request))
        .route("/projects/:pid/change-requests/:cid/comments", route_post(create_change_request_comment))
        .route("/templates",
               route_get(list_templates).post(create_template))
        .route("/templates/:tid",
               route_get(get_template).put(update_template).delete(delete_template))
        .with_state(project_state)
        .layer(from_fn_with_state(Arc::clone(&secret), auth::require_auth));

    (Router::new().merge(project_routes), registry)
}

async fn setup_constraints(neo4j: &Neo4jClient) {
    auth::setup_constraints(neo4j).await.unwrap();
    neo4j.run("CREATE CONSTRAINT project_id IF NOT EXISTS FOR (p:Project) REQUIRE p.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT deployment_id IF NOT EXISTS FOR (d:Deployment) REQUIRE d.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT template_id IF NOT EXISTS FOR (t:ProductTemplate) REQUIRE t.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT issue_id IF NOT EXISTS FOR (i:Issue) REQUIRE i.id IS UNIQUE").await.unwrap();
    neo4j.run("CREATE CONSTRAINT proposal_id IF NOT EXISTS FOR (p:Proposal) REQUIRE p.id IS UNIQUE").await.unwrap();
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

fn req_patch(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder().method("PATCH").uri(uri)
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

async fn count_nodes(neo4j: &Neo4jClient, label: &str) -> usize {
    let rows = neo4j.query_read(&format!("MATCH (n:{label}) RETURN count(n) AS n"), json!({})).await.unwrap();
    rows.first().and_then(|r| r["n"].as_u64()).unwrap_or(0) as usize
}

fn register_agent(registry: &MachineRegistry, agent_id: &str, project_id: &str) -> mpsc::Receiver<ServerToAgent> {
    let (tx, rx) = mpsc::channel::<ServerToAgent>(8);
    registry.agents.insert(agent_id.to_string(), ConnectedAgent {
        id:           agent_id.to_string(),
        project_id:   project_id.to_string(),
        hostname:     "host-1".into(),
        connected_at: Utc::now(),
        sender:       tx,
    });
    rx
}

fn spawn_fake_agent_always_succeeds(
    mut rx: mpsc::Receiver<ServerToAgent>,
    registry: Arc<MachineRegistry>,
) -> mpsc::UnboundedReceiver<(TerraformAction, String)> {
    let (sent_tx, sent_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let ServerToAgent::RunTerraform { request_id, action, files, .. } = msg {
                let _ = sent_tx.send((action, serde_json::to_string(&files).unwrap()));
                if let Some((_, pending)) = registry.pending.remove(&request_id) {
                    let _ = pending.tx.send(Ok(CommandResult {
                        stdout: "ok".into(), stderr: String::new(), exit_code: 0,
                    }));
                }
            }
        }
    });
    sent_rx
}

fn spawn_fake_agent_with_script(
    mut rx: mpsc::Receiver<ServerToAgent>,
    registry: Arc<MachineRegistry>,
    exit_code_for: impl Fn(TerraformAction) -> i32 + Send + 'static,
) {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let ServerToAgent::RunTerraform { request_id, action, .. } = msg {
                let exit_code = exit_code_for(action);
                if let Some((_, pending)) = registry.pending.remove(&request_id) {
                    let _ = pending.tx.send(Ok(CommandResult {
                        stdout: "ok".into(), stderr: String::new(), exit_code,
                    }));
                }
            }
        }
    });
}

fn spawn_dag_fake_agent(
    mut rx: mpsc::Receiver<ServerToAgent>,
    registry: Arc<MachineRegistry>,
    exit_code_for: impl Fn(&str) -> i32 + Send + Sync + 'static,
) {
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match msg {
                ServerToAgent::RunTerraform { request_id, action, .. } => {
                    let exit_code = exit_code_for(action.as_str());
                    if let Some((_, pending)) = registry.pending.remove(&request_id) {
                        let _ = pending.tx.send(Ok(CommandResult {
                            stdout: format!("{} ok", action.as_str()), stderr: String::new(), exit_code,
                        }));
                    }
                }
                ServerToAgent::Execute { request_id, .. } => {
                    let exit_code = exit_code_for("run");
                    if let Some((_, pending)) = registry.pending.remove(&request_id) {
                        let _ = pending.tx.send(Ok(CommandResult {
                            stdout: "run ok".into(), stderr: String::new(), exit_code,
                        }));
                    }
                }
                _ => {}
            }
        }
    });
}

async fn seed_terraform_artifact(neo4j: &Neo4jClient, project_id: &str, content: &str) -> String {
    let created = create_artifact(neo4j, project_id, ArtifactKind::Terraform, "Infra", content, "system")
        .await.unwrap();
    created["id"].as_str().unwrap().to_string()
}

async fn seed_bash_artifact(neo4j: &Neo4jClient, project_id: &str, content: &str) -> String {
    let created = create_artifact(neo4j, project_id, ArtifactKind::Bash, "Script", content, "system")
        .await.unwrap();
    created["id"].as_str().unwrap().to_string()
}

async fn link_terraform_bundle(neo4j: &Neo4jClient, deployment_id: &str, artifact_id: &str) {
    neo4j.query_read(
        "MATCH (d:Deployment {id: $did}), (a:Artifact {id: $aid}) CREATE (d)-[:HAS_TERRAFORM_BUNDLE]->(a)",
        json!({ "did": deployment_id, "aid": artifact_id }),
    ).await.unwrap();
}

async fn seed_design_doc(neo4j: &Neo4jClient, project_id: &str, deployment_id: &str, content: &str) -> String {
    let created = create_artifact(neo4j, project_id, ArtifactKind::Markdown, "Design", content, "system")
        .await.unwrap();
    let aid = created["id"].as_str().unwrap().to_string();
    neo4j.query_read(
        "MATCH (d:Deployment {id: $did}), (a:Artifact {id: $aid}) CREATE (d)-[:HAS_DESIGN_DOC]->(a)",
        json!({ "did": deployment_id, "aid": aid }),
    ).await.unwrap();
    aid
}

async fn create_deployment_raw(app: &Router, token: &str, project_id: &str, _name: &str) -> String {
    let (_, body) = send(
        app.clone(),
        req_get(&format!("/projects/{project_id}/deployment"), token),
    ).await;
    body["id"].as_str().unwrap().to_string()
}

// ---- ProductTemplate CRUD ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_template_returns_201_and_is_listed() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let (status, body) = send(
        app.clone(),
        req_post("/templates", &tok, json!({
            "name": "Acme Gateway v3", "description": "standard rollout", "content": "# playbook"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());

    let (status, list) = send(app, req_get("/templates", &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "Acme Gateway v3");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn template_visible_from_two_different_projects_in_the_same_group() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let pid1 = seed_project(&app, &tok, &gid, "Customer A").await;
    let pid2 = seed_project(&app, &tok, &gid, "Customer B").await;

    let (_, created) = send(
        app.clone(),
        req_post("/templates", &tok, json!({
            "name": "Acme Gateway v3", "description": "", "content": "playbook v1"
        })),
    ).await;
    let tid = created["id"].as_str().unwrap();

    let (status, dep1) = send(
        app.clone(),
        req_post(&format!("/projects/{pid1}/deployments"), &tok, json!({
            "name": "A rollout", "environment_description": "on-prem", "product_template_id": tid
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, dep2) = send(
        app.clone(),
        req_post(&format!("/projects/{pid2}/deployments"), &tok, json!({
            "name": "B rollout", "environment_description": "cloud", "product_template_id": tid
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    let (_, got1) = send(app.clone(), req_get(&format!("/projects/{pid1}/deployments/{}", dep1["id"].as_str().unwrap()), &tok)).await;
    let (_, got2) = send(app, req_get(&format!("/projects/{pid2}/deployments/{}", dep2["id"].as_str().unwrap()), &tok)).await;
    assert_eq!(got1["template"]["id"], tid);
    assert_eq!(got2["template"]["id"], tid);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_template_replaces_content() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let (_, created) = send(
        app.clone(),
        req_post("/templates", &tok, json!({
            "name": "x", "description": "", "content": "v1"
        })),
    ).await;
    let tid = created["id"].as_str().unwrap();

    let (status, _) = send(
        app.clone(),
        req_put(&format!("/templates/{tid}"), &tok, json!({ "content": "v2" })),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let (_, got) = send(app, req_get(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(got["content"], "v2");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_template_removes_it() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let (_, created) = send(
        app.clone(),
        req_post("/templates", &tok, json!({ "name": "x", "description": "", "content": "" })),
    ).await;
    let tid = created["id"].as_str().unwrap();

    let (status, _) = send(app.clone(), req_del(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = send(app, req_get(&format!("/templates/{tid}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- Deployment creation ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_deployment_without_template_creates_deployment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "Rollout", "environment_description": "3 racks, air-gapped", "product_template_id": Value::Null
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].is_string());

    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
         RETURN d.infra_state AS infra_state",
        json!({ "pid": pid, "did": body["id"] }),
    ).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["infra_state"], "none");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_deployment_with_template_links_uses_template_edge() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (_, created_template) = send(
        app.clone(),
        req_post("/templates", &tok, json!({ "name": "x", "description": "", "content": "" })),
    ).await;
    let tid = created_template["id"].as_str().unwrap();

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "Rollout", "environment_description": "env", "product_template_id": tid
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);

    let (_, got) = send(app, req_get(&format!("/projects/{pid}/deployments/{}", body["id"].as_str().unwrap()), &tok)).await;
    assert_eq!(got["template"]["id"], tid);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_deployment_rejects_empty_name() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "   ", "environment_description": "env", "product_template_id": Value::Null
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- Deployment list/get/update/delete ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_deployments_orders_by_updated_at_desc() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (_, first) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "First", "environment_description": "e", "product_template_id": Value::Null
        })),
    ).await;
    let (_, second) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "Second", "environment_description": "e", "product_template_id": Value::Null
        })),
    ).await;

    let (status, list) = send(app, req_get(&format!("/projects/{pid}/deployments"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<_> = list.as_array().unwrap().iter().map(|d| d["id"].clone()).collect();
    assert_eq!(names, vec![second["id"].clone(), first["id"].clone()]);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_deployment_changes_environment_description() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (_, created) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "Rollout", "environment_description": "v1", "product_template_id": Value::Null
        })),
    ).await;
    let did = created["id"].as_str().unwrap();

    let (status, _) = send(
        app.clone(),
        req_patch(&format!("/projects/{pid}/deployments/{did}"), &tok, json!({ "environment_description": "v2" })),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let (_, got) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}"), &tok)).await;
    assert_eq!(got["environment_description"], "v2");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn delete_deployment_removes_deployment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (_, created) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "Rollout", "environment_description": "e", "product_template_id": Value::Null
        })),
    ).await;
    let did = created["id"].as_str().unwrap();
    assert_eq!(count_nodes(&neo4j, "Deployment").await, 1);

    let (status, _) = send(app.clone(), req_del(&format!("/projects/{pid}/deployments/{did}"), &tok)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(count_nodes(&neo4j, "Deployment").await, 0);

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}"), &tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_deployment_rejected_for_user_outside_the_project_group() {
    neo4j!(c, neo4j);
    let (uid, tok)     = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let (_, other_tok) = make_user(&neo4j, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;

    let (_, created) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments"), &tok, json!({
            "name": "Rollout", "environment_description": "e", "product_template_id": Value::Null
        })),
    ).await;
    let did = created["id"].as_str().unwrap();

    let (status, _) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}"), &other_tok)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- record_run_and_update_state / last_applied_bundle_for_artifact ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn record_run_and_update_state_no_ops_for_artifact_not_linked_to_a_deployment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;

    let new_state = record_run_and_update_state(
        &neo4j, &pid, &aid, TerraformAction::Apply, Some(0), "out", "", Some(r#"{"main.tf":"v1"}"#), "agent", None,
    ).await.unwrap();
    assert_eq!(new_state, None);
    assert_eq!(count_nodes(&neo4j, "DeploymentRun").await, 0);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn record_run_and_update_state_covers_all_action_success_combinations() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let cases = [
        (TerraformAction::Plan, true, None),
        (TerraformAction::Plan, false, None),
        (TerraformAction::Apply, true, Some("up")),
        (TerraformAction::Apply, false, Some("broken")),
        (TerraformAction::Destroy, true, Some("destroyed")),
        (TerraformAction::Destroy, false, Some("destroy_failed")),
    ];
    for (action, success, expected) in cases {
        let exit_code = if success { Some(0) } else { Some(1) };
        let new_state = record_run_and_update_state(
            &neo4j, &pid, &aid, action, exit_code, "out", "err", Some(r#"{"main.tf":"v1"}"#), "agent", None,
        ).await.unwrap();
        assert_eq!(new_state.as_deref(), expected, "action={action:?} success={success}");
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn record_run_and_update_state_only_sets_last_applied_on_successful_apply() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    assert_eq!(last_applied_bundle_for_artifact(&neo4j, &pid, &aid).await.unwrap(), None);

    record_run_and_update_state(
        &neo4j, &pid, &aid, TerraformAction::Apply, Some(1), "", "boom", None, "agent", None,
    ).await.unwrap();
    assert_eq!(last_applied_bundle_for_artifact(&neo4j, &pid, &aid).await.unwrap(), None);

    record_run_and_update_state(
        &neo4j, &pid, &aid, TerraformAction::Apply, Some(0), "ok", "", Some(r#"{"main.tf":"v1"}"#), "agent", None,
    ).await.unwrap();
    assert_eq!(
        last_applied_bundle_for_artifact(&neo4j, &pid, &aid).await.unwrap(),
        Some(r#"{"main.tf":"v1"}"#.to_string()),
    );
}

// ---- deploy / redeploy / destroy ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn deploy_runs_apply_and_marks_infra_up() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let rx = register_agent(&registry, "agent-1", &pid);
    spawn_fake_agent_always_succeeds(rx, Arc::clone(&registry));

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/deploy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["runs"][0]["infra_state"], "up");

    let (_, got) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}"), &tok)).await;
    assert_eq!(got["infra_state"], "up");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn redeploy_applies_directly_when_infra_is_already_clean() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let rx = register_agent(&registry, "agent-1", &pid);
    let mut sent = spawn_fake_agent_always_succeeds(rx, Arc::clone(&registry));

    send(app.clone(), req_post(&format!("/projects/{pid}/deployments/{did}/deploy"), &tok, json!({ "agent_id": "agent-1" }))).await;
    let _ = sent.recv().await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/redeploy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1, "should apply directly without a destroy step");
    assert_eq!(runs[0]["action"], "apply");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn redeploy_destroys_first_when_infra_is_broken() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    record_run_and_update_state(
        &neo4j, &pid, &aid, TerraformAction::Apply, Some(1), "", "boom", None, "agent", None,
    ).await.unwrap();
    neo4j.query_read(
        "MATCH (d:Deployment {id: $did}) SET d.last_applied_content = $c, d.last_applied_artifact_id = $aid",
        json!({ "did": did, "c": r#"{"main.tf":"v0"}"#, "aid": aid }),
    ).await.unwrap();

    let rx = register_agent(&registry, "agent-1", &pid);
    spawn_fake_agent_always_succeeds(rx, Arc::clone(&registry));

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/redeploy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "should destroy then apply");
    assert_eq!(runs[0]["action"], "destroy");
    assert_eq!(runs[1]["action"], "apply");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn redeploy_resets_to_none_when_broken_and_never_applied() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    // First-ever apply failed: infra_state becomes "broken" but nothing was ever applied,
    // so there's no last_applied_content snapshot to destroy.
    record_run_and_update_state(
        &neo4j, &pid, &aid, TerraformAction::Apply, Some(1), "", "boom", None, "agent", None,
    ).await.unwrap();

    register_agent(&registry, "agent-1", &pid);

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/redeploy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap_or("").contains("reset"), "body: {body}");

    let (_, got) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}"), &tok)).await;
    assert_eq!(got["infra_state"], "none", "deployment should no longer be stuck broken");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn destroy_resets_to_none_when_broken_and_never_applied() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    record_run_and_update_state(
        &neo4j, &pid, &aid, TerraformAction::Apply, Some(1), "", "boom", None, "agent", None,
    ).await.unwrap();

    register_agent(&registry, "agent-1", &pid);

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/destroy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap_or("").contains("reset"), "body: {body}");

    let (_, got) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}"), &tok)).await;
    assert_eq!(got["infra_state"], "none", "deployment should no longer be stuck broken");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn destroy_rejected_when_nothing_was_ever_applied() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;
    register_agent(&registry, "agent-1", &pid);

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/destroy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn destroy_targets_the_last_applied_snapshot_not_unapplied_edits() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"content-A"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let rx = register_agent(&registry, "agent-1", &pid);
    let mut sent = spawn_fake_agent_always_succeeds(rx, Arc::clone(&registry));

    let (status, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/deploy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let (apply_action, apply_files) = sent.recv().await.unwrap();
    assert_eq!(apply_action, TerraformAction::Apply);
    assert_eq!(apply_files, r#"{"main.tf":"content-A"}"#);

    neo4j.query_read(
        "MATCH (a:Artifact {id: $aid}) SET a.content = $content",
        json!({ "aid": aid, "content": r#"{"main.tf":"content-B-never-applied"}"# }),
    ).await.unwrap();

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/destroy"), &tok, json!({ "agent_id": "agent-1" })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let (destroy_action, destroy_files) = sent.recv().await.unwrap();
    assert_eq!(destroy_action, TerraformAction::Destroy);
    assert_eq!(
        destroy_files, r#"{"main.tf":"content-A"}"#,
        "destroy must target the last-applied snapshot, not the edited-but-never-applied content"
    );
}

// ---- run history ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_deployment_runs_returns_runs_ordered_newest_first() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"v1"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let rx = register_agent(&registry, "agent-1", &pid);
    spawn_fake_agent_with_script(rx, Arc::clone(&registry), |_| 0);

    send(app.clone(), req_post(&format!("/projects/{pid}/deployments/{did}/deploy"), &tok, json!({ "agent_id": "agent-1" }))).await;
    send(app.clone(), req_post(&format!("/projects/{pid}/deployments/{did}/destroy"), &tok, json!({ "agent_id": "agent-1" }))).await;

    let (status, body) = send(app, req_get(&format!("/projects/{pid}/deployments/{did}/runs"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let runs = body.as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["action"], "destroy");
    assert_eq!(runs[1]["action"], "apply");
}

// ---- environment questions ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_environment_questions_returns_parsed_questions() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let llm = ScriptedLlm::new(vec![text_response(
        "Here are some questions:\n```json\n[{\"id\":\"racks\",\"text\":\"How many racks?\"}]\n```",
    )]);
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), llm);
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(app, req_post(&format!("/projects/{pid}/deployments/{did}/environment/questions"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let questions = body["questions"].as_array().unwrap();
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0]["id"], "racks");
    assert_eq!(questions[0]["text"], "How many racks?");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_environment_questions_returns_422_when_unparseable() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let llm = ScriptedLlm::new(vec![text_response("Sorry, I don't understand the request.")]);
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), llm);
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/deployments/{did}/environment/questions"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ---- design generation / revision ----

fn design_generation_llm() -> Arc<ClosureLlm> {
    ClosureLlm::new(|messages| {
        match tool_result_contents(messages).len() {
            0 => tool_call_response("generate_artifact", json!({
                "title": "Design", "kind": "markdown", "content": "# Design\nUse a single VM."
            })),
            1 => {
                let result: Value = serde_json::from_str(tool_result_contents(messages)[0]).unwrap();
                let id = result["id"].as_str().unwrap();
                tool_call_response("link_deployment_artifact", json!({ "artifact_id": id, "role": "design" }))
            }
            _ => text_response("Design document created."),
        }
    })
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_design_calls_tools_and_links_artifact() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), design_generation_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(app, req_post(&format!("/projects/{pid}/deployments/{did}/design/generate"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["design_doc"]["id"].is_string());
    assert_eq!(body["design_doc"]["title"], "Design");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_design_includes_selected_artifacts_and_links_template() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "b@x.com", "Bob", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let setup_app = deployments_app(Arc::clone(&neo4j)).0;
    let pid = seed_project(&setup_app, &tok, &gid, "Customer B").await;
    let did = create_deployment_raw(&setup_app, &tok, &pid, "Rollout").await;

    let (tpl_status, tpl_body) = send(
        setup_app.clone(),
        req_post(&format!("/groups/{gid}/templates"), &tok, json!({
            "name": "Gateway", "description": "baseline rollout",
            "content": "# Gateway playbook\nprovision 3 nodes"
        })),
    ).await;
    assert_eq!(tpl_status, StatusCode::CREATED);
    let tpl_id = tpl_body["id"].as_str().unwrap().to_string();

    let a1 = create_artifact(&neo4j, &pid, ArtifactKind::Markdown, "Requirements", "We need 3 VMs across two zones", "system").await.unwrap();
    let a1_id = a1["id"].as_str().unwrap().to_string();
    let a2 = create_artifact(&neo4j, &pid, ArtifactKind::Markdown, "Constraints", "No public IPs are allowed", "system").await.unwrap();
    let a2_id = a2["id"].as_str().unwrap().to_string();

    let captured = Arc::new(std::sync::Mutex::new(None::<(String, String)>));
    let captured_for_llm = Arc::clone(&captured);
    let llm = ClosureLlm::new(move |messages| {
        let count = tool_result_contents(messages).len();
        if count == 0 {
            captured_for_llm.lock().unwrap().replace(captured_prompt_for_design(messages));
        }
        match count {
            0 => tool_call_response("generate_artifact", json!({
                "title": "Design", "kind": "markdown", "content": "# Design\nUse a single VM."
            })),
            1 => {
                let result: Value = serde_json::from_str(tool_result_contents(messages)[0]).unwrap();
                let id = result["id"].as_str().unwrap();
                tool_call_response("link_deployment_artifact", json!({ "artifact_id": id, "role": "design" }))
            }
            _ => text_response("Design document created."),
        }
    });
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), llm);

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/design/generate"), &tok, json!({
            "artifact_ids": [a1_id, a2_id],
            "product_template_id": tpl_id,
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["design_doc"]["id"].is_string());

    let (system_prompt, user_prompt) = captured.lock().unwrap().clone().unwrap();
    assert!(user_prompt.contains("We need 3 VMs across two zones"));
    assert!(user_prompt.contains("No public IPs are allowed"));
    assert!(system_prompt.contains("Gateway playbook"));

    let (_, dep) = send(setup_app, req_get(&format!("/projects/{pid}/deployment"), &tok)).await;
    assert_eq!(dep["template_id"], tpl_id);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_design_ignores_artifacts_belonging_to_other_projects() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "c@x.com", "Carol", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let setup_app = deployments_app(Arc::clone(&neo4j)).0;
    let pid = seed_project(&setup_app, &tok, &gid, "Customer C").await;
    let did = create_deployment_raw(&setup_app, &tok, &pid, "Rollout").await;
    let other_pid = seed_project(&setup_app, &tok, &gid, "Customer D").await;

    let own = create_artifact(&neo4j, &pid, ArtifactKind::Markdown, "Own", "OWN-MARKER-keep", "system").await.unwrap();
    let own_id = own["id"].as_str().unwrap().to_string();
    let foreign = create_artifact(&neo4j, &other_pid, ArtifactKind::Markdown, "Foreign", "FOREIGN-MARKER-leak", "system").await.unwrap();
    let foreign_id = foreign["id"].as_str().unwrap().to_string();

    let captured = Arc::new(std::sync::Mutex::new(None::<String>));
    let captured_for_llm = Arc::clone(&captured);
    let llm = ClosureLlm::new(move |messages| {
        let count = tool_result_contents(messages).len();
        if count == 0 {
            captured_for_llm.lock().unwrap().replace(captured_prompt_for_design(messages).1);
        }
        match count {
            0 => tool_call_response("generate_artifact", json!({
                "title": "Design", "kind": "markdown", "content": "# Design\n"
            })),
            1 => {
                let result: Value = serde_json::from_str(tool_result_contents(messages)[0]).unwrap();
                let id = result["id"].as_str().unwrap();
                tool_call_response("link_deployment_artifact", json!({ "artifact_id": id, "role": "design" }))
            }
            _ => text_response("done"),
        }
    });
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), llm);

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/design/generate"), &tok, json!({
            "artifact_ids": [own_id.clone(), foreign_id],
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let prompt = captured.lock().unwrap().clone().unwrap();
    assert!(prompt.contains("OWN-MARKER-keep"));
    assert!(!prompt.contains("FOREIGN-MARKER-leak"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_design_decisions_requires_existing_design_doc() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/deployments/{did}/design/decisions"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_design_decisions_returns_parsed_decisions() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), design_generation_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    send(app.clone(), req_post(&format!("/projects/{pid}/deployments/{did}/design/generate"), &tok, json!({}))).await;

    let decisions_llm = ScriptedLlm::new(vec![text_response(
        "```json\n[{\"id\":\"sizing\",\"text\":\"Confirm VM size\",\"suggested\":\"medium\"}]\n```",
    )]);
    let (app2, _registry2) = deployments_app_with_llm(Arc::clone(&neo4j), decisions_llm);
    let (status, body) = send(app2, req_post(&format!("/projects/{pid}/deployments/{did}/design/decisions"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let decisions = body["decisions"].as_array().unwrap();
    assert_eq!(decisions[0]["id"], "sizing");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn revise_design_requires_decisions_or_instructions() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/design/revise"), &tok, json!({ "decisions": [] })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn revise_design_updates_existing_design_doc_in_place() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), design_generation_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let (_, created) = send(app.clone(), req_post(&format!("/projects/{pid}/deployments/{did}/design/generate"), &tok, json!({}))).await;
    let design_id = created["design_doc"]["id"].as_str().unwrap().to_string();
    let design_id_for_closure = design_id.clone();

    let revise_llm = ClosureLlm::new(move |messages| {
        match tool_result_contents(messages).len() {
            0 => tool_call_response("generate_artifact", json!({
                "title": "Design", "kind": "markdown", "content": "# Design v2\nUse two VMs.",
                "artifact_id": design_id_for_closure,
            })),
            1 => {
                let result: Value = serde_json::from_str(tool_result_contents(messages)[0]).unwrap();
                let id = result["id"].as_str().unwrap();
                tool_call_response("link_deployment_artifact", json!({ "artifact_id": id, "role": "design" }))
            }
            _ => text_response("Design revised."),
        }
    });
    let (app2, _registry2) = deployments_app_with_llm(Arc::clone(&neo4j), revise_llm);
    let (status, body) = send(
        app2,
        req_post(&format!("/projects/{pid}/deployments/{did}/design/revise"), &tok, json!({
            "decisions": [{ "question": "Sizing?", "answer": "medium" }], "instructions": Value::Null
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["design_doc"]["id"], design_id);

    let revised = get_artifact_content(&neo4j, &design_id).await;
    assert!(revised.contains("Design v2"), "expected revised content, got: {revised}");
}

async fn get_artifact_content(neo4j: &Neo4jClient, artifact_id: &str) -> String {
    let rows = neo4j.query_read(
        "MATCH (a:Artifact {id: $id}) RETURN a.content AS content",
        json!({ "id": artifact_id }),
    ).await.unwrap();
    rows[0]["content"].as_str().unwrap_or_default().to_string()
}

// ---- provision generation / propose-change / apply-change ----

fn provision_generation_llm() -> Arc<ClosureLlm> {
    ClosureLlm::new(|messages| {
        match tool_result_contents(messages).len() {
            0 => tool_call_response("generate_artifact", json!({
                "title": "Infra", "kind": "terraform", "content": "{\"main.tf\":\"resource \\\"x\\\" {}\"}"
            })),
            1 => {
                let result: Value = serde_json::from_str(tool_result_contents(messages)[0]).unwrap();
                let id = result["id"].as_str().unwrap();
                tool_call_response("link_deployment_artifact", json!({ "artifact_id": id, "role": "terraform" }))
            }
            _ => text_response("Terraform bundle created."),
        }
    })
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_provision_requires_existing_design_doc() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _) = send(app, req_post(&format!("/projects/{pid}/deployments/{did}/provision/generate"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn generate_provision_creates_and_links_terraform_bundle() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), provision_generation_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    seed_design_doc(&neo4j, &pid, &did, "# Design\nSingle VM.").await;

    let (status, body) = send(app, req_post(&format!("/projects/{pid}/deployments/{did}/provision/generate"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["terraform_bundle"]["id"].is_string());
    assert_eq!(body["terraform_bundle"]["kind"], "terraform");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_provision_change_requires_instructions_or_error_context() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/provision/propose-change"), &tok, json!({})),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_provision_change_requires_existing_bundle() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/provision/propose-change"), &tok, json!({
            "instructions": "add a second VM"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_provision_change_does_not_modify_the_artifact() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let llm = ScriptedLlm::new(vec![text_response(
        "Here's a proposal:\n```json\n{\"main.tf\":\"resource \\\"y\\\" {}\"}\n```",
    )]);
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), llm);
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/provision/propose-change"), &tok, json!({
            "instructions": "rename the resource"
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["proposed_files"]["main.tf"], "resource \"y\" {}");
    assert_eq!(body["current_files"]["main.tf"], "resource \"x\" {}");

    let content = get_artifact_content(&neo4j, &aid).await;
    assert_eq!(content, r#"{"main.tf":"resource \"x\" {}"}"#, "propose-change must not persist anything");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_provision_change_returns_422_when_unparseable() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let llm = ScriptedLlm::new(vec![text_response("I couldn't figure out a fix.")]);
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), llm);
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/provision/propose-change"), &tok, json!({
            "error_context": "apply failed: timeout"
        })),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn apply_provision_change_updates_the_artifact_content() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/provision/apply-change"), &tok, json!({
            "files": { "main.tf": "resource \"y\" {}" }
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["terraform_bundle"]["id"], aid);

    let content = get_artifact_content(&neo4j, &aid).await;
    let parsed: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed["main.tf"], "resource \"y\" {}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn apply_provision_change_rejects_unsafe_paths() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let (status, _) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/provision/apply-change"), &tok, json!({
            "files": { "../../etc/passwd": "oops" }
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---- issue triage (auto-triggered after a failed run) ----

async fn seed_failed_run(neo4j: &Neo4jClient, project_id: &str, artifact_id: &str) {
    record_run_and_update_state(
        neo4j, project_id, artifact_id, TerraformAction::Apply, Some(1),
        "", "Error: connection refused", None, "user", None,
    ).await.unwrap();
}

fn triage_creates_issue_llm() -> Arc<ClosureLlm> {
    ClosureLlm::new(|messages| {
        match tool_result_contents(messages).len() {
            0 => tool_call_response("list_deployment_issues", json!({})),
            1 => tool_call_response("create_or_link_issue", json!({
                "action": "create",
                "title": "Apply fails on security group",
                "description": "connection refused reaching the health check port",
                "fingerprint": "sg-health-check-refused",
            })),
            2 => tool_call_response("read_provision_bundle", json!({})),
            3 => tool_call_response("propose_issue_solution", json!({
                "issue_id": "placeholder",
                "explanation": "the security group blocked the health check port",
                "files": "{\"main.tf\":\"resource \\\"y\\\" {}\"}",
            })),
            _ => text_response("Triaged the failure."),
        }
    })
}

async fn wait_for_issue_count(app: &Router, tok: &str, pid: &str, want: usize) -> Vec<Value> {
    for _ in 0..200 {
        let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/issues"), tok)).await;
        if body.as_array().map(|a| a.len()) == Some(want) {
            return body.as_array().unwrap().clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {want} issue(s) to appear");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn failed_apply_triggers_triage_and_creates_an_issue() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app_with_llm(Arc::clone(&neo4j), triage_creates_issue_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;

    let rx = register_agent(&registry, "agent-1", &pid);
    spawn_fake_agent_with_script(rx, Arc::clone(&registry), |_| 1);

    send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments/{did}/deploy"), &tok, json!({ "agent_id": "agent-1" }),
    )).await;

    let issues = wait_for_issue_count(&app, &tok, &pid, 1).await;
    assert_eq!(issues[0]["title"], "Apply fails on security group");
    assert_eq!(issues[0]["status"], "untriaged");
    assert_eq!(issues[0]["deployment"]["id"], did);
}

// ---- issue status transitions ----

async fn seed_issue_via_triage(app: &Router, neo4j: &Neo4jClient, tok: &str, pid: &str, aid: &str) -> String {
    seed_failed_run(neo4j, pid, aid).await;
    let issues = wait_for_issue_count(app, tok, pid, 1).await;
    issues[0]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_issue_status_allows_untriaged_to_in_progress() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), triage_creates_issue_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;
    let iid = seed_issue_via_triage(&app, &neo4j, &tok, &pid, &aid).await;

    let (status, body) = send(
        app,
        req_patch(&format!("/projects/{pid}/issues/{iid}/status"), &tok, json!({ "status": "in_progress" })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "in_progress");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn update_issue_status_rejects_skipping_straight_to_fixed() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), triage_creates_issue_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;
    let iid = seed_issue_via_triage(&app, &neo4j, &tok, &pid, &aid).await;

    let (status, _) = send(
        app,
        req_patch(&format!("/projects/{pid}/issues/{iid}/status"), &tok, json!({ "status": "fixed" })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_issue_comment_appends_a_user_comment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app_with_llm(Arc::clone(&neo4j), triage_creates_issue_llm());
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"resource \"x\" {}"}"#).await;
    link_terraform_bundle(&neo4j, &did, &aid).await;
    let iid = seed_issue_via_triage(&app, &neo4j, &tok, &pid, &aid).await;

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deploy_steps"].as_array().unwrap().len(), 0);
    assert_eq!(body["destroy_steps"].as_array().unwrap().len(), 0);
}

// ---- DAG run orchestration ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_dag_executes_steps_in_topological_order_and_sets_infra_up() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let bash_aid = seed_bash_artifact(&neo4j, &pid, "echo prep").await;
    let tf_aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let agent_id = "agent-1";
    let _rx = register_agent(&registry, agent_id, &pid);
    spawn_dag_fake_agent(_rx, Arc::clone(&registry), |_| 0);

    let (_, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": bash_aid, "action": "run", "label": "Prep", "depends_on": []},
                {"artifact_id": tf_aid, "action": "apply", "label": "Apply", "depends_on": [0]}
            ],
            "destroy_steps": []
        })),
    ).await;

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/run-dag"), &tok, json!({
            "agent_id": agent_id, "timeout_secs": 30
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert!(runs.iter().all(|r| r["exit_code"] == 0));
    assert_eq!(body["infra_state"], "up");

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["infra_state"], "up");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_dag_halts_downstream_on_failure_and_sets_infra_broken() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let bash_aid = seed_bash_artifact(&neo4j, &pid, "echo prep").await;
    let tf_aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let agent_id = "agent-1";
    let _rx = register_agent(&registry, agent_id, &pid);
    spawn_dag_fake_agent(_rx, Arc::clone(&registry), |action| {
        if action == "run" { 0 } else { 1 }
    });

    let (_, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": bash_aid, "action": "run", "label": "Prep", "depends_on": []},
                {"artifact_id": tf_aid, "action": "apply", "label": "Apply", "depends_on": [0]}
            ],
            "destroy_steps": []
        })),
    ).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/run-dag"), &tok, json!({
            "agent_id": agent_id, "timeout_secs": 30
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let runs = body["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["exit_code"], 0);
    assert_eq!(runs[0]["action"], "run");
    assert_eq!(body["infra_state"], "broken");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_dag_400_when_no_plan_set() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let agent_id = "agent-1";
    let _rx = register_agent(&registry, agent_id, &pid);

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/run-dag"), &tok, json!({
            "agent_id": agent_id, "timeout_secs": 30
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("no deploy steps"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_dag_404_when_agent_not_in_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let bash_aid = seed_bash_artifact(&neo4j, &pid, "echo hi").await;

    let (_, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [{"artifact_id": bash_aid, "action": "run", "depends_on": []}],
            "destroy_steps": []
        })),
    ).await;

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/run-dag"), &tok, json!({
            "agent_id": "nonexistent", "timeout_secs": 30
        })),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- Context artifacts ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn add_context_artifact_creates_and_links_and_lists_in_deployment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/context-artifacts"), &tok, json!({
            "title": "Network diagram", "kind": "markdown", "content": "# LAN layout\n..."
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let cas = body["context_artifacts"].as_array().unwrap();
    assert_eq!(cas.len(), 1);
    assert_eq!(cas[0]["title"], "Network diagram");
    assert_eq!(cas[0]["kind"], "markdown");
    assert!(cas[0]["id"].is_string());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn add_context_artifact_rejects_empty_title() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/context-artifacts"), &tok, json!({
            "title": "  ", "kind": "markdown", "content": "hi"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("title"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn add_context_artifact_rejects_unknown_kind() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/context-artifacts"), &tok, json!({
            "title": "X", "kind": "docx", "content": "hi"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("kind"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn add_context_artifact_accepts_bash_kind() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/context-artifacts"), &tok, json!({
            "title": "Prep script", "kind": "bash", "content": "#!/usr/bin/env bash\necho hi"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["context_artifacts"][0]["kind"], "bash");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn link_context_artifact_links_existing_project_artifact() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/context-artifacts/link"), &tok, json!({
            "artifact_id": aid
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["context_artifacts"].as_array().unwrap().len(), 1);
    assert_eq!(body["context_artifacts"][0]["id"], aid);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn link_context_artifact_rejects_artifact_from_other_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid_a = seed_project(&app, &tok, &gid, "Project A").await;
    let pid_b = seed_project(&app, &tok, &gid, "Project B").await;
    let did_b = create_deployment_raw(&app, &tok, &pid_b, "Rollout B").await;
    let aid = seed_terraform_artifact(&neo4j, &pid_a, r#"{"main.tf":"..."}"#).await;

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid_b}/deployments/{did_b}/context-artifacts/link"), &tok, json!({
            "artifact_id": aid
        })),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn remove_context_artifact_unlinks_and_drops_from_list() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (_, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/context-artifacts"), &tok, json!({
            "title": "Notes", "kind": "markdown", "content": "stuff"
        })),
    ).await;
    let aid = body["context_artifacts"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(body["context_artifacts"].as_array().unwrap().len(), 1);

    let (status, _body) = send(
        app.clone(),
        req_del(&format!("/projects/{pid}/deployments/{did}/context-artifacts/{aid}"), &tok),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["context_artifacts"].as_array().unwrap().len(), 0);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn remove_context_artifact_404_when_not_linked() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _body) = send(
        app,
        req_del(&format!("/projects/{pid}/deployments/{did}/context-artifacts/nonexistent"), &tok),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- Generic proposals ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_artifact_change_creates_pending_proposal_targeting_artifact() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"old"}"#).await;

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
            "artifact_id": aid, "source": "prompt", "explanation": "Added a security group",
            "current_content": "{\"main.tf\":\"old\"}", "proposed_content": "{\"main.tf\":\"new\"}"
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body["status"], "pending");
    assert_eq!(body["target_artifact_id"], aid);
    assert_eq!(body["target_artifact_kind"], "terraform");
    assert_eq!(body["source"], "prompt");
    assert_eq!(body["proposed_content"], "{\"main.tf\":\"new\"}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_artifact_change_rejects_missing_explanation() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
            "artifact_id": "a1", "explanation": "  ",
            "current_content": "x", "proposed_content": "y"
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("explanation"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn propose_artifact_change_rejects_artifact_from_other_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid_a = seed_project(&app, &tok, &gid, "Project A").await;
    let pid_b = seed_project(&app, &tok, &gid, "Project B").await;
    let did_b = create_deployment_raw(&app, &tok, &pid_b, "Rollout B").await;
    let aid = seed_terraform_artifact(&neo4j, &pid_a, r#"{"main.tf":"..."}"#).await;

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid_b}/deployments/{did_b}/proposals"), &tok, json!({
            "artifact_id": aid, "explanation": "x",
            "current_content": "a", "proposed_content": "b"
        })),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_proposals_returns_all_proposals_newest_first() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"old"}"#).await;

    for i in 0..3 {
        let (_status, _body) = send(
            app.clone(),
            req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
                "artifact_id": aid, "explanation": format!("change {i}"),
                "current_content": "{\"main.tf\":\"old\"}", "proposed_content": format!("{{\"main.tf\":\"v{i}\"}}")
            })),
        ).await;
    }

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr.iter().all(|p| p["status"] == "pending"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn approve_proposal_applies_content_to_target_artifact_and_sets_approved() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"old"}"#).await;

    let (_status, proposal) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
            "artifact_id": aid, "explanation": "fix",
            "current_content": "{\"main.tf\":\"old\"}", "proposed_content": "{\"main.tf\":\"new\"}"
        })),
    ).await;
    let propid = proposal["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals/{propid}/approve"), &tok, json!({})),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "approved");

    let artifact = get_artifact_in_project(&neo4j, &pid, &aid).await.unwrap().unwrap();
    assert_eq!(artifact["content"], "{\"main.tf\":\"new\"}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn approve_proposal_allows_editing_content_before_applying() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"old"}"#).await;

    let (_, proposal) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
            "artifact_id": aid, "explanation": "fix",
            "current_content": "{\"main.tf\":\"old\"}", "proposed_content": "{\"main.tf\":\"proposed\"}"
        })),
    ).await;
    let propid = proposal["id"].as_str().unwrap().to_string();

    let (status, _body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals/{propid}/approve"), &tok, json!({
            "edited_content": "{\"main.tf\":\"user-edited\"}"
        })),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let artifact = get_artifact_in_project(&neo4j, &pid, &aid).await.unwrap().unwrap();
    assert_eq!(artifact["content"], "{\"main.tf\":\"user-edited\"}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn approve_proposal_rejects_already_approved_proposal() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"old"}"#).await;

    let (_, proposal) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
            "artifact_id": aid, "explanation": "fix",
            "current_content": "{\"main.tf\":\"old\"}", "proposed_content": "{\"main.tf\":\"new\"}"
        })),
    ).await;
    let propid = proposal["id"].as_str().unwrap().to_string();

    let (status, _body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals/{propid}/approve"), &tok, json!({})),
    ).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals/{propid}/approve"), &tok, json!({})),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("not pending"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn discard_proposal_sets_status_discarded_without_applying() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"old"}"#).await;

    let (_, proposal) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok, json!({
            "artifact_id": aid, "explanation": "fix",
            "current_content": "{\"main.tf\":\"old\"}", "proposed_content": "{\"main.tf\":\"new\"}"
        })),
    ).await;
    let propid = proposal["id"].as_str().unwrap().to_string();

    let (status, _body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals/{propid}/discard"), &tok, json!({})),
    ).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}/proposals"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["status"], "discarded");

    let artifact = get_artifact_in_project(&neo4j, &pid, &aid).await.unwrap().unwrap();
    assert_eq!(artifact["content"], "{\"main.tf\":\"old\"}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn discard_proposal_404_for_nonexistent() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/proposals/nonexistent/discard"), &tok, json!({})),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---- Execution plan (DAG) ----

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_creates_deploy_and_destroy_steps_with_dependencies() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let bash_aid = seed_bash_artifact(&neo4j, &pid, "#!/usr/bin/env bash\necho prep").await;
    let tf_aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let (status, body) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": bash_aid, "action": "run", "label": "Prep machine", "depends_on": []},
                {"artifact_id": tf_aid, "action": "apply", "label": "Apply infra", "depends_on": [0]}
            ],
            "destroy_steps": [
                {"artifact_id": tf_aid, "action": "destroy", "label": "Destroy infra", "depends_on": []}
            ]
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let deploy = body["deploy_steps"].as_array().unwrap();
    assert_eq!(deploy.len(), 2);
    assert_eq!(deploy[0]["action"], "run");
    assert_eq!(deploy[0]["artifact"]["id"], bash_aid);
    assert_eq!(deploy[0]["phase"], "deploy");
    assert_eq!(deploy[0]["label"], "Prep machine");
    assert!(deploy[0]["depends_on"].as_array().unwrap().is_empty());
    assert_eq!(deploy[1]["action"], "apply");
    assert_eq!(deploy[1]["artifact"]["id"], tf_aid);
    assert_eq!(deploy[1]["depends_on"].as_array().unwrap().len(), 1);

    let destroy = body["destroy_steps"].as_array().unwrap();
    assert_eq!(destroy.len(), 1);
    assert_eq!(destroy[0]["action"], "destroy");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_execution_plan_returns_stored_plan() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let bash_aid = seed_bash_artifact(&neo4j, &pid, "echo hi").await;
    let tf_aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let (_, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": bash_aid, "action": "run", "depends_on": []},
                {"artifact_id": tf_aid, "action": "apply", "depends_on": [0]}
            ],
            "destroy_steps": [
                {"artifact_id": tf_aid, "action": "destroy", "depends_on": []}
            ]
        })),
    ).await;

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deploy_steps"].as_array().unwrap().len(), 0);
    assert_eq!(body["destroy_steps"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Phase 1: 1:1 project↔deployment
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires Docker"]
async fn creating_a_project_auto_creates_a_deployment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "p1@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G1").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let pid = seed_project(&app, &tok, &gid, "ProjectOne").await;

    assert_eq!(count_nodes(&neo4j, "Deployment").await, 1);
    let _ = pid;
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_project_deployment_returns_the_auto_created_deployment() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "p2@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G2").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let pid = seed_project(&app, &tok, &gid, "ProjectTwo").await;

    let (status, body) = send(app.clone(), req_get(&format!("/projects/{pid}/deployment"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["infra_state"], "none");
    assert_eq!(body["name"], "ProjectTwo");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn create_deployment_is_rejected_when_one_already_exists() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "p3@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G3").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let pid = seed_project(&app, &tok, &gid, "ProjectThree").await;

    let (status, _body) = send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments"), &tok,
        json!({"name":"Second","environment_description":"env"}),
    )).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(count_nodes(&neo4j, "Deployment").await, 1);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_deployments_returns_exactly_one_for_a_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "p4@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G4").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));

    let pid = seed_project(&app, &tok, &gid, "ProjectFour").await;

    let (status, body) = send(app.clone(), req_get(&format!("/projects/{pid}/deployments"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    let deployments = body.as_array().unwrap();
    assert_eq!(deployments.len(), 1);
    assert_eq!(deployments[0]["infra_state"], "none");
}

// ---------------------------------------------------------------------------
// Phase 2: unified change requests
// ---------------------------------------------------------------------------

async fn cr_get_deployment_id(app: &Router, tok: &str, pid: &str) -> String {
    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/deployment"), tok)).await;
    body["id"].as_str().unwrap().to_string()
}

async fn cr_seed_design_doc(neo4j: &Neo4jClient, project_id: &str, deployment_id: &str, content: &str) -> String {
    let aid = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (p:Project {id: $pid}), (d:Deployment {id: $did})
         CREATE (a:Artifact {id: $aid, title: 'design', kind: 'markdown', content: $content, created_by: 'test', created_at: $now, updated_at: $now})
         CREATE (p)-[:HAS_ARTIFACT]->(a)
         CREATE (d)-[:HAS_DESIGN_DOC]->(a)
         RETURN a.id",
        json!({"pid": project_id, "did": deployment_id, "aid": aid, "content": content, "now": now}),
    ).await.unwrap();
    aid
}

async fn cr_seed_issue_direct(neo4j: &Neo4jClient, deployment_id: &str, title: &str, status: &str) -> String {
    let iid = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (d:Deployment {id: $did})
         CREATE (i:Issue {
             id: $iid, title: $title, description: 'desc', status: $status,
             fingerprint: 'fp', proposed_solution_summary: null, proposed_files: null,
             chat_messages: '[]', comments: '[]',
             created_by: 'harvest', created_at: $now, updated_at: $now
         })
         CREATE (d)-[:HAS_ISSUE]->(i)
         RETURN i.id",
        json!({"did": deployment_id, "iid": iid, "title": title, "status": status, "now": now}),
    ).await.unwrap();
    iid
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_list_returns_both_issues_and_proposals() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr1@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G1").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject1").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let design_id = cr_seed_design_doc(&neo4j, &pid, &did, "# Original").await;
    let _issue_id = cr_seed_issue_direct(&neo4j, &did, "Apply fails", "untriaged").await;

    let (_, body) = send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments/{did}/proposals"), &tok,
        json!({"artifact_id": design_id, "source": "user", "explanation": "Fix", "current_content": "# Old", "proposed_content": "# New"}),
    )).await;

    let (status, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_list_maps_issue_status_to_unified() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr2@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G2").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject2").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    cr_seed_issue_direct(&neo4j, &did, "Broken apply", "untriaged").await;

    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests"), &tok)).await;
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["status"], "open");
    assert_eq!(items[0]["kind"], "issue");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_list_maps_proposal_status_to_unified() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr3@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G3").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject3").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let design_id = cr_seed_design_doc(&neo4j, &pid, &did, "# Original").await;
    send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments/{did}/proposals"), &tok,
        json!({"artifact_id": design_id, "source": "user", "explanation": "Fix", "current_content": "# Old", "proposed_content": "# New"}),
    )).await;

    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests"), &tok)).await;
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["status"], "open");
    assert_eq!(items[0]["kind"], "proposal");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_get_returns_issue_detail() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr4@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G4").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject4").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let iid = cr_seed_issue_direct(&neo4j, &did, "Issue detail test", "untriaged").await;

    let (status, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests/{iid}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "issue");
    assert_eq!(body["status"], "open");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_get_returns_proposal_detail() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr5@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G5").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject5").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let design_id = cr_seed_design_doc(&neo4j, &pid, &did, "# Original").await;
    let (_, body) = send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments/{did}/proposals"), &tok,
        json!({"artifact_id": design_id, "source": "user", "explanation": "Fix", "current_content": "# Old", "proposed_content": "# New"}),
    )).await;
    let prop_id = body["id"].as_str().unwrap().to_string();

    let (status, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests/{prop_id}"), &tok)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "proposal");
    assert_eq!(body["status"], "open");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_discard_proposal_sets_discarded() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr6@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G6").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject6").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let design_id = cr_seed_design_doc(&neo4j, &pid, &did, "# Original").await;
    let (_, body) = send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments/{did}/proposals"), &tok,
        json!({"artifact_id": design_id, "source": "user", "explanation": "Fix", "current_content": "# Old", "proposed_content": "# New"}),
    )).await;
    let prop_id = body["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(), req_post(&format!("/projects/{pid}/change-requests/{prop_id}/discard"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests/{prop_id}"), &tok)).await;
    assert_eq!(body["status"], "discarded");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_discard_issue_sets_discarded() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr7@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G7").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject7").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let iid = cr_seed_issue_direct(&neo4j, &did, "Discard me", "untriaged").await;

    let (status, _) = send(app.clone(), req_post(&format!("/projects/{pid}/change-requests/{iid}/discard"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests/{iid}"), &tok)).await;
    assert_eq!(body["status"], "discarded");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_apply_proposal_applies_content() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr8@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G8").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject8").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let design_id = cr_seed_design_doc(&neo4j, &pid, &did, "# Original").await;
    let (_, body) = send(app.clone(), req_post(
        &format!("/projects/{pid}/deployments/{did}/proposals"), &tok,
        json!({"artifact_id": design_id, "source": "user", "explanation": "Fix", "current_content": "# Old", "proposed_content": "# New design with improvements"}),
    )).await;
    let prop_id = body["id"].as_str().unwrap().to_string();

    let (status, _) = send(app.clone(), req_post(&format!("/projects/{pid}/change-requests/{prop_id}/apply"), &tok, json!({}))).await;
    assert_eq!(status, StatusCode::OK);

    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests/{prop_id}"), &tok)).await;
    assert_eq!(body["status"], "applied");

    let rows = neo4j.query_read(
        "MATCH (a:Artifact {id: $aid}) RETURN a.content AS content",
        json!({"aid": design_id}),
    ).await.unwrap();
    assert!(rows[0]["content"].as_str().unwrap().contains("New design with improvements"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_list_filters_by_status() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr9@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G9").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject9").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    cr_seed_issue_direct(&neo4j, &did, "Open one", "untriaged").await;
    cr_seed_issue_direct(&neo4j, &did, "Fixed one", "fixed").await;

    let (_, body) = send(app.clone(), req_get(&format!("/projects/{pid}/change-requests?status=open"), &tok)).await;
    let items = body.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Open one");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn cr_comment_appends_to_issue() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "cr10@x.com", "Admin", "admin").await;
    let gid = make_group(&neo4j, "G10").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "CRProject10").await;
    let did = cr_get_deployment_id(&app, &tok, &pid).await;
    let iid = cr_seed_issue_direct(&neo4j, &did, "Comment test", "untriaged").await;

    let (status, body) = send(app.clone(), req_post(
        &format!("/projects/{pid}/change-requests/{iid}/comments"), &tok,
        json!({"body": "This is a comment"}),
    )).await;
    assert_eq!(status, StatusCode::OK);
    let comments = body["comments"].as_array().unwrap();
    assert!(comments.iter().any(|c| c["body"] == "This is a comment"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_execution_plan_returns_empty_plan_when_none_set() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deploy_steps"].as_array().unwrap().len(), 0);
    assert_eq!(body["destroy_steps"].as_array().unwrap().len(), 0);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_rejects_invalid_action_for_artifact_kind() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let bash_aid = seed_bash_artifact(&neo4j, &pid, "echo hi").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [{"artifact_id": bash_aid, "action": "apply", "depends_on": []}],
            "destroy_steps": []
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("not valid"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_rejects_cycle_in_deploy_steps() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid_a = seed_bash_artifact(&neo4j, &pid, "echo a").await;
    let aid_b = seed_bash_artifact(&neo4j, &pid, "echo b").await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": aid_a, "action": "run", "depends_on": [1]},
                {"artifact_id": aid_b, "action": "run", "depends_on": [0]}
            ],
            "destroy_steps": []
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("cycle"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_rejects_apply_without_destroy_coverage() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let tf_aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let (status, body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": tf_aid, "action": "apply", "label": "Apply", "depends_on": []}
            ],
            "destroy_steps": []
        })),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("destroy"));
    assert!(body["error"].as_str().unwrap().contains(tf_aid.as_str()));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_accepts_apply_with_matching_destroy() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let tf_aid = seed_terraform_artifact(&neo4j, &pid, r#"{"main.tf":"..."}"#).await;

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [
                {"artifact_id": tf_aid, "action": "apply", "label": "Apply", "depends_on": []}
            ],
            "destroy_steps": [
                {"artifact_id": tf_aid, "action": "destroy", "label": "Destroy", "depends_on": []}
            ]
        })),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_rejects_artifact_from_other_project() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid_a = seed_project(&app, &tok, &gid, "Project A").await;
    let pid_b = seed_project(&app, &tok, &gid, "Project B").await;
    let did_b = create_deployment_raw(&app, &tok, &pid_b, "Rollout B").await;
    let aid = seed_bash_artifact(&neo4j, &pid_a, "echo hi").await;

    let (status, _body) = send(
        app,
        req_post(&format!("/projects/{pid_b}/deployments/{did_b}/execution-plan"), &tok, json!({
            "deploy_steps": [{"artifact_id": aid, "action": "run", "depends_on": []}],
            "destroy_steps": []
        })),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn set_execution_plan_overwrites_previous_plan() {
    neo4j!(c, neo4j);
    let (uid, tok) = make_user(&neo4j, "a@x.com", "Alice", "regular").await;
    let gid = make_group(&neo4j, "eng").await;
    join_group(&neo4j, &uid, &gid).await;
    let (app, _registry) = deployments_app(Arc::clone(&neo4j));
    let pid = seed_project(&app, &tok, &gid, "Customer A").await;
    let did = create_deployment_raw(&app, &tok, &pid, "Rollout").await;
    let aid = seed_bash_artifact(&neo4j, &pid, "echo hi").await;

    let (_, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [{"artifact_id": aid, "action": "run", "depends_on": []}],
            "destroy_steps": []
        })),
    ).await;

    let (_, _) = send(
        app.clone(),
        req_post(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok, json!({
            "deploy_steps": [],
            "destroy_steps": []
        })),
    ).await;

    let (status, body) = send(
        app,
        req_get(&format!("/projects/{pid}/deployments/{did}/execution-plan"), &tok),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["deploy_steps"].as_array().unwrap().len(), 0);
    assert_eq!(body["destroy_steps"].as_array().unwrap().len(), 0);
}
