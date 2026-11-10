use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use harvest_search::engine::{SearchEngine, TieredEngine, WithFallback};
use harvest_search::gate::completeness::ListCompleteness;
use harvest_search::gate::dangling::DanglingReferences;
use harvest_search::gate::score::CalibratedScore;
use harvest_search::gate::Gate;
use harvest_search::index::corpus::{Corpus, CorpusSource, Relation, Unit, UnitKind};
use harvest_search::index::vectors::HashingEmbedder;
use harvest_search::index::IndexRegistry;
use harvest_search::llm::{LanguageModel, ScriptedModel};
use harvest_search::retrieve::rerank::LinearReranker;
use harvest_search::retrieve::{Bm25Retriever, DenseRetriever, ExactRetriever};
use harvest_search::tiers::plan::Plan;
use harvest_search::tiers::rounds::Rounds;
use harvest_search::tiers::steps::Steps;
use harvest_search::tiers::Tier;
use harvest_search::{
    Deadline, Degraded, EvidenceKey, Query, Scope, SearchError, SearchResult, Source, TierId,
};

struct Fixed(Corpus);

#[async_trait]
impl CorpusSource for Fixed {
    async fn generation(&self, _scope: &Scope) -> anyhow::Result<Option<String>> {
        Ok(Some(self.0.generation.clone()))
    }

    async fn load(&self, _scope: &Scope) -> anyhow::Result<Corpus> {
        Ok(self.0.clone())
    }
}

fn scope() -> Scope {
    Scope::new("cinder", "v1")
}

fn class(name: &str, file: &str, body: &str, bases: &[&str]) -> Unit {
    Unit {
        name: name.into(),
        kind: Some(UnitKind::Class),
        file: file.into(),
        start_line: 1,
        end_line: body.lines().count() as u32,
        body: body.into(),
        bases: bases.iter().map(|b| b.to_string()).collect(),
        language: "python".into(),
        ..Unit::default()
    }
}

fn method(class: &str, name: &str, file: &str, lines: (u32, u32), body: &str) -> Unit {
    Unit {
        name: name.into(),
        qualified_name: format!("{class}.{name}"),
        parent: Some(class.into()),
        kind: Some(UnitKind::Method),
        file: file.into(),
        start_line: lines.0,
        end_line: lines.1,
        body: body.into(),
        language: "python".into(),
        ..Unit::default()
    }
}

const NAMED: [&str; 6] = [
    "LVMVolumeDriver",
    "NfsDriver",
    "RBDDriver",
    "PureISCSIDriver",
    "NetAppDriver",
    "GlusterDriver",
];

fn drivers() -> Vec<String> {
    NAMED
        .iter()
        .map(|d| d.to_string())
        .chain((0..34).map(|i| format!("Vendor{i}Driver")))
        .collect()
}

fn corpus() -> Corpus {
    let mut corpus = Corpus::new(scope(), "g1");
    let base = corpus.push_unit(class(
        "VolumeDriver",
        "cinder/volume/driver.py",
        "class VolumeDriver(object):\n    \"\"\"Base class for volume drivers.\"\"\"\n    supports_multiattach = False\n\n    def initialize_connection(self, volume, connector):\n        raise NotImplementedError()\n",
        &["object"],
    ));
    let init = corpus.push_unit(method(
        "VolumeDriver",
        "initialize_connection",
        "cinder/volume/driver.py",
        (5, 6),
        "def initialize_connection(self, volume, connector):\n    raise NotImplementedError()\n",
    ));
    corpus.link(base, init, Relation::Contains);
    for (i, driver) in drivers().iter().enumerate() {
        let file = format!("cinder/volume/drivers/{}.py", driver.to_lowercase());
        let value = if i % 2 == 0 { "True" } else { "False" };
        let id = corpus.push_unit(class(
            driver,
            &file,
            &format!(
                "class {driver}(VolumeDriver):\n    supports_multiattach = {value}\n    vendor_name = '{driver}'\n{}",
                "    def _vendor_call(self, payload):\n        return self._client.post('/api', payload, timeout=30, retries=3)\n".repeat(12)
            ),
            &["VolumeDriver"],
        ));
        corpus.link(id, base, Relation::Inherits);
    }
    let manager = corpus.push_unit(class(
        "VolumeManager",
        "cinder/volume/manager.py",
        "class VolumeManager(object):\n    def attach_volume(self, context, volume_id):\n        conf = CONF.retry_limit\n        return self.driver.initialize_connection(volume, connector)\n",
        &["object"],
    ));
    let attach = corpus.push_unit(method(
        "VolumeManager",
        "attach_volume",
        "cinder/volume/manager.py",
        (2, 4),
        "def attach_volume(self, context, volume_id):\n    conf = CONF.retry_limit\n    return self.driver.initialize_connection(volume, connector)\n",
    ));
    corpus.link(manager, attach, Relation::Contains);
    corpus.link(attach, init, Relation::Calls);
    for i in 0..30 {
        corpus.push_unit(Unit {
            name: format!("helper_{i}"),
            kind: Some(UnitKind::Function),
            file: format!("cinder/utils/h{i}.py"),
            start_line: 1,
            end_line: 3,
            body: format!("def helper_{i}(x):\n    return x + {i}\n"),
            ..Unit::default()
        });
    }
    corpus
}

