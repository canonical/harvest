use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::SearchContext;
use crate::domain::Gap;
use crate::gate::signals::{Signals, SIGNALS};
use crate::gate::{SufficiencyCheck, Verdict};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibratedScore {
    pub weights: Vec<f32>,
    pub bias: f32,
    pub accept: f32,
    pub reject: f32,
}

impl Default for CalibratedScore {
    fn default() -> Self {
        Self {
            weights: vec![0.8, 1.0, 1.0, 1.5, 0.5, -1.5, 1.0],
            bias: -1.5,
            accept: 0.75,
            reject: 0.2,
        }
    }
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

impl CalibratedScore {
    pub fn probability(&self, signals: &Signals) -> f32 {
        let features: [f32; SIGNALS] = signals.features();
        sigmoid(
            self.bias
                + self
                    .weights
                    .iter()
                    .zip(features.iter())
                    .map(|(w, f)| w * f)
                    .sum::<f32>(),
        )
    }
}

#[async_trait]
impl SufficiencyCheck for CalibratedScore {
    fn name(&self) -> &'static str {
        "calibrated_score"
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        let signals = ctx
            .trace
            .signals
            .clone()
            .unwrap_or_else(|| Signals::compute(ctx));
        let p = self.probability(&signals);
        if p >= self.accept {
            Verdict::Sufficient
        } else if p <= self.reject {
            Verdict::Insufficient(vec![Gap::Unanswered {
                hint: format!("retrieval signals are weak (p={p:.2})"),
            }])
        } else {
            Verdict::Undecided
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_signals_score_higher_than_weak_ones() {
        let score = CalibratedScore::default();
        let strong = Signals {
            top_stable: true,
            score_gap: 0.6,
            agreement: 1.0,
            identifier_coverage: 1.0,
            evidence: 8,
            dangling: 0,
            list_coverage: None,
        };
        let weak = Signals {
            evidence: 1,
            dangling: 4,
            list_coverage: Some(0.1),
            ..Signals::default()
        };
        assert!(score.probability(&strong) > score.accept);
        assert!(score.probability(&weak) < score.reject);
        assert_eq!(score.weights.len(), SIGNALS);
    }
}
