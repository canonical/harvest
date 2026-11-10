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
