use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

use harvest_db::test_support::TestDb;
use harvest_search::Scope;
use knowledge_harvester::graph::model::{CallRef, ClassNode, FunctionNode, ParsedFile};
use knowledge_harvester::graph::writer::GraphWriter;
use knowledge_server::agent::tool::Tool;
use knowledge_server::config::{SearchConfig, SearchMode};
use knowledge_server::llm::types::{LlmResponse, Message, ModelInfo, ToolDefinition, Usage};
use knowledge_server::llm::LlmProvider;
use knowledge_server::search::code_tool::SearchCodeTool;
use knowledge_server::search::record::Recorder;
use knowledge_server::search::wiring;

struct CountingLlm {
    calls: AtomicUsize,
    reply: Option<&'static str>,
}

impl CountingLlm {
    fn answering(reply: &'static str) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            reply: Some(reply),
        })
    }

    fn failing() -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            reply: None,
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for CountingLlm {
    fn id(&self) -> &str {
        "counting"
    }
    fn kind(&self) -> &str {
        "mock"
    }
    fn default_model(&self) -> &str {
        "mock-model"
    }
    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }
    async fn chat_with(
        &self,
        _model: Option<&str>,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.reply {
            Some(text) => Ok(LlmResponse::Message {
                text: text.into(),
                usage: Usage::default(),
            }),
            None => Err(anyhow::anyhow!("no key configured")),
        }
    }
}

async fn warm(service: &knowledge_server::search::SearchService) {
    let scope = Scope::new("cinder", "v1");
    let _ = service.registry.ensure(&scope).await;
}

fn class(name: &str, file: &str, source: &str, bases: &[&str]) -> ClassNode {
    ClassNode {
        repo: "cinder".into(),
        version: "v1".into(),
        file: file.into(),
        name: name.into(),
        kind: "class".into(),
        start_line: 1,
        end_line: source.lines().count() as u32,
        source: source.into(),
        bases: bases.iter().map(|b| b.to_string()).collect(),
        traits: vec![],
        embeds: vec![],
        uses: vec![],
        docstring: None,
        decorators: vec![],
    }
}

fn function(name: &str, file: &str, source: &str, calls: &[&str]) -> FunctionNode {
    FunctionNode {
        repo: "cinder".into(),
        version: "v1".into(),
        file: file.into(),
        name: name.into(),
        kind: "function".into(),
        signature: source.lines().next().unwrap_or_default().into(),
        start_line: 1,
        end_line: source.lines().count() as u32,
        source: source.into(),
        impl_type: None,
        docstring: None,
        calls: calls
            .iter()
            .map(|c| CallRef {
                callee: c.to_string(),
                line: 2,
            })
            .collect(),
    }
}

