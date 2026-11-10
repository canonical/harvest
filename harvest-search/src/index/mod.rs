pub mod bm25;
pub mod corpus;
pub mod disk_cache;
pub mod graph;
pub mod pg;
pub mod tokenizer;
pub mod vectors;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;

use crate::domain::{Scope, SearchError, UnitId};
use bm25::Bm25Index;
use corpus::{Corpus, CorpusSource, Edge, Unit};
use disk_cache::DiskCache;
use graph::CodeGraph;
use vectors::{embedding_text, Embedder, VectorIndex};

const EMBED_BATCH: usize = 256;

pub struct CacheIndex {
    pub bm25: Bm25Index,
    pub questions: Vec<String>,
    pub units: Vec<Vec<UnitId>>,
}

impl CacheIndex {
    fn build(cached: &[corpus::CachedEvidence]) -> Option<Self> {
        if cached.is_empty() {
            return None;
        }
        let pseudo: Vec<Unit> = cached
            .iter()
            .enumerate()
            .map(|(i, c)| Unit {
                id: i as UnitId,
                docstring: c.question.clone(),
                ..Unit::default()
            })
            .collect();
        Some(Self {
            bm25: Bm25Index::build(&pseudo),
            questions: cached.iter().map(|c| c.question.clone()).collect(),
            units: cached.iter().map(|c| c.units.clone()).collect(),
        })
    }
}

pub const MAX_NAMED_MATCHES: usize = 5;
const MAX_NAMED_WORD_MATCHES: usize = 2;
const MIN_NAMED_WORD: usize = 4;

pub struct IndexSnapshot {
    pub scope: Scope,
    pub generation: String,
    pub calibration: Option<crate::calibration::Calibration>,
    pub units: Vec<Unit>,
    pub edges: Vec<Edge>,
    pub graph: CodeGraph,
    pub bm25: Bm25Index,
    pub vectors: VectorIndex,
    pub cache: Option<CacheIndex>,
    names: HashMap<String, Vec<UnitId>>,
}

impl IndexSnapshot {
    pub async fn build(corpus: Corpus, embedder: &dyn Embedder) -> Result<Self> {
        let vectors = match corpus.vectors {
            Some(stored)
                if stored.model == embedder.model()
                    && stored.vectors.len() == corpus.units.len() =>
            {
                VectorIndex::from_vectors(stored.model, stored.vectors)
            }
            _ => {
                let mut all = Vec::with_capacity(corpus.units.len());
                for batch in corpus.units.chunks(EMBED_BATCH) {
                    let texts: Vec<String> = batch.iter().map(embedding_text).collect();
                    all.extend(embedder.embed(&texts).await?.into_iter().map(Some));
                }
                VectorIndex::from_vectors(embedder.model(), all)
            }
        };
        let mut names: HashMap<String, Vec<UnitId>> = HashMap::new();
        for unit in &corpus.units {
            let mut keys = vec![
                tokenizer::name_key(&unit.name),
                tokenizer::name_key(&unit.qualified_name),
            ];
            if let Some(last) = tokenizer::name_parts(&unit.name).pop() {
                keys.push(last);
            }
            keys.retain(|k| !k.is_empty());
            keys.sort();
            keys.dedup();
            for key in keys {
                let ids = names.entry(key).or_default();
                if !ids.contains(&unit.id) {
                    ids.push(unit.id);
                }
            }
        }
        Ok(Self {
            cache: CacheIndex::build(&corpus.cached),
            graph: CodeGraph::build(corpus.units.len(), &corpus.edges),
            bm25: Bm25Index::build(&corpus.units),
            scope: corpus.scope,
            generation: corpus.generation,
            calibration: corpus.calibration,
            units: corpus.units,
            edges: corpus.edges,
            vectors,
            names,
        })
    }

    pub fn unit(&self, id: UnitId) -> &Unit {
        &self.units[id as usize]
    }

    pub fn find_by_name(&self, name: &str) -> &[UnitId] {
        let parts = tokenizer::name_parts(name);
        for start in 0..parts.len() {
            if let Some(found) = self.names.get(&parts[start..].join(".")) {
                return found;
            }
        }
        &[]
    }

    pub fn named_in(&self, question: &crate::question::Question) -> Vec<UnitId> {
        let mut named = self.named_units(&question.identifiers);
        let covered: HashSet<String> = question
            .identifiers
            .iter()
            .flat_map(|i| {
                let parts = tokenizer::name_parts(i);
                let pieces: Vec<String> = parts
                    .iter()
                    .flat_map(|p| tokenizer::split_identifier(p))
                    .collect();
                parts.into_iter().chain(pieces)
            })
            .collect();
        for word in &question.words {
            if word.chars().count() < MIN_NAMED_WORD
                || tokenizer::is_stopword(word)
                || covered.contains(word)
            {
                continue;
            }
            let Some(found) = self.names.get(word) else {
                continue;
            };
            let exact: Vec<UnitId> = found
                .iter()
                .copied()
                .filter(|u| {
                    let unit = self.unit(*u);
                    tokenizer::name_key(&unit.name) == *word
                        && unit.kind().is_symbol()
                        && !unit.is_vendored()
                        && !unit.is_generated()
                        && !unit.is_test()
                })
                .collect();
            if (1..=MAX_NAMED_WORD_MATCHES).contains(&exact.len()) {
                for unit in exact {
                    if !named.contains(&unit) {
                        named.push(unit);
                    }
                }
            }
        }
        named
    }

