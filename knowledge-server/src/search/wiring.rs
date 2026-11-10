use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use harvest_db::Db;
use harvest_search::engine::{TieredEngine, WithFallback};
use harvest_search::fallback::PostgresEngine;
use harvest_search::gate::completeness::ListCompleteness;
use harvest_search::gate::dangling::DanglingReferences;
use harvest_search::gate::judge::ModelJudge;
use harvest_search::gate::score::CalibratedScore;
use harvest_search::gate::{Gate, SufficiencyCheck};
use harvest_search::index::corpus::CorpusSource;
use harvest_search::index::disk_cache::DiskCache;
use harvest_search::index::pg::PgCorpus;
use harvest_search::index::vectors::{Embedder, HashingEmbedder};
use harvest_search::index::IndexRegistry;
use harvest_search::llm::LanguageModel;
use harvest_search::retrieve::rerank::{LinearReranker, Reranker};
use harvest_search::retrieve::{Bm25Retriever, CacheRetriever, DenseRetriever, ExactRetriever};
use harvest_search::tiers::plan::Plan;
use harvest_search::tiers::rounds::Rounds;
use harvest_search::tiers::steps::Steps;
use harvest_search::tiers::Tier;
use harvest_search::SearchEngine;
use serde::de::DeserializeOwned;

use crate::config::SearchConfig;
use crate::llm::types::ProviderSelection;
use crate::llm::LlmProvider;
use crate::search::llm_adapter::ProviderModel;
use crate::search::record::Recorder;
use crate::search::SearchService;

fn load_json<T: DeserializeOwned + Default>(path: Option<&Path>, what: &str) -> Result<T> {
    let Some(path) = path else {
        return Ok(T::default());
    };
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the {what} file {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("parsing the {what} file {}", path.display()))
}

pub struct StoredEmbedder {
    inner: Arc<dyn crate::agent::semantic::Embedder>,
}

impl StoredEmbedder {
    pub fn new(inner: Arc<dyn crate::agent::semantic::Embedder>) -> Arc<Self> {
        Arc::new(Self { inner })
    }
}

#[async_trait::async_trait]
impl Embedder for StoredEmbedder {
    fn model(&self) -> &str {
        self.inner.model()
    }

    async fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inner.embed(texts).await
    }
}

pub struct Parts {
    pub registry: Arc<IndexRegistry>,
    pub reranker: Arc<dyn Reranker>,
    pub gate: CalibratedScore,
    pub model: Option<Arc<dyn LanguageModel>>,
}

pub fn parts(
    config: &SearchConfig,
    source: Arc<dyn CorpusSource>,
    model: Option<Arc<dyn LanguageModel>>,
) -> Result<Parts> {
    parts_with_embedder(config, source, model, Arc::new(HashingEmbedder::default()))
}

pub fn parts_with_embedder(
    config: &SearchConfig,
    source: Arc<dyn CorpusSource>,
    model: Option<Arc<dyn LanguageModel>>,
    embedder: Arc<dyn Embedder>,
) -> Result<Parts> {
    let cache = config.cache_dir.as_ref().map(DiskCache::new);
    let reranker: LinearReranker = load_json(config.reranker.as_deref(), "reranker")?;
    Ok(Parts {
        registry: IndexRegistry::new(source, embedder, cache),
        reranker: Arc::new(reranker),
        gate: load_json(config.gate.as_deref(), "gate calibration")?,
        model,
    })
}

pub fn rounds(parts: &Parts) -> Rounds {
    Rounds::new(
        vec![
            Box::new(Bm25Retriever),
            Box::new(DenseRetriever),
            Box::new(ExactRetriever),
            Box::new(CacheRetriever::default()),
        ],
        Arc::clone(&parts.reranker),
        Arc::clone(parts.registry.embedder()),
    )
}

