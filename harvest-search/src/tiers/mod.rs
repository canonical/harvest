pub mod plan;
pub mod rounds;
pub mod steps;

use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::{SearchError, TierId};
use crate::retrieve::pack::{pack_shaped, Shape};
use crate::retrieve::rerank::Reranker;

#[async_trait]
pub trait Tier: Send + Sync {
    fn id(&self) -> TierId;
    fn expected_cost(&self) -> Duration;
    async fn run(&self, ctx: &mut SearchContext) -> Result<(), SearchError>;
}

pub async fn rerank_and_pack(
    ctx: &mut SearchContext,
    reranker: &dyn Reranker,
) -> Result<(), SearchError> {
    let started = Instant::now();
    let candidates = std::mem::take(&mut ctx.candidates);
    ctx.candidates = reranker
        .rerank(&ctx.question, &ctx.index, candidates)
        .await?;
    ctx.trace
        .record(format!("{}.rerank", ctx.tier.as_str()), started.elapsed());
    let started = Instant::now();
    ctx.evidence = pack_shaped(
        &ctx.index,
        &ctx.candidates,
        ctx.fixed.clone(),
        ctx.query.budget,
        Shape::of(&ctx.question),
    );
    ctx.trace
        .record(format!("{}.pack", ctx.tier.as_str()), started.elapsed());
    Ok(())
}
