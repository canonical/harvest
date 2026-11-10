use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::deadline::Deadline;
use crate::domain::{Degraded, Query, SearchError, SearchResult};
use crate::gate::{Decision, Gate};
use crate::index::IndexRegistry;
use crate::tiers::Tier;

#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, query: &Query, deadline: Deadline) -> Result<SearchResult, SearchError>;
}

pub struct TieredEngine {
    registry: Arc<IndexRegistry>,
    tiers: Vec<Box<dyn Tier>>,
    gate: Gate,
}

impl TieredEngine {
    pub fn new(registry: Arc<IndexRegistry>, tiers: Vec<Box<dyn Tier>>, gate: Gate) -> Self {
        Self {
            registry,
            tiers,
            gate,
        }
    }

    pub fn registry(&self) -> &Arc<IndexRegistry> {
        &self.registry
    }
}

#[async_trait]
impl SearchEngine for TieredEngine {
    async fn search(&self, query: &Query, deadline: Deadline) -> Result<SearchResult, SearchError> {
        let started = Instant::now();
        let index = self.registry.snapshot(&query.scope)?;
        let mut ctx = SearchContext::new(query, index, deadline);
        for (position, tier) in self.tiers.iter().enumerate() {
            if position > 0 && !deadline.leaves_room_for(tier.expected_cost()) {
                ctx.mark_truncated();
                break;
            }
            ctx.tier = tier.id();
            let tier_started = Instant::now();
            let outcome = tier.run(&mut ctx).await;
            ctx.trace.record(tier.id().as_str(), tier_started.elapsed());
            match outcome {
                Ok(()) => {}
                Err(SearchError::Model(message)) => {
                    tracing::warn!(tier = tier.id().as_str(), %message, "tier failed; answering with the evidence so far");
                    ctx.degraded = Some(Degraded::ModelUnavailable);
                    break;
                }
                Err(SearchError::Plan(message)) => {
                    ctx.notes.push(format!("plan rejected: {message}"));
                    continue;
                }
                Err(other) => return Err(other),
            }
            let gate_started = Instant::now();
            let decision = self.gate.decide(&mut ctx).await;
            ctx.trace.record(
                format!("{}.gate", tier.id().as_str()),
                gate_started.elapsed(),
            );
            match decision {
                Decision::Answer => {
                    ctx.gaps.clear();
                    break;
                }
                Decision::Escalate(gaps) => ctx.gaps = gaps,
            }
        }
        Ok(ctx.into_result(started))
    }
}

pub struct WithFallback<P, F> {
    primary: P,
    fallback: F,
}

impl<P, F> WithFallback<P, F> {
    pub fn new(primary: P, fallback: F) -> Self {
        Self { primary, fallback }
    }

    pub fn primary(&self) -> &P {
        &self.primary
    }
}

#[async_trait]
impl<P: SearchEngine, F: SearchEngine> SearchEngine for WithFallback<P, F> {
    async fn search(&self, query: &Query, deadline: Deadline) -> Result<SearchResult, SearchError> {
        match self.primary.search(query, deadline).await {
            Err(SearchError::IndexCold(_)) => {
                let mut result = self.fallback.search(query, deadline).await?;
                result.degraded = Some(Degraded::IndexCold);
                Ok(result)
            }
            other => other,
        }
    }
}
