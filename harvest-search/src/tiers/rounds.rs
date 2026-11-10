use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::{SearchError, TierId};
use crate::index::vectors::Embedder;
use crate::retrieve::expand::expand;
use crate::retrieve::extract::extract;
use crate::retrieve::fuse::fuse;
use crate::retrieve::rerank::Reranker;
use crate::retrieve::{merge_pinned, Candidate, RetrievalInput, Retriever};
use crate::tiers::{rerank_and_pack, Tier};

pub struct Rounds {
    retrievers: Vec<Box<dyn Retriever>>,
    reranker: Arc<dyn Reranker>,
    embedder: Arc<dyn Embedder>,
    max_rounds: usize,
    k: usize,
    pool: usize,
}

impl Rounds {
    pub fn new(
        retrievers: Vec<Box<dyn Retriever>>,
        reranker: Arc<dyn Reranker>,
        embedder: Arc<dyn Embedder>,
    ) -> Self {
        Self {
            retrievers,
            reranker,
            embedder,
            max_rounds: 3,
            k: 50,
            pool: 200,
        }
    }

    pub fn with_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds.max(1);
        self
    }

    async fn query_vector(&self, ctx: &SearchContext) -> Option<Vec<f32>> {
        if ctx.index.vectors.is_empty() || ctx.index.vectors.model() != self.embedder.model() {
            return None;
        }
        match self.embedder.embed(&[ctx.terms.embedding_text()]).await {
            Ok(mut vectors) => vectors.pop(),
            Err(e) => {
                tracing::warn!(error = %e, "query embedding failed; continuing without dense retrieval");
                None
            }
        }
    }
}

const FAMILY_SCAN: usize = 30;
const MIN_FAMILY: usize = 3;

