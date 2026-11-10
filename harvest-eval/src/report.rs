use std::collections::BTreeMap;

use harvest_search::gate::signals::Signals;
use serde::{Deserialize, Serialize};

use crate::metrics::stats::{bootstrap_mean_ci, mean, Latency};

pub const INSUFFICIENT_BELOW: f64 = 0.9;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QueryRecord {
    pub id: String,
    pub kind: String,
    pub question: String,
    pub tier: String,
    #[serde(default)]
    pub degraded: Option<String>,
    pub recall_essential: f64,
    pub recall_all: f64,
    #[serde(default)]
    pub pool_recall: Option<f64>,
    pub ndcg: f64,
    pub precision: f64,
    pub e2e_ms: f64,
    #[serde(default)]
    pub stages: BTreeMap<String, f64>,
    pub llm_calls: u32,
    pub evidence: usize,
    #[serde(default)]
    pub tier1_said_sufficient: Option<bool>,
    #[serde(default)]
    pub tier1_recall: Option<f64>,
    #[serde(default)]
    pub signals: Option<Signals>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub judge: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JudgeSummary {
    pub wins: usize,
    pub ties: usize,
    pub losses: usize,
    pub margin: f64,
    pub margin_ci: (f64, f64),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub queries: usize,
    pub errors: usize,
    pub recall_essential: f64,
    pub recall_all: f64,
    pub pool_recall: Option<f64>,
    pub ndcg: f64,
    pub precision: f64,
    pub latency: Latency,
    pub latency_by_tier: BTreeMap<String, Latency>,
    pub stage_p99: BTreeMap<String, f64>,
    pub escalation: BTreeMap<String, f64>,
    pub degraded: BTreeMap<String, f64>,
    pub budget_compliance: f64,
    pub max_llm_calls_by_tier: BTreeMap<String, u32>,
    pub missed_insufficiency: Option<f64>,
    pub unneeded_escalation: Option<f64>,
    pub recall_by_kind: BTreeMap<String, f64>,
    pub judge: Option<JudgeSummary>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub engine: String,
    pub split: String,
    pub gold: String,
    #[serde(default)]
    pub commit: Option<String>,
    pub created_at: String,
    pub budget_ms: u64,
    #[serde(default)]
    pub index_load_ms: BTreeMap<String, f64>,
    pub records: Vec<QueryRecord>,
    pub summary: Summary,
}

fn share<F: Fn(&QueryRecord) -> bool>(records: &[&QueryRecord], predicate: F) -> f64 {
    if records.is_empty() {
        return 0.0;
    }
    records.iter().filter(|r| predicate(r)).count() as f64 / records.len() as f64
}

pub fn summarize(records: &[QueryRecord], budget_ms: u64) -> Summary {
    let ok: Vec<&QueryRecord> = records.iter().filter(|r| r.error.is_none()).collect();
    let values = |f: fn(&QueryRecord) -> f64| ok.iter().map(|r| f(r)).collect::<Vec<f64>>();
    let latencies = values(|r| r.e2e_ms);

    let mut by_tier: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut max_calls: BTreeMap<String, u32> = BTreeMap::new();
    let mut stages: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for r in &ok {
        by_tier.entry(r.tier.clone()).or_default().push(r.e2e_ms);
        let calls = max_calls.entry(r.tier.clone()).or_default();
        *calls = (*calls).max(r.llm_calls);
        for (stage, ms) in &r.stages {
            stages.entry(stage.clone()).or_default().push(*ms);
        }
        by_kind
            .entry(if r.kind.is_empty() {
                "unknown".into()
            } else {
                r.kind.clone()
            })
            .or_default()
            .push(r.recall_essential);
    }
    let tiers: Vec<String> = by_tier.keys().cloned().collect();
    let pools: Vec<f64> = ok.iter().filter_map(|r| r.pool_recall).collect();

    let labelled: Vec<&&QueryRecord> = ok
        .iter()
        .filter(|r| r.tier1_said_sufficient.is_some() && r.tier1_recall.is_some())
        .collect();
    let insufficient: Vec<&&&QueryRecord> = labelled
        .iter()
        .filter(|r| r.tier1_recall.unwrap_or(0.0) < INSUFFICIENT_BELOW)
        .collect();
    let sufficient: Vec<&&&QueryRecord> = labelled
        .iter()
        .filter(|r| r.tier1_recall.unwrap_or(0.0) >= INSUFFICIENT_BELOW)
        .collect();
    let rate = |group: &[&&&QueryRecord], said: bool| {
        (!group.is_empty()).then(|| {
            group
                .iter()
                .filter(|r| r.tier1_said_sufficient == Some(said))
                .count() as f64
                / group.len() as f64
        })
    };

    let mut degraded: BTreeMap<String, f64> = BTreeMap::new();
    for reason in ok.iter().filter_map(|r| r.degraded.clone()) {
        *degraded.entry(reason).or_default() += 1.0 / ok.len() as f64;
    }

    Summary {
        queries: records.len(),
        errors: records.len() - ok.len(),
        recall_essential: mean(&values(|r| r.recall_essential)),
        recall_all: mean(&values(|r| r.recall_all)),
        pool_recall: (!pools.is_empty()).then(|| mean(&pools)),
        ndcg: mean(&values(|r| r.ndcg)),
        precision: mean(&values(|r| r.precision)),
        latency: Latency::of(&latencies),
        latency_by_tier: by_tier
            .iter()
            .map(|(tier, values)| (tier.clone(), Latency::of(values)))
            .collect(),
        stage_p99: stages
            .iter()
            .map(|(stage, values)| (stage.clone(), Latency::of(values).p99))
            .collect(),
        escalation: tiers
            .iter()
            .map(|tier| (tier.clone(), share(&ok, |r| &r.tier == tier)))
            .collect(),
        degraded,
        budget_compliance: share(&ok, |r| r.e2e_ms <= budget_ms as f64),
        max_llm_calls_by_tier: max_calls,
        missed_insufficiency: rate(&insufficient, true),
        unneeded_escalation: rate(&sufficient, false),
        recall_by_kind: by_kind
            .iter()
            .map(|(kind, values)| (kind.clone(), mean(values)))
            .collect(),
        judge: judge_summary(&ok),
    }
}

fn judge_summary(records: &[&QueryRecord]) -> Option<JudgeSummary> {
    let outcomes: Vec<f64> = records
        .iter()
        .filter_map(|r| r.judge.as_deref())
        .map(|verdict| match verdict {
            "win" => 1.0,
            "loss" => -1.0,
            _ => 0.0,
        })
        .collect();
    if outcomes.is_empty() {
        return None;
    }
    Some(JudgeSummary {
        wins: outcomes.iter().filter(|o| **o > 0.0).count(),
        ties: outcomes.iter().filter(|o| **o == 0.0).count(),
        losses: outcomes.iter().filter(|o| **o < 0.0).count(),
        margin: mean(&outcomes),
        margin_ci: bootstrap_mean_ci(&outcomes, 2000, 17),
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gates {
    pub recall_ratio_vs_loop: f64,
    pub max_recall_drop: f64,
    pub p99_ms: f64,
    pub tier1_p99_ms: f64,
    pub max_missed_insufficiency: f64,
    pub min_judge_margin: f64,
    pub max_tier2_llm_calls: u32,
    pub max_tier3_llm_calls: u32,
}

impl Default for Gates {
    fn default() -> Self {
        Self {
            recall_ratio_vs_loop: 0.95,
            max_recall_drop: 0.01,
            p99_ms: 10_000.0,
            tier1_p99_ms: 500.0,
            max_missed_insufficiency: 0.05,
            min_judge_margin: -0.03,
            max_tier2_llm_calls: 2,
            max_tier3_llm_calls: 8,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

fn gate(name: &str, passed: bool, detail: String) -> GateResult {
    GateResult {
        name: name.into(),
        passed,
        detail,
    }
}

pub fn paired_recall_deltas(base: &Report, new: &Report) -> Vec<f64> {
    let before: BTreeMap<&str, f64> = base
        .records
        .iter()
        .filter(|r| r.error.is_none())
        .map(|r| (r.id.as_str(), r.recall_essential))
        .collect();
    new.records
        .iter()
        .filter(|r| r.error.is_none())
        .filter_map(|r| before.get(r.id.as_str()).map(|b| r.recall_essential - b))
        .collect()
}

pub fn compare(base: &Report, new: &Report, gates: &Gates) -> Vec<GateResult> {
    let mut results = Vec::new();
    let (b, n) = (&base.summary, &new.summary);
    if base.engine == "legacy" {
        let target = gates.recall_ratio_vs_loop * b.recall_essential;
        results.push(gate(
            "recall_vs_loop",
            n.recall_essential >= target,
            format!(
                "essential recall {:.3} vs loop {:.3} (needs ≥ {:.3})",
                n.recall_essential, b.recall_essential, target
            ),
        ));
    } else {
        let deltas = paired_recall_deltas(base, new);
        let (low, high) = bootstrap_mean_ci(&deltas, 2000, 11);
        results.push(gate(
            "recall_no_regression",
            deltas.is_empty() || low >= -gates.max_recall_drop,
            format!(
                "paired essential recall change {:+.3} (95% CI {low:+.3}..{high:+.3}) over {} queries",
                mean(&deltas),
                deltas.len()
            ),
        ));
    }
    results.push(gate(
        "p99_latency",
        n.latency.p99 <= gates.p99_ms,
        format!("p99 {:.0} ms (limit {:.0})", n.latency.p99, gates.p99_ms),
    ));
    if let Some(tier1) = n.latency_by_tier.get("tier1") {
        results.push(gate(
            "tier1_p99_latency",
            tier1.p99 <= gates.tier1_p99_ms,
            format!(
                "tier 1 p99 {:.0} ms (limit {:.0})",
                tier1.p99, gates.tier1_p99_ms
            ),
        ));
    }
    for (tier, limit) in [
        ("tier1", 0),
        ("tier2", gates.max_tier2_llm_calls),
        ("tier3", gates.max_tier3_llm_calls),
    ] {
        if let Some(calls) = n.max_llm_calls_by_tier.get(tier) {
            results.push(gate(
                &format!("{tier}_llm_calls"),
                *calls <= limit,
                format!("at most {calls} model calls (limit {limit})"),
            ));
        }
    }
    if let Some(missed) = n.missed_insufficiency {
        results.push(gate(
            "missed_insufficiency",
            missed <= gates.max_missed_insufficiency,
            format!(
                "{:.1}% of insufficient tier 1 evidence was judged sufficient (limit {:.1}%)",
                missed * 100.0,
                gates.max_missed_insufficiency * 100.0
            ),
        ));
    }
    if let Some(judge) = &n.judge {
        results.push(gate(
            "answer_quality",
            judge.margin_ci.0 >= gates.min_judge_margin,
            format!(
                "win-loss margin {:+.3} (95% CI {:+.3}..{:+.3}); {} wins, {} ties, {} losses",
                judge.margin,
                judge.margin_ci.0,
                judge.margin_ci.1,
                judge.wins,
                judge.ties,
                judge.losses
            ),
        ));
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, tier: &str, recall: f64, ms: f64, calls: u32) -> QueryRecord {
        QueryRecord {
            id: id.into(),
            tier: tier.into(),
            recall_essential: recall,
            recall_all: recall,
            e2e_ms: ms,
            llm_calls: calls,
            ..QueryRecord::default()
        }
    }

    fn report(engine: &str, records: Vec<QueryRecord>) -> Report {
        Report {
            engine: engine.into(),
            summary: summarize(&records, 10_000),
            records,
            ..Report::default()
        }
    }

    #[test]
    fn summaries_split_latency_and_escalation_by_tier() {
        let mut errored = record("e", "tier1", 0.0, 0.0, 0);
        errored.error = Some("boom".into());
        let summary = summarize(
            &[
                record("a", "tier1", 1.0, 100.0, 0),
                record("b", "tier1", 0.5, 300.0, 0),
                record("c", "tier2", 1.0, 4_000.0, 1),
                record("d", "tier3", 0.0, 12_000.0, 5),
                errored,
            ],
            10_000,
        );
        assert_eq!((summary.queries, summary.errors), (5, 1));
        assert!((summary.recall_essential - 0.625).abs() < 1e-9);
        assert_eq!(summary.escalation["tier1"], 0.5);
        assert_eq!(summary.latency_by_tier["tier2"].p99, 4_000.0);
        assert_eq!(summary.budget_compliance, 0.75);
        assert_eq!(summary.max_llm_calls_by_tier["tier3"], 5);
    }

    #[test]
    fn missed_insufficiency_counts_confident_tier_one_mistakes() {
        let mut records = Vec::new();
        for (i, (said, recall)) in [
            (true, 0.5),
            (false, 0.5),
            (false, 0.4),
            (true, 1.0),
            (false, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut r = record(&i.to_string(), "tier1", recall, 10.0, 0);
            r.tier1_said_sufficient = Some(said);
            r.tier1_recall = Some(recall);
            records.push(r);
        }
        let summary = summarize(&records, 10_000);
        assert!((summary.missed_insufficiency.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        assert!((summary.unneeded_escalation.unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn comparing_against_the_loop_uses_the_recall_ratio_and_limits() {
        let base = report(
            "legacy",
            vec![
                record("a", "loop", 1.0, 30_000.0, 20),
                record("b", "loop", 0.8, 40_000.0, 20),
            ],
        );
        let good = report(
            "tiered",
            vec![
                record("a", "tier1", 1.0, 200.0, 0),
                record("b", "tier2", 0.75, 3_000.0, 1),
            ],
        );
        let results = compare(&base, &good, &Gates::default());
        assert!(results.iter().all(|g| g.passed), "{results:#?}");
        let slow = report(
            "tiered",
            vec![
                record("a", "tier1", 1.0, 900.0, 1),
                record("b", "tier2", 0.2, 11_000.0, 3),
            ],
        );
        let failed: Vec<String> = compare(&base, &slow, &Gates::default())
            .into_iter()
            .filter(|g| !g.passed)
            .map(|g| g.name)
            .collect();
        assert_eq!(
            failed,
            vec![
                "recall_vs_loop",
                "p99_latency",
                "tier1_p99_latency",
                "tier1_llm_calls",
                "tier2_llm_calls"
            ]
        );
    }

    #[test]
    fn comparing_two_runs_of_one_engine_checks_for_paired_regressions() {
        let base = report(
            "tiered",
            (0..50)
                .map(|i| record(&i.to_string(), "tier1", 1.0, 100.0, 0))
                .collect(),
        );
        let same = report(
            "tiered",
            (0..50)
                .map(|i| record(&i.to_string(), "tier1", 1.0, 100.0, 0))
                .collect(),
        );
        let worse = report(
            "tiered",
            (0..50)
                .map(|i| {
                    record(
                        &i.to_string(),
                        "tier1",
                        if i % 5 == 0 { 0.0 } else { 1.0 },
                        100.0,
                        0,
                    )
                })
                .collect(),
        );
        assert!(compare(&base, &same, &Gates::default())[0].passed);
        assert!(!compare(&base, &worse, &Gates::default())[0].passed);
    }
}