async fn registry() -> Arc<IndexRegistry> {
    let registry = IndexRegistry::new(
        Arc::new(Fixed(corpus())),
        Arc::new(HashingEmbedder::default()),
        None,
    );
    registry.ensure(&scope()).await.unwrap();
    registry
}

fn gate() -> Gate {
    Gate::new(vec![
        Box::new(ListCompleteness::default()),
        Box::new(DanglingReferences),
        Box::new(CalibratedScore {
            accept: 0.5,
            ..CalibratedScore::default()
        }),
    ])
}

fn engine(registry: Arc<IndexRegistry>, model: Arc<dyn LanguageModel>) -> TieredEngine {
    let reranker = Arc::new(LinearReranker::default());
    let tiers: Vec<Box<dyn Tier>> = vec![
        Box::new(Rounds::new(
            vec![
                Box::new(Bm25Retriever),
                Box::new(DenseRetriever),
                Box::new(ExactRetriever),
            ],
            reranker.clone(),
            registry.embedder().clone(),
        )),
        Box::new(Plan::new(model.clone(), reranker.clone())),
        Box::new(Steps::new(model, reranker).with_turns(2)),
    ];
    TieredEngine::new(registry, tiers, gate())
}

fn list_query() -> Query {
    let mut query = Query::new("Which volume drivers support multiattach?", scope());
    query.budget = harvest_search::TokenBudget(3_000);
    query
}

fn deadline() -> Deadline {
    Deadline::after(Duration::from_secs(10))
}

fn has_symbol(result: &SearchResult, name: &str) -> bool {
    result
        .evidence
        .iter()
        .flat_map(|e| e.keys())
        .any(|k| matches!(k, EvidenceKey::Symbol { name: n, .. } if n == name))
}

#[tokio::test]
async fn a_direct_lookup_is_answered_in_tier_one_without_the_model() {
    let model = Arc::new(ScriptedModel::failing());
    let engine = engine(registry().await, model.clone());
    let result = engine
        .search(
            &Query::new("What does VolumeManager.attach_volume do?", scope()),
            deadline(),
        )
        .await
        .unwrap();
    assert_eq!(
        result.tier_reached,
        TierId::Rounds,
        "{:?} {:?} {:#?}",
        result.trace.gate,
        result.gaps,
        result.evidence.iter().map(|e| &e.label).collect::<Vec<_>>()
    );
    assert_eq!(result.trace.llm_calls, 0);
    assert!(model.prompts().is_empty());
    assert!(has_symbol(&result, "VolumeManager.attach_volume"));
    assert!(!result.evidence.iter().any(|e| e.label.contains("helper_")));
    assert!(
        has_symbol(&result, "VolumeDriver.initialize_connection")
            || has_symbol(&result, "VolumeDriver"),
        "the callee or its class is packed"
    );
    assert!(result.trace.stage_ms("tier1.seed") > 0.0);
    assert!(result.degraded.is_none());
}