    pub fn named_units(&self, identifiers: &[String]) -> Vec<UnitId> {
        let mut named = Vec::new();
        for identifier in identifiers {
            let candidates: Vec<UnitId> = self
                .find_by_name(identifier)
                .iter()
                .copied()
                .filter(|u| {
                    let unit = self.unit(*u);
                    !unit.is_vendored() && !unit.is_generated()
                })
                .collect();
            let preferred: Vec<UnitId> = {
                let source: Vec<UnitId> = candidates
                    .iter()
                    .copied()
                    .filter(|u| !self.unit(*u).is_test())
                    .collect();
                if source.is_empty() {
                    candidates
                } else {
                    source
                }
            };
            if (1..=MAX_NAMED_MATCHES).contains(&preferred.len()) {
                for unit in preferred {
                    if !named.contains(&unit) {
                        named.push(unit);
                    }
                }
            }
        }
        named
    }
}

pub struct IndexRegistry {
    source: Arc<dyn CorpusSource>,
    embedder: Arc<dyn Embedder>,
    cache: Option<DiskCache>,
    loaded: RwLock<HashMap<Scope, Arc<IndexSnapshot>>>,
    building: Mutex<HashSet<Scope>>,
}

impl IndexRegistry {
    pub fn new(
        source: Arc<dyn CorpusSource>,
        embedder: Arc<dyn Embedder>,
        cache: Option<DiskCache>,
    ) -> Arc<Self> {
        Arc::new(Self {
            source,
            embedder,
            cache,
            loaded: RwLock::new(HashMap::new()),
            building: Mutex::new(HashSet::new()),
        })
    }

    pub fn embedder(&self) -> &Arc<dyn Embedder> {
        &self.embedder
    }

    pub fn get(&self, scope: &Scope) -> Option<Arc<IndexSnapshot>> {
        self.loaded
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(scope)
            .cloned()
    }

    pub fn snapshot(self: &Arc<Self>, scope: &Scope) -> Result<Arc<IndexSnapshot>, SearchError> {
        match self.get(scope) {
            Some(snapshot) => Ok(snapshot),
            None => {
                self.warm(scope.clone());
                Err(SearchError::IndexCold(scope.clone()))
            }
        }
    }

