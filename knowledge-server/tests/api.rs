use std::{path::PathBuf, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::{get, post},
    Json, Router,
};
use http_body_util::BodyExt as _;
use serde_json::{json, Value};
use tower::ServiceExt as _;

use std::collections::HashMap;
use tokio::sync::RwLock;

use knowledge_server::{
    agent::{Agent, tool::Tool},
    api::{
        docs::{handle_get_index, handle_get_page},
        query::{handle_query, handle_query_stream},
        repositories::handle_list_repositories,
        tool_description::handle_tool_description,
        GraphState,
    },
    llm::{
        LlmProvider,
        types::{LlmResponse, Message, ToolDefinition},
    },
    neo4j::Neo4jClient,
};


struct FixedTextLlm(String);

impl FixedTextLlm {
    fn new(text: impl Into<String>) -> Arc<Self> {
        Arc::new(Self(text.into()))
    }
}

#[async_trait]
impl LlmProvider for FixedTextLlm {
    async fn chat(&self, _messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
        Ok(LlmResponse::Message { text: self.0.clone() })
    }
}

struct ErrorLlm;

#[async_trait]
impl LlmProvider for ErrorLlm {
    async fn chat(&self, _messages: &[Message], _tools: &[ToolDefinition]) -> Result<LlmResponse> {
        Err(anyhow::anyhow!("simulated LLM failure"))
    }
}


fn query_app(agent: Arc<Agent>) -> Router {
    Router::new()
        .route("/query", post(handle_query))
        .route("/query/stream", post(handle_query_stream))
        .route("/tool-description", post(handle_tool_description))
        .route("/health", get(|| async { Json(json!({ "status": "ok" })) }))
        .with_state(agent)
}

fn repos_app(neo4j: Arc<Neo4jClient>) -> Router {
    let state = Arc::new(GraphState {
        neo4j,
        cache: Arc::new(RwLock::new(HashMap::new())),
    });
    Router::new()
        .route("/repositories", get(handle_list_repositories))
        .with_state(state)
}

fn make_agent(text: &str) -> Arc<Agent> {
    Arc::new(Agent::new(FixedTextLlm::new(text), vec![], 5))
}

fn make_error_agent() -> Arc<Agent> {
    Arc::new(Agent::new(Arc::new(ErrorLlm), vec![], 5))
}


async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_status_json(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let json = body_json(resp).await;
    (status, json)
}