#[tokio::test]
async fn an_incomplete_list_escalates_to_a_plan_that_covers_every_driver() {
    let model = Arc::new(ScriptedModel::new(vec![json!({"steps": [
        {"op": "subclasses_of", "class": "VolumeDriver"},
        {"op": "capability", "of": 0, "name": "supports_multiattach"}
    ]})]));
    let engine = engine(registry().await, model.clone());
    let result = engine.search(&list_query(), deadline()).await.unwrap();
    assert_eq!(result.tier_reached, TierId::Plan, "{:?}", result.trace.gate);
    assert_eq!(result.trace.llm_calls, 1);
    assert!(
        model.prompts()[0].contains("only"),
        "the plan prompt names the gap: {}",
        model.prompts()[0]
    );
    for driver in drivers() {
        assert!(has_symbol(&result, &driver), "{driver} missing");
    }
    let matrix = result
        .evidence
        .iter()
        .find(|e| e.label == "capability supports_multiattach")
        .expect("the capability matrix is packed");
    assert!(matrix.text.contains("LVMVolumeDriver"));
    assert_eq!(matrix.found_by, Source::Plan);
    assert!(result.gaps.is_empty());
}

#[tokio::test]
async fn a_failing_model_degrades_to_tier_one_evidence() {
    let engine = engine(registry().await, Arc::new(ScriptedModel::failing()));
    let result = engine.search(&list_query(), deadline()).await.unwrap();
    assert_eq!(result.degraded, Some(Degraded::ModelUnavailable));
    assert_eq!(result.tier_reached, TierId::Plan);
    assert!(!result.evidence.is_empty());
    assert!(!result.gaps.is_empty());
}

#[tokio::test]
async fn an_exhausted_deadline_stops_escalation() {
    let model = Arc::new(ScriptedModel::new(vec![json!({"steps": []})]));
    let engine = engine(registry().await, model.clone());
    let result = engine
        .search(&list_query(), Deadline::after(Duration::from_millis(1)))
        .await
        .unwrap();
    assert_eq!(result.tier_reached, TierId::Rounds);
    assert_eq!(result.degraded, Some(Degraded::DeadlineTruncated));
    assert!(model.prompts().is_empty());
}

#[tokio::test]
async fn tier_three_takes_steps_until_done() {
    let model = Arc::new(ScriptedModel::new(vec![
        json!({"steps": [{"op": "search", "terms": "nothing useful"}]}),
        json!({"steps": [{"op": "subclasses_of", "class": "VolumeDriver"}], "done": false}),
        json!({"steps": [{"op": "capability", "of": 1, "name": "supports_multiattach"}], "done": true}),
    ]));
    let engine = engine(registry().await, model.clone());
    let result = engine.search(&list_query(), deadline()).await.unwrap();
    assert_eq!(
        result.tier_reached,
        TierId::Steps,
        "{:?}",
        result.trace.gate
    );
    assert_eq!(result.trace.llm_calls, 3);
    assert!(
        model.prompts()[2].contains("[0]"),
        "tier 3 sees its own history"
    );
    for driver in drivers() {
        assert!(has_symbol(&result, &driver), "{driver} missing");
    }
}

struct Canned;

#[async_trait]
impl SearchEngine for Canned {
    async fn search(
        &self,
        _query: &Query,
        _deadline: Deadline,
    ) -> Result<SearchResult, SearchError> {
        Ok(SearchResult {
            evidence: Vec::new(),
            gaps: Vec::new(),
            tier_reached: TierId::Fallback,
            degraded: None,
            generation: None,
            trace: Default::default(),
        })
    }
}

#[tokio::test]
async fn a_cold_index_falls_back_and_says_so() {
    let registry = IndexRegistry::new(
        Arc::new(Fixed(corpus())),
        Arc::new(HashingEmbedder::default()),
        None,
    );
    let engine = WithFallback::new(
        engine(registry.clone(), Arc::new(ScriptedModel::failing())),
        Canned,
    );
    let result = engine
        .search(&Query::new("VolumeDriver", scope()), deadline())
        .await
        .unwrap();
    assert_eq!(result.tier_reached, TierId::Fallback);
    assert_eq!(result.degraded, Some(Degraded::IndexCold));
    for _ in 0..200 {
        if registry.get(&scope()).is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the cold index was never warmed");
}
