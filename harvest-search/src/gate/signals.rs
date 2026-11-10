use serde::{Deserialize, Serialize};

use crate::context::SearchContext;
use crate::gate::{completeness, dangling};
use crate::index::tokenizer::tokens;

pub const SIGNALS: usize = 7;

pub const SIGNAL_NAMES: [&str; SIGNALS] = [
    "top_stable",
    "score_gap",
    "agreement",
    "identifier_coverage",
    "evidence",
    "dangling",
    "list_coverage",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Signals {
    pub top_stable: bool,
    pub score_gap: f32,
    pub agreement: f32,
    pub identifier_coverage: f32,
    pub evidence: usize,
    pub dangling: usize,
    pub list_coverage: Option<f32>,
}

impl Signals {
    pub fn compute(ctx: &SearchContext) -> Self {
        let scores: Vec<f32> = ctx.candidates.iter().take(2).map(|c| c.score).collect();
        let score_gap = match scores.as_slice() {
            [first, second, ..] if *first > 0.0 => ((first - second) / first).clamp(0.0, 1.0),
            [_] => 1.0,
            _ => 0.0,
        };
        let top: Vec<_> = ctx.candidates.iter().take(5).collect();
        let agreement = if top.is_empty() {
            0.0
        } else {
            top.iter().filter(|c| c.sources.len() >= 2).count() as f32 / top.len() as f32
        };
        let identifier_coverage = if ctx.question.identifiers.is_empty() {
            1.0
        } else {
            let haystack: std::collections::HashSet<String> = ctx
                .evidence
                .iter()
                .flat_map(|e| tokens(&format!("{} {}", e.label, e.text)))
                .collect();
            let found = ctx
                .question
                .identifiers
                .iter()
                .filter(|i| {
                    tokens(i)
                        .first()
                        .is_some_and(|whole| haystack.contains(whole))
                })
                .count();
            found as f32 / ctx.question.identifiers.len() as f32
        };
        Self {
            top_stable: ctx.stable,
            score_gap,
            agreement,
            identifier_coverage,
            evidence: ctx.evidence.len(),
            dangling: dangling::dangling(ctx).len(),
            list_coverage: completeness::coverage(ctx).map(|c| c.ratio()),
        }
    }

    pub fn features(&self) -> [f32; SIGNALS] {
        [
            f32::from(u8::from(self.top_stable)),
            self.score_gap,
            self.agreement,
            self.identifier_coverage,
            (self.evidence as f32 / 10.0).min(1.0),
            (self.dangling as f32 / 4.0).min(1.0),
            self.list_coverage.unwrap_or(1.0),
        ]
    }
}
