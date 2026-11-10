use std::future::Future;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::domain::EvidenceKey;
use crate::gate::signals::Signals;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageTiming {
    pub stage: String,
    pub ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateRecord {
    pub tier: String,
    pub check: String,
    pub verdict: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Trace {
    pub stages: Vec<StageTiming>,
    pub llm_calls: u32,
    pub rounds: u32,
    pub gate: Vec<GateRecord>,
    pub signals: Option<Signals>,
    pub total_ms: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pool: Vec<EvidenceKey>,
}

impl Trace {
    pub fn record(&mut self, stage: impl Into<String>, elapsed: Duration) {
        self.stages.push(StageTiming {
            stage: stage.into(),
            ms: elapsed.as_secs_f64() * 1000.0,
        });
    }

    pub async fn time<T>(&mut self, stage: &str, work: impl Future<Output = T>) -> T {
        let started = Instant::now();
        let output = work.await;
        self.record(stage, started.elapsed());
        output
    }

    pub fn stage_ms(&self, stage: &str) -> f64 {
        self.stages
            .iter()
            .filter(|s| s.stage == stage)
            .map(|s| s.ms)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timed_stages_accumulate_by_name() {
        let mut trace = Trace::default();
        trace.record("seed", Duration::from_millis(2));
        trace.record("seed", Duration::from_millis(3));
        let value = trace.time("rerank", async { 7 }).await;
        assert_eq!(value, 7);
        assert!((trace.stage_ms("seed") - 5.0).abs() < 0.01);
        assert_eq!(trace.stages.len(), 3);
    }
}