    pub fn warm(self: &Arc<Self>, scope: Scope) {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = registry.ensure(&scope).await {
                tracing::warn!(error = %e, ?scope, "building the search index failed");
            }
        });
    }

    pub fn loaded(&self) -> Vec<(Scope, String)> {
        self.loaded
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(s, snap)| (s.clone(), snap.generation.clone()))
            .collect()
    }

    pub fn invalidate(&self, scope: &Scope) {
        self.loaded
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(scope);
    }

    fn claim(&self, scope: &Scope) -> bool {
        self.building
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(scope.clone())
    }

    fn unclaim(&self, scope: &Scope) {
        self.building
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(scope);
    }

    pub async fn ensure(&self, scope: &Scope) -> Result<Option<Arc<IndexSnapshot>>, SearchError> {
        if let Some(snapshot) = self.get(scope) {
            return Ok(Some(snapshot));
        }
        if !self.claim(scope) {
            return Ok(None);
        }
        let built = self.build(scope).await;
        self.unclaim(scope);
        let snapshot = Arc::new(built?);
        self.loaded
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(scope.clone(), Arc::clone(&snapshot));
        Ok(Some(snapshot))
    }

    pub async fn refresh(&self, scope: &Scope) -> Result<bool, SearchError> {
        let Some(current) = self.get(scope) else {
            return Ok(false);
        };
        let generation = self.source.generation(scope).await?;
        if generation.as_deref() == Some(current.generation.as_str()) {
            return Ok(false);
        }
        if !self.claim(scope) {
            return Ok(false);
        }
        let built = self.build(scope).await;
        self.unclaim(scope);
        let snapshot = Arc::new(built?);
        self.loaded
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(scope.clone(), snapshot);
        Ok(true)
    }

    async fn build(&self, scope: &Scope) -> Result<IndexSnapshot, SearchError> {
        let corpus = self.corpus(scope).await?;
        Ok(IndexSnapshot::build(corpus, self.embedder.as_ref()).await?)
    }

    async fn corpus(&self, scope: &Scope) -> Result<Corpus, SearchError> {
        let generation = match self.source.generation(scope).await {
            Ok(Some(generation)) => generation,
            Ok(None) => return Err(SearchError::UnknownScope(scope.clone())),
            Err(e) => {
                if let Some(cache) = &self.cache {
                    if let Some(corpus) = cache.latest(scope).await {
                        tracing::warn!(error = %e, ?scope, "corpus source unavailable; serving the cached index");
                        return Ok(corpus);
                    }
                }
                return Err(e.into());
            }
        };
        if let Some(cache) = &self.cache {
            if let Some(corpus) = cache.read(scope, &generation).await {
                return Ok(corpus);
            }
        }
        let corpus = self.source.load(scope).await?;
        if let Some(cache) = &self.cache {
            if let Err(e) = cache.write(&corpus).await {
                tracing::warn!(error = %e, ?scope, "could not write the index cache");
            }
        }
        Ok(corpus)
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct MemorySource {
        pub corpora: Mutex<HashMap<Scope, Corpus>>,
        pub loads: AtomicUsize,
        pub fail: std::sync::atomic::AtomicBool,
    }

    impl MemorySource {
        pub fn new(corpora: Vec<Corpus>) -> Arc<Self> {
            Arc::new(Self {
                corpora: Mutex::new(corpora.into_iter().map(|c| (c.scope.clone(), c)).collect()),
                loads: AtomicUsize::new(0),
                fail: std::sync::atomic::AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl CorpusSource for MemorySource {
        async fn generation(&self, scope: &Scope) -> Result<Option<String>> {
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("database down");
            }
            Ok(self
                .corpora
                .lock()
                .unwrap()
                .get(scope)
                .map(|c| c.generation.clone()))
        }

        async fn load(&self, scope: &Scope) -> Result<Corpus> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.corpora
                .lock()
                .unwrap()
                .get(scope)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MemorySource;
    use super::*;
    use std::sync::atomic::Ordering;
    use vectors::HashingEmbedder;

    fn corpus(generation: &str) -> Corpus {
        let mut corpus = Corpus::new(Scope::new("r", "v"), generation);
        corpus.push_unit(Unit {
            name: "VolumeDriver".into(),
            file: "drivers/base.py".into(),
            ..Unit::default()
        });
        corpus
    }

    #[tokio::test]
    async fn a_cold_scope_reports_itself_then_loads_in_the_background() {
        let source = MemorySource::new(vec![corpus("g1")]);
        let registry = IndexRegistry::new(source, Arc::new(HashingEmbedder::default()), None);
        let scope = Scope::new("r", "v");
        assert!(matches!(
            registry.snapshot(&scope),
            Err(SearchError::IndexCold(_))
        ));
        for _ in 0..100 {
            if registry.get(&scope).is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let snapshot = registry.snapshot(&scope).unwrap();
        assert_eq!(snapshot.find_by_name("volumedriver"), &[0]);
        assert_eq!(snapshot.find_by_name("base.VolumeDriver"), &[0]);
        assert_eq!(snapshot.vectors.len(), 1);
    }

    #[tokio::test]
    async fn unknown_scopes_are_an_error() {
        let registry = IndexRegistry::new(
            MemorySource::new(vec![]),
            Arc::new(HashingEmbedder::default()),
            None,
        );
        assert!(matches!(
            registry.ensure(&Scope::new("x", "y")).await,
            Err(SearchError::UnknownScope(_))
        ));
    }

    #[tokio::test]
    async fn refresh_swaps_in_a_new_generation_only_when_it_changed() {
        let source = MemorySource::new(vec![corpus("g1")]);
        let registry =
            IndexRegistry::new(source.clone(), Arc::new(HashingEmbedder::default()), None);
        let scope = Scope::new("r", "v");
        let pinned = registry.ensure(&scope).await.unwrap().unwrap();
        assert!(!registry.refresh(&scope).await.unwrap());
        source
            .corpora
            .lock()
            .unwrap()
            .insert(scope.clone(), corpus("g2"));
        assert!(registry.refresh(&scope).await.unwrap());
        assert_eq!(pinned.generation, "g1");
        assert_eq!(registry.get(&scope).unwrap().generation, "g2");
    }

    #[tokio::test]
    async fn the_disk_cache_avoids_reloading_and_serves_when_the_source_is_down() {
        let dir = tempfile::tempdir().unwrap();
        let scope = Scope::new("r", "v");
        let source = MemorySource::new(vec![corpus("g1")]);
        let first = IndexRegistry::new(
            source.clone(),
            Arc::new(HashingEmbedder::default()),
            Some(DiskCache::new(dir.path())),
        );
        first.ensure(&scope).await.unwrap();
        let second = IndexRegistry::new(
            source.clone(),
            Arc::new(HashingEmbedder::default()),
            Some(DiskCache::new(dir.path())),
        );
        second.ensure(&scope).await.unwrap();
        assert_eq!(source.loads.load(Ordering::SeqCst), 1);
        source.fail.store(true, Ordering::SeqCst);
        let third = IndexRegistry::new(
            source.clone(),
            Arc::new(HashingEmbedder::default()),
            Some(DiskCache::new(dir.path())),
        );
        assert_eq!(
            third.ensure(&scope).await.unwrap().unwrap().generation,
            "g1"
        );
    }
}
