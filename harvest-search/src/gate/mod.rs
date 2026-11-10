pub mod completeness;
pub mod dangling;
pub mod judge;
pub mod score;
pub mod signals;

use async_trait::async_trait;

use crate::context::SearchContext;
use crate::domain::Gap;
use crate::trace::GateRecord;

#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Sufficient,
    Insufficient(Vec<Gap>),
    Undecided,
}

impl Verdict {
    fn label(&self) -> &'static str {
        match self {
            Self::Sufficient => "sufficient",
            Self::Insufficient(_) => "insufficient",
            Self::Undecided => "undecided",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    Answer,
    Escalate(Vec<Gap>),
}

#[async_trait]
pub trait SufficiencyCheck: Send + Sync {
    fn name(&self) -> &'static str;
    fn uses_model(&self) -> bool {
        false
    }
    async fn check(&self, ctx: &SearchContext) -> Verdict;
}

#[derive(Default)]
pub struct Gate {
    checks: Vec<Box<dyn SufficiencyCheck>>,
}

impl Gate {
    pub fn new(checks: Vec<Box<dyn SufficiencyCheck>>) -> Self {
        Self { checks }
    }

    pub async fn decide(&self, ctx: &mut SearchContext) -> Decision {
        ctx.trace.signals = Some(signals::Signals::compute(ctx));
        for check in &self.checks {
            let verdict = check.check(ctx).await;
            if check.uses_model() {
                ctx.trace.llm_calls += 1;
            }
            ctx.trace.gate.push(GateRecord {
                tier: ctx.tier.as_str().to_string(),
                check: check.name().to_string(),
                verdict: verdict.label().to_string(),
            });
            match verdict {
                Verdict::Sufficient => return Decision::Answer,
                Verdict::Insufficient(gaps) => return Decision::Escalate(gaps),
                Verdict::Undecided => continue,
            }
        }
        Decision::Escalate(Vec::new())
    }
}