pub fn enumerate_family(ctx: &mut SearchContext) {
    use crate::index::corpus::UnitKind;
    use crate::question::QuestionKind;
    if ctx.question.kind != QuestionKind::Hierarchy || !ctx.question.wants_list {
        return;
    }
    if ctx
        .fixed
        .iter()
        .any(|e| e.label.contains(" subclasses of "))
    {
        return;
    }
    let index = std::sync::Arc::clone(&ctx.index);
    let mut roots: Vec<u32> = ctx
        .named
        .iter()
        .copied()
        .filter(|u| index.unit(*u).kind() == UnitKind::Class)
        .collect();
    if roots.is_empty() {
        for candidate in ctx.candidates.iter().take(FAMILY_SCAN) {
            let unit = index.unit(candidate.unit);
            if unit.kind() != UnitKind::Class || unit.is_test() || unit.is_vendored() {
                continue;
            }
            for root in std::iter::once(candidate.unit).chain(index.graph.ancestors(candidate.unit))
            {
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
    }
    let best = roots
        .into_iter()
        .map(|root| {
            let family = index
                .graph
                .subclasses(root, crate::index::graph::Depth::Transitive);
            let related = family
                .iter()
                .filter(|u| {
                    let unit = index.unit(**u);
                    !unit.is_test() && ctx.question.relatedness(&unit.qualified_name) >= 1
                })
                .count();
            (root, related, family.len())
        })
        .filter(|(root, related, size)| {
            *related >= MIN_FAMILY || (*size >= MIN_FAMILY && ctx.named.contains(root))
        })
        .max_by_key(|(root, related, size)| (*related, *size, std::cmp::Reverse(*root)));
    let Some((root, _, _)) = best else {
        return;
    };
    let (_, table) =
        crate::tiers::plan::execute::subclass_table(&index, root, crate::domain::Source::Graph);
    ctx.fixed.insert(0, table);
}

fn top_units(candidates: &[Candidate], k: usize) -> Vec<u32> {
    candidates.iter().take(k).map(|c| c.unit).collect()
}

#[async_trait]
impl Tier for Rounds {
    fn id(&self) -> TierId {
        TierId::Rounds
    }

    fn expected_cost(&self) -> Duration {
        Duration::from_millis(500)
    }

    async fn run(&self, ctx: &mut SearchContext) -> Result<(), SearchError> {
        self.gather(ctx).await;
        enumerate_family(ctx);
        rerank_and_pack(ctx, self.reranker.as_ref()).await
    }
}

impl Rounds {
    pub async fn gather(&self, ctx: &mut SearchContext) {
        let weights = ctx.question.edge_weights();
        let mut pinned: Vec<Candidate> = ctx
            .candidates
            .iter()
            .filter(|c| c.pinned)
            .cloned()
            .collect();
        for unit in &ctx.named {
            if !pinned.iter().any(|c| c.unit == *unit) {
                pinned.push(Candidate {
                    unit: *unit,
                    score: 1.0,
                    exact: true,
                    pinned: true,
                    sources: vec![crate::domain::Source::Exact],
                    ..Candidate::default()
                });
            }
        }
        let mut previous: Vec<u32> = Vec::new();
        for round in 0..self.max_rounds {
            let started = Instant::now();
            let vector = self.query_vector(ctx).await;
            let input = RetrievalInput {
                terms: &ctx.terms,
                vector: vector.as_deref(),
            };
            let lists = self
                .retrievers
                .iter()
                .map(|r| r.retrieve(&ctx.index, &input, self.k))
                .collect();
            let seeds = fuse(lists);
            ctx.trace.record("tier1.seed", started.elapsed());

            let started = Instant::now();
            let mut expanded = expand(&ctx.index, seeds, &weights, self.pool);
            merge_pinned(&mut expanded, pinned.clone());
            ctx.trace.record("tier1.expand", started.elapsed());
            ctx.trace.rounds = round as u32 + 1;

            let current = top_units(&expanded, 10);
            ctx.stable = !previous.is_empty() && current == previous;
            ctx.candidates = expanded;
            if ctx.stable {
                break;
            }
            previous = current;
            let started = Instant::now();
            let names = extract(&ctx.index, &ctx.candidates, 5);
            let added = ctx.terms.extend(&names);
            ctx.trace.record("tier1.extract", started.elapsed());
            if added == 0 {
                ctx.stable = true;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Query, Scope};
    use crate::index::corpus::{Corpus, Relation, Unit, UnitKind};
    use crate::index::vectors::HashingEmbedder;
    use crate::index::IndexSnapshot;
    use crate::Deadline;

    fn class(name: &str, file: &str) -> Unit {
        Unit {
            name: name.into(),
            qualified_name: name.into(),
            kind: Some(UnitKind::Class),
            file: file.into(),
            start_line: 1,
            end_line: 10,
            ..Unit::default()
        }
    }

    #[tokio::test]
    async fn list_questions_about_a_family_get_the_whole_family() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        let base = corpus.push_unit(class("BaseVD", "volume/driver.py"));
        let mut drivers = Vec::new();
        for i in 0..20 {
            let d = corpus.push_unit(class(
                &format!("Vendor{i}Driver"),
                &format!("volume/drivers/v{i}.py"),
            ));
            corpus.link(d, base, Relation::Inherits);
            drivers.push(d);
        }
        let mut fake = class("FakeDriver", "tests/fake.py");
        fake.test = true;
        let fake = corpus.push_unit(fake);
        corpus.link(fake, base, Relation::Inherits);
        let index = Arc::new(
            IndexSnapshot::build(corpus, &HashingEmbedder::default())
                .await
                .unwrap(),
        );
        let query = Query::new(
            "What are the volume drivers available?",
            Scope::new("r", "v"),
        );
        let mut ctx = SearchContext::new(&query, index, Deadline::after(Duration::from_secs(5)));
        ctx.candidates = drivers[..3]
            .iter()
            .map(|u| Candidate {
                unit: *u,
                score: 1.0,
                ..Candidate::default()
            })
            .collect();
        enumerate_family(&mut ctx);
        assert_eq!(ctx.fixed.len(), 1);
        let table = &ctx.fixed[0];
        assert!(
            table.label.contains("all 20 subclasses of BaseVD"),
            "{}",
            table.label
        );
        assert_eq!(table.covers.len(), 20);
        assert!(!table.text.contains("FakeDriver"));
    }
}
