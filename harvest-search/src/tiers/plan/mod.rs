pub mod execute;
pub mod step;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use crate::context::SearchContext;
use crate::domain::{SearchError, Source, TierId};
use crate::llm::LanguageModel;
use crate::retrieve::merge_pinned;
use crate::retrieve::rerank::Reranker;
use crate::tiers::{rerank_and_pack, Tier};
use execute::{Executed, Executor};
use step::{Step, STEP_GUIDE};

const MAX_STEPS: usize = 12;
const EVIDENCE_LABELS: usize = 15;

const SYSTEM: &str = "You plan code searches. You see a question about a codebase, the evidence found so far and what is still missing. Write a short retrieval plan using only the listed operations. Reply with JSON only.";

pub fn parse_steps(reply: &Value, max: usize) -> Result<Vec<Step>, SearchError> {
    let items = reply["steps"]
        .as_array()
        .ok_or_else(|| SearchError::Plan("the reply has no \"steps\" array".into()))?;
    let steps: Vec<Step> = items
        .iter()
        .filter_map(|item| match serde_json::from_value::<Step>(item.clone()) {
            Ok(step) => Some(step),
            Err(e) => {
                tracing::debug!(error = %e, %item, "skipping an invalid plan step");
                None
            }
        })
        .take(max)
        .collect();
    Ok(steps)
}

pub fn situation(ctx: &SearchContext) -> String {
    let evidence: Vec<String> = ctx
        .evidence
        .iter()
        .take(EVIDENCE_LABELS)
        .map(|e| format!("- {}", e.label))
        .collect();
    let gaps: Vec<String> = ctx
        .gaps
        .iter()
        .map(|g| format!("- {}", g.describe()))
        .collect();
    format!(
        "Question: {}\n\nEvidence so far:\n{}\n\nMissing:\n{}\n\n{STEP_GUIDE}",
        ctx.query.text,
        if evidence.is_empty() {
            "- nothing yet".to_string()
        } else {
            evidence.join("\n")
        },
        if gaps.is_empty() {
            "- unknown".to_string()
        } else {
            gaps.join("\n")
        },
    )
}

pub fn apply(ctx: &mut SearchContext, executed: Executed) {
    merge_pinned(
        &mut ctx.candidates,
        executed
            .candidates
            .iter()
            .filter(|c| c.pinned)
            .cloned()
            .collect(),
    );
    for candidate in executed.candidates.into_iter().filter(|c| !c.pinned) {
        if !ctx.candidates.iter().any(|c| c.unit == candidate.unit) {
            ctx.candidates.push(candidate);
        }
    }
    for table in executed.tables {
        ctx.fixed.retain(|t| t.key != table.key);
        ctx.fixed.push(table);
    }
    for output in executed.outputs {
        ctx.notes.push(output.summary);
    }
}

pub struct Plan {
    model: Arc<dyn LanguageModel>,
    reranker: Arc<dyn Reranker>,
}

impl Plan {
    pub fn new(model: Arc<dyn LanguageModel>, reranker: Arc<dyn Reranker>) -> Self {
        Self { model, reranker }
    }
}

#[async_trait]
impl Tier for Plan {
    fn id(&self) -> TierId {
        TierId::Plan
    }

    fn expected_cost(&self) -> Duration {
        Duration::from_millis(2_500)
    }

    async fn run(&self, ctx: &mut SearchContext) -> Result<(), SearchError> {
        let prompt = format!(
            "{}\n\nReply as {{\"steps\": [ ... ]}} with at most {MAX_STEPS} steps.",
            situation(ctx)
        );
        let started = Instant::now();
        ctx.trace.llm_calls += 1;
        let reply = self
            .model
            .complete_json(SYSTEM, &prompt, ctx.deadline)
            .await;
        ctx.trace.record("tier2.plan_llm", started.elapsed());
        let steps = parse_steps(&reply?, MAX_STEPS)?;
        let started = Instant::now();
        let executed = Executor::new(&ctx.index, Source::Plan).run(&steps, &[]);
        apply(ctx, executed);
        ctx.trace.record("tier2.execute", started.elapsed());
        rerank_and_pack(ctx, self.reranker.as_ref()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invalid_steps_are_skipped_and_the_count_is_capped() {
        let reply = json!({"steps": [
            {"op": "search", "terms": "a"},
            {"op": "bogus"},
            {"op": "search", "terms": "b"},
            {"op": "search", "terms": "c"}
        ]});
        assert_eq!(parse_steps(&reply, 2).unwrap().len(), 2);
        assert!(parse_steps(&json!({"plan": []}), 5).is_err());
    }
}
