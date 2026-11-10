use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::{SearchError, Source, TierId};
use crate::llm::LanguageModel;
use crate::retrieve::rerank::Reranker;
use crate::tiers::plan::execute::{Executor, StepOutput};
use crate::tiers::plan::{apply, parse_steps, situation};
use crate::tiers::{rerank_and_pack, Tier};

const SYSTEM: &str = "You explore a codebase one move at a time to gather the evidence needed to answer a question. Each turn, choose the next one to three operations, or finish when the evidence is enough. Reply with JSON only.";
const STEPS_PER_TURN: usize = 3;
const MODEL_STEP_COST: Duration = Duration::from_millis(900);

pub struct Steps {
    model: Arc<dyn LanguageModel>,
    reranker: Arc<dyn Reranker>,
    max_turns: usize,
}

impl Steps {
    pub fn new(model: Arc<dyn LanguageModel>, reranker: Arc<dyn Reranker>) -> Self {
        Self {
            model,
            reranker,
            max_turns: 6,
        }
    }

    pub fn with_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns.max(1);
        self
    }
}

#[async_trait]
impl Tier for Steps {
    fn id(&self) -> TierId {
        TierId::Steps
    }

    fn expected_cost(&self) -> Duration {
        MODEL_STEP_COST * 2
    }

    async fn run(&self, ctx: &mut SearchContext) -> Result<(), SearchError> {
        let mut history: Vec<String> = Vec::new();
        let mut outputs: Vec<StepOutput> = Vec::new();
        for turn in 0..self.max_turns {
            if !ctx.deadline.leaves_room_for(MODEL_STEP_COST) {
                ctx.mark_truncated();
                break;
            }
            let prompt = format!(
                "{}\n\nSteps taken so far (earlier step indexes continue across turns):\n{}\n\nReply as {{\"steps\": [ ... ], \"done\": true | false}}.",
                situation(ctx),
                if history.is_empty() { "- none".to_string() } else { history.join("\n") }
            );
            let started = Instant::now();
            ctx.trace.llm_calls += 1;
            let reply = self
                .model
                .complete_json(SYSTEM, &prompt, ctx.deadline)
                .await;
            ctx.trace.record("tier3.step_llm", started.elapsed());
            let reply = reply?;
            let steps = parse_steps(&reply, STEPS_PER_TURN).unwrap_or_default();
            let started = Instant::now();
            let executed = Executor::new(&ctx.index, Source::Step).run(&steps, &outputs);
            for (offset, (step, output)) in steps.iter().zip(&executed.outputs).enumerate() {
                history.push(format!(
                    "[{}] {} -> {}",
                    outputs.len() + offset,
                    serde_json::to_string(step).unwrap_or_default(),
                    output.summary
                ));
            }
            outputs.extend(executed.outputs.iter().cloned());
            apply(ctx, executed);
            ctx.trace.record("tier3.execute", started.elapsed());
            rerank_and_pack(ctx, self.reranker.as_ref()).await?;
            if reply["done"].as_bool().unwrap_or(steps.is_empty()) {
                tracing::debug!(turn, "tier 3 finished");
                break;
            }
        }
        Ok(())
    }
}