async fn seed(db: &harvest_db::Db) {
    let writer = GraphWriter::new(db.clone());
    writer
        .upsert_repository("cinder", "https://example.com/cinder.git")
        .await
        .unwrap();
    writer
        .upsert_version("cinder", "v1", 1000, false)
        .await
        .unwrap();
    let files = vec![
        ParsedFile {
            path: "cinder/volume/driver.py".into(),
            language: "python".into(),
            classes: vec![class(
                "VolumeDriver",
                "cinder/volume/driver.py",
                "class VolumeDriver(object):\n    supports_multiattach = False\n",
                &["object"],
            )],
            ..ParsedFile::default()
        },
        ParsedFile {
            path: "cinder/volume/drivers/lvm.py".into(),
            language: "python".into(),
            classes: vec![class(
                "LVMVolumeDriver",
                "cinder/volume/drivers/lvm.py",
                "class LVMVolumeDriver(VolumeDriver):\n    supports_multiattach = True\n",
                &["VolumeDriver"],
            )],
            ..ParsedFile::default()
        },
        ParsedFile {
            path: "cinder/volume/manager.py".into(),
            language: "python".into(),
            functions: vec![
                function(
                    "attach_volume",
                    "cinder/volume/manager.py",
                    "def attach_volume(context, volume_id):\n    return initialize_connection(volume_id)\n",
                    &["initialize_connection"],
                ),
                function(
                    "initialize_connection",
                    "cinder/volume/manager.py",
                    "def initialize_connection(volume_id):\n    return {}\n",
                    &[],
                ),
            ],
            ..ParsedFile::default()
        },
    ];
    writer.write_version("cinder", "v1", &files).await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_code_falls_back_while_cold_then_answers_from_the_index_and_records_traces() {
    let test_db = TestDb::new().await;
    seed(&test_db.db).await;
    let db = Arc::new(test_db.db.clone());
    let config = SearchConfig {
        engine: SearchMode::Tiered,
        ..SearchConfig::default()
    };
    let service = wiring::build(
        &config,
        Arc::clone(&db),
        Some(CountingLlm::failing() as Arc<dyn LlmProvider>),
        Recorder::new(Arc::clone(&db), "node-a"),
    )
    .unwrap();
    let tool = SearchCodeTool::new(Arc::clone(&service), None);
    let params = json!({
        "query": "What does attach_volume call?",
        "repo": "cinder",
        "version": "v1",
    });

    let cold = tool.execute(params.clone()).await.unwrap();
    assert!(cold.contains("fallback"), "{cold}");
    assert!(cold.contains("still loading"), "{cold}");

    let scope = Scope::new("cinder", "v1");
    for _ in 0..200 {
        if service.registry.get(&scope).is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let warm = tool.execute(params).await.unwrap();
    assert!(warm.contains("tier1"), "{warm}");
    assert!(
        warm.contains("[cinder:v1:cinder/volume/manager.py:1-2]"),
        "{warm}"
    );
    assert!(warm.contains("initialize_connection"), "{warm}");

    let mut stored = Vec::new();
    for _ in 0..100 {
        stored = db
            .query(
                "SELECT engine, tier_reached, degraded, repo FROM search_traces ORDER BY id",
                json!({}),
            )
            .await
            .unwrap();
        if stored.len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert_eq!(stored[0]["degraded"], "index_cold");
    assert_eq!(stored[1]["tier_reached"], "tier1");
    assert_eq!(stored[1]["repo"], "cinder");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_code_rejects_missing_scope() {
    let test_db = TestDb::new().await;
    let db = Arc::new(test_db.db.clone());
    let service = wiring::build(
        &SearchConfig::default(),
        Arc::clone(&db),
        None,
        Recorder::metrics_only(),
    )
    .unwrap();
    let tool = SearchCodeTool::new(service, None);
    assert!(tool.execute(json!({"query": "x"})).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_code_calls_the_model_of_the_asking_user_not_the_server() {
    let test_db = TestDb::new().await;
    seed(&test_db.db).await;
    let db = Arc::new(test_db.db.clone());
    let dir = tempfile::tempdir().unwrap();
    let gate = dir.path().join("gate.json");
    std::fs::write(
        &gate,
        json!({"weights": [], "bias": 0.0, "accept": 2.0, "reject": 1.5}).to_string(),
    )
    .unwrap();
    let config = SearchConfig {
        engine: SearchMode::Tiered,
        gate: Some(gate),
        ..SearchConfig::default()
    };
    let server = CountingLlm::failing();
    let service = wiring::build(
        &config,
        Arc::clone(&db),
        Some(Arc::clone(&server) as Arc<dyn LlmProvider>),
        Recorder::metrics_only(),
    )
    .unwrap();
    warm(&service).await;
    let user = CountingLlm::answering(r#"{"sufficient": false, "steps": [], "done": true}"#);
    let tool = SearchCodeTool::new(
        Arc::clone(&service),
        Some(Arc::clone(&user) as Arc<dyn LlmProvider>),
    );
    let answer = tool
        .execute(json!({
            "query": "Which drivers support multiattach and what enforces it?",
            "repo": "cinder",
            "version": "v1",
        }))
        .await
        .unwrap();
    assert!(
        user.calls() > 0,
        "the user's model was never called: {answer}"
    );
    assert_eq!(server.calls(), 0, "the server's model was called: {answer}");
}