fn post_query(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/query")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn get_req(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}


#[tokio::test]
async fn health_returns_200() {
    let app = query_app(make_agent("irrelevant"));
    let resp = app.oneshot(get_req("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_body_is_ok_json() {
    let app = query_app(make_agent("irrelevant"));
    let resp = app.oneshot(get_req("/health")).await.unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["status"], "ok");
}


#[tokio::test]
async fn query_valid_request_returns_200() {
    let app = query_app(make_agent("all good"));
    let (status, _) = body_status_json(app, post_query(json!({ "query": "what is main?" }))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn query_response_contains_answer_field() {
    let app = query_app(make_agent("here is the answer"));
    let (_, body) = body_status_json(app, post_query(json!({ "query": "describe alpha" }))).await;
    assert_eq!(body["answer"], "here is the answer");
}

#[tokio::test]
async fn query_response_contains_tool_calls_made_field() {
    let app = query_app(make_agent("done"));
    let (_, body) = body_status_json(app, post_query(json!({ "query": "hi" }))).await;
    assert!(body.get("tool_calls_made").is_some(), "missing tool_calls_made field");
    assert_eq!(body["tool_calls_made"], 0);
}

#[tokio::test]
async fn query_response_contains_sources_field() {
    let app = query_app(make_agent("done"));
    let (_, body) = body_status_json(app, post_query(json!({ "query": "hi" }))).await;
    assert!(body["sources"].is_array(), "sources should be an array");
}

#[tokio::test]
async fn query_sources_populated_from_citations_in_answer() {
    let answer = "See [myrepo:v1.0:src/lib.rs:42] for details.";
    let app = query_app(make_agent(answer));
    let (_, body) = body_status_json(app, post_query(json!({ "query": "hi" }))).await;
    let sources = body["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0]["repo"], "myrepo");
    assert_eq!(sources[0]["version"], "v1.0");
    assert_eq!(sources[0]["file"], "src/lib.rs");
    assert_eq!(sources[0]["line"], 42);
}

#[tokio::test]
async fn query_with_optional_repositories_field_returns_200() {
    let app = query_app(make_agent("ok"));
    let req = post_query(json!({
        "query": "hi",
        "repositories": ["myrepo"],
        "versions": ["v1.0"]
    }));
    let (status, _) = body_status_json(app, req).await;
    assert_eq!(status, StatusCode::OK);
}


#[tokio::test]
async fn query_missing_query_field_returns_422() {
    let app = query_app(make_agent("irrelevant"));
    let req = post_query(json!({ "not_query": "oops" }));
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn query_empty_body_returns_4xx() {
    let app = query_app(make_agent("irrelevant"));
    let req = Request::builder()
        .method("POST")
        .uri("/query")
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.status().is_client_error(),
        "expected a 4xx status, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn query_non_json_content_type_returns_415_or_422() {
    let app = query_app(make_agent("irrelevant"));
    let req = Request::builder()
        .method("POST")
        .uri("/query")
        .header("content-type", "text/plain")
        .body(Body::from(r#"{"query":"hi"}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::UNSUPPORTED_MEDIA_TYPE
            || resp.status() == StatusCode::UNPROCESSABLE_ENTITY,
        "expected 415 or 422, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn query_agent_error_returns_500() {
    let app = query_app(make_error_agent());
    let resp = app
        .oneshot(post_query(json!({ "query": "hi" })))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(body.contains("simulated LLM failure"), "body: {body}");
}


#[tokio::test]
async fn tool_description_returns_200_with_description_field() {
    let app = query_app(make_agent("searching for authenticate"));
    let req = Request::builder()
        .method("POST")
        .uri("/tool-description")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"search_symbols","input":{"query":"authenticate"}}"#))
        .unwrap();
    let (status, body) = body_status_json(app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("description").is_some(), "missing description field");
    assert_eq!(body["description"], "searching for authenticate");
}

#[tokio::test]
async fn tool_description_missing_body_returns_4xx() {
    let app = query_app(make_agent("irrelevant"));
    let req = Request::builder()
        .method("POST")
        .uri("/tool-description")
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_client_error());
}

#[tokio::test]
async fn tool_description_missing_name_returns_4xx() {
    let app = query_app(make_agent("irrelevant"));
    let req = Request::builder()
        .method("POST")
        .uri("/tool-description")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"input":{}}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert!(resp.status().is_client_error());
}


fn post_query_stream(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/query/stream")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn parse_sse_events(body: &str) -> Vec<Value> {
    body.split("\n\n")
        .filter_map(|block| {
            let line = block.lines().find(|l| l.starts_with("data: "))?;
            serde_json::from_str(&line["data: ".len()..]).ok()
        })
        .collect()
}

#[tokio::test]
async fn stream_returns_200_with_event_stream_content_type() {
    let app = query_app(make_agent("all done"));
    let resp = app.oneshot(post_query_stream(json!({ "query": "hi" }))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/event-stream"), "content-type: {ct}");
}

#[tokio::test]
async fn stream_emits_done_event_with_answer() {
    let app = query_app(make_agent("the answer is 42"));
    let resp = app.oneshot(post_query_stream(json!({ "query": "what is the answer?" }))).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&bytes).unwrap();
    let events = parse_sse_events(body);
    let done = events.iter().find(|e| e["type"] == "done").expect("expected a done event");
    assert_eq!(done["answer"], "the answer is 42");
}

#[tokio::test]
async fn stream_done_event_contains_sources_and_tool_count() {
    let answer = "See [repo:v1:src/lib.rs:10] here.";
    let app = query_app(make_agent(answer));
    let resp = app.oneshot(post_query_stream(json!({ "query": "q" }))).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&bytes).unwrap();
    let events = parse_sse_events(body);
    let done = events.iter().find(|e| e["type"] == "done").expect("expected done");
    assert!(done["sources"].as_array().unwrap().len() >= 1);
    assert_eq!(done["tool_calls_made"], 0);
}

#[tokio::test]
async fn stream_missing_query_field_returns_422() {
    let app = query_app(make_agent("irrelevant"));
    let resp = app.oneshot(post_query_stream(json!({ "not_query": "oops" }))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}


use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j, Neo4jImageExt as _};
use neo4rs::{query, Graph};

async fn seed_one_repo(graph: &Graph) {
    let stmts = [
        "CREATE (:Repository {name: 'testrepo', url: 'https://example.com/t.git'})",
        "CREATE (:Version   {repo: 'testrepo', tag: 'v1.0', timestamp: 1000, ingested: true})",
        "MATCH  (r:Repository {name:'testrepo'}),(v:Version {repo:'testrepo',tag:'v1.0'}) \
         CREATE (r)-[:HAS_VERSION]->(v)",
    ];
    for stmt in stmts {
        graph.run(query(stmt)).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn repositories_empty_graph_returns_empty_array() {
    let container = Neo4j::default().start().await;
    let uri  = container.image().bolt_uri_ipv4();
    let user = container.image().user().unwrap_or("neo4j");
    let pass = container.image().password().unwrap_or("neo");
    let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

    let app = repos_app(neo4j);
    let (status, body) = body_status_json(app, get_req("/repositories")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!([]));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn repositories_returns_ingested_repos() {
    let container = Neo4j::default().start().await;
    let uri  = container.image().bolt_uri_ipv4();
    let user = container.image().user().unwrap_or("neo4j");
    let pass = container.image().password().unwrap_or("neo");
    let graph = Graph::new(&uri, user, pass).await.unwrap();
    seed_one_repo(&graph).await;
    let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

    let app = repos_app(neo4j);
    let (status, body) = body_status_json(app, get_req("/repositories")).await;
    assert_eq!(status, StatusCode::OK);
    let repos = body.as_array().unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0]["name"], "testrepo");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn repositories_returns_versions_for_repo() {
    let container = Neo4j::default().start().await;
    let uri  = container.image().bolt_uri_ipv4();
    let user = container.image().user().unwrap_or("neo4j");
    let pass = container.image().password().unwrap_or("neo");
    let graph = Graph::new(&uri, user, pass).await.unwrap();
    seed_one_repo(&graph).await;
    let neo4j = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());

    let app = repos_app(neo4j);
    let (_, body) = body_status_json(app, get_req("/repositories")).await;
    let versions = body[0]["versions"].as_array().unwrap();
    assert!(
        versions.iter().any(|v| v.as_str() == Some("v1.0")),
        "versions: {versions:?}"
    );
}


fn docs_app(docs_dir: PathBuf) -> Router {
    Router::new()
        .route("/docs/:repo/:version", get(handle_get_index))
        .route("/docs/:repo/:version/:section/*filename", get(handle_get_page))
        .with_state(Arc::new(docs_dir))
}

async fn body_text(resp: axum::response::Response) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn docs_index_missing_repo_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = docs_app(dir.path().to_path_buf());
    let resp = app.oneshot(get_req("/docs/missing-repo/v1.0")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn docs_index_returns_sections_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("testrepo").join("v1.0");
    std::fs::create_dir_all(&path).unwrap();
    let index = json!({
        "repo": "testrepo",
        "version": "v1.0",
        "sections": {
            "tutorials": [{"filename": "getting-started.md", "title": "Getting Started"}],
            "how-to-guides": [],
            "explanations": [],
            "reference": [{"filename": "api.md", "title": "API Reference"}]
        }
    });
    std::fs::write(path.join("index.json"), serde_json::to_string(&index).unwrap()).unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let (status, body) = body_status_json(app, get_req("/docs/testrepo/v1.0")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["repo"], "testrepo");
    assert_eq!(body["version"], "v1.0");
    let tutorials = body["sections"]["tutorials"].as_array().unwrap();
    assert_eq!(tutorials.len(), 1);
    assert_eq!(tutorials[0]["filename"], "getting-started.md");
    assert_eq!(tutorials[0]["title"], "Getting Started");
}

#[tokio::test]
async fn docs_index_different_repo_version_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("testrepo").join("v1.0");
    std::fs::create_dir_all(&path).unwrap();
    let index = json!({"repo":"testrepo","version":"v1.0","sections":{"tutorials":[],"how-to-guides":[],"explanations":[],"reference":[]}});
    std::fs::write(path.join("index.json"), serde_json::to_string(&index).unwrap()).unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let resp = app.oneshot(get_req("/docs/testrepo/v2.0")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn docs_page_returns_markdown_content() {
    let dir = tempfile::tempdir().unwrap();
    let section_path = dir.path().join("testrepo").join("v1.0").join("tutorials");
    std::fs::create_dir_all(&section_path).unwrap();
    std::fs::write(section_path.join("getting-started.md"), "# Getting Started\n\nHello!").unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/testrepo/v1.0/tutorials/getting-started.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/markdown"), "content-type: {ct}");
    let body = body_text(resp).await;
    assert!(body.contains("# Getting Started"));
    assert!(body.contains("Hello!"));
}

#[tokio::test]
async fn docs_page_missing_file_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("testrepo").join("v1.0").join("tutorials")).unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/testrepo/v1.0/tutorials/nonexistent.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn docs_page_unknown_section_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/testrepo/v1.0/unknown-section/file.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn docs_page_path_traversal_returns_400() {
    let dir = tempfile::tempdir().unwrap();
    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/testrepo/v1.0/tutorials/..%2F..%2Fetc%2Fpasswd"))
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
        "expected 400 or 404, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn docs_page_reference_section_works() {
    let dir = tempfile::tempdir().unwrap();
    let section_path = dir.path().join("myrepo").join("v2.0").join("reference");
    std::fs::create_dir_all(&section_path).unwrap();
    std::fs::write(section_path.join("api.md"), "# API Reference\n\nEndpoints here.").unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/myrepo/v2.0/reference/api.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    assert!(body.contains("# API Reference"));
}

#[tokio::test]
async fn docs_page_howto_section_works() {
    let dir = tempfile::tempdir().unwrap();
    let section_path = dir.path().join("myrepo").join("v1.0").join("how-to-guides");
    std::fs::create_dir_all(&section_path).unwrap();
    std::fs::write(section_path.join("install.md"), "# How to Install").unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/myrepo/v1.0/how-to-guides/install.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn docs_page_explanations_section_works() {
    let dir = tempfile::tempdir().unwrap();
    let section_path = dir.path().join("myrepo").join("v1.0").join("explanations");
    std::fs::create_dir_all(&section_path).unwrap();
    std::fs::write(section_path.join("architecture.md"), "# Architecture").unwrap();

    let app = docs_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(get_req("/docs/myrepo/v1.0/explanations/architecture.md"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