pub fn tiered(config: &SearchConfig, parts: &Parts, tier_one_only: bool) -> TieredEngine {
    let mut tiers: Vec<Box<dyn Tier>> = vec![Box::new(rounds(parts))];
    let mut checks: Vec<Box<dyn SufficiencyCheck>> = vec![
        Box::new(ListCompleteness::default()),
        Box::new(DanglingReferences),
        Box::new(parts.gate.clone()),
    ];
    if let (Some(model), false) = (&parts.model, tier_one_only) {
        tiers.push(Box::new(Plan::new(
            Arc::clone(model),
            Arc::clone(&parts.reranker),
        )));
        tiers.push(Box::new(
            Steps::new(Arc::clone(model), Arc::clone(&parts.reranker))
                .with_turns(config.tier3_turns),
        ));
        if config.judge {
            checks.push(Box::new(ModelJudge::new(Arc::clone(model))));
        }
    }
    TieredEngine::new(Arc::clone(&parts.registry), tiers, Gate::new(checks))
}

pub struct ModelEngines {
    registry: Arc<IndexRegistry>,
    reranker: Arc<dyn Reranker>,
    gate: CalibratedScore,
    db: Arc<Db>,
    selection: Option<ProviderSelection>,
}

impl ModelEngines {
    pub fn engine(
        &self,
        config: &SearchConfig,
        llm: &Arc<dyn LlmProvider>,
    ) -> Arc<dyn SearchEngine> {
        let parts = Parts {
            registry: Arc::clone(&self.registry),
            reranker: Arc::clone(&self.reranker),
            gate: self.gate.clone(),
            model: Some(ProviderModel::new(Arc::clone(llm), self.selection.clone())),
        };
        with_fallback(config, &parts, Arc::clone(&self.db))
    }
}

fn with_fallback(config: &SearchConfig, parts: &Parts, db: Arc<Db>) -> Arc<dyn SearchEngine> {
    Arc::new(WithFallback::new(
        tiered(config, parts, false),
        PostgresEngine::new(db),
    ))
}

pub fn fast_model_selection(config: &SearchConfig) -> Option<ProviderSelection> {
    config.fast_model.as_ref().map(|m| ProviderSelection {
        provider_id: m.provider_id.clone(),
        model: m.model.clone(),
        cache_breakpoint_index: None,
    })
}

pub fn build(
    config: &SearchConfig,
    db: Arc<Db>,
    llm: Option<Arc<dyn LlmProvider>>,
    recorder: Recorder,
) -> Result<Arc<SearchService>> {
    build_with_embeddings(config, db, llm, recorder, None)
}

pub fn build_with_embeddings(
    config: &SearchConfig,
    db: Arc<Db>,
    llm: Option<Arc<dyn LlmProvider>>,
    recorder: Recorder,
    stored: Option<Arc<dyn crate::agent::semantic::Embedder>>,
) -> Result<Arc<SearchService>> {
    let selection = fast_model_selection(config);
    let model: Option<Arc<dyn LanguageModel>> = llm.as_ref().map(|llm| {
        ProviderModel::new(Arc::clone(llm), selection.clone()) as Arc<dyn LanguageModel>
    });
    let parts = match stored {
        Some(embedder) => parts_with_embedder(
            config,
            PgCorpus::with_stored_embeddings(Arc::clone(&db), embedder.model()),
            model,
            StoredEmbedder::new(embedder),
        )?,
        None => parts(config, PgCorpus::new(Arc::clone(&db)), model)?,
    };
    let engine = with_fallback(config, &parts, Arc::clone(&db));
    let models = ModelEngines {
        registry: Arc::clone(&parts.registry),
        reranker: Arc::clone(&parts.reranker),
        gate: parts.gate.clone(),
        db,
        selection,
    };
    Ok(Arc::new(SearchService {
        engine,
        registry: parts.registry,
        recorder,
        config: config.clone(),
        models,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_calibration_files_fall_back_to_defaults_and_bad_ones_fail() {
        let gate: CalibratedScore = load_json(None, "gate").unwrap();
        assert_eq!(gate, CalibratedScore::default());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gate.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(load_json::<CalibratedScore>(Some(&path), "gate").is_err());
        let custom = CalibratedScore {
            accept: 0.9,
            ..CalibratedScore::default()
        };
        std::fs::write(&path, serde_json::to_string(&custom).unwrap()).unwrap();
        assert_eq!(
            load_json::<CalibratedScore>(Some(&path), "gate").unwrap(),
            custom
        );
    }
}
