use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use harvest_search::context::SearchContext;
use harvest_search::gate::score::CalibratedScore;
use harvest_search::index::IndexRegistry;
use harvest_search::retrieve::rerank::feature_matrix;
use harvest_search::tiers::rounds::Rounds;
use harvest_search::{Deadline, EvidenceKey};

use crate::gold::GoldQuery;
use crate::metrics::quality::matches_key;
use crate::report::{QueryRecord, INSUFFICIENT_BELOW};
use crate::run::query_for;

pub use harvest_search::calibration::{logistic, train_reranker};

pub fn calibrate_gate(records: &[QueryRecord]) -> Option<CalibratedScore> {
    let mut rows = Vec::new();
    let mut labels = Vec::new();
    for record in records.iter().filter(|r| r.error.is_none()) {
        let (Some(signals), Some(recall)) = (
            &record.signals,
            record
                .tier1_recall
                .or((record.tier == "tier1").then_some(record.recall_essential)),
        ) else {
            continue;
        };
        let Some(row) = harvest_search::calibration::gate_row(signals) else {
            continue;
        };
        rows.push(row);
        labels.push(recall >= INSUFFICIENT_BELOW);
    }
    harvest_search::calibration::calibrate_gate(&rows, &labels)
}

pub async fn reranker_rows(
    rounds: &Rounds,
    registry: &Arc<IndexRegistry>,
    gold: &[GoldQuery],
) -> Result<(Vec<Vec<f32>>, Vec<bool>)> {
    let mut rows = Vec::new();
    let mut labels = Vec::new();
    for item in gold {
        let query = query_for(item, 8_000);
        let Some(index) = registry.ensure(&query.scope).await? else {
            continue;
        };
        let mut ctx = SearchContext::new(&query, index, Deadline::after(Duration::from_secs(30)));
        rounds.gather(&mut ctx).await;
        let matrix = feature_matrix(&ctx.question, &ctx.index, &ctx.candidates);
        for (candidate, features) in ctx.candidates.iter().zip(matrix) {
            let unit = ctx.index.unit(candidate.unit);
            let key: EvidenceKey = unit.key();
            let lines = Some((unit.start_line, unit.end_line));
            rows.push(features.to_vec());
            labels.push(item.evidence.iter().any(|g| matches_key(g, &key, lines)));
        }
    }
    Ok((rows, labels))
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::gate::score::sigmoid;
    use harvest_search::gate::signals::Signals;

    #[test]
    fn logistic_regression_separates_a_simple_rule() {
        let rows: Vec<Vec<f32>> = (0..200)
            .map(|i| vec![(i % 10) as f32 / 10.0, ((i * 7) % 3) as f32])
            .collect();
        let labels: Vec<bool> = rows.iter().map(|r| r[0] > 0.45).collect();
        let (weights, bias) = logistic(&rows, &labels, false);
        assert!(weights[0] > 1.0, "{weights:?}");
        let correct = rows
            .iter()
            .zip(&labels)
            .filter(|(r, l)| (sigmoid(bias + r[0] * weights[0] + r[1] * weights[1]) > 0.5) == **l)
            .count();
        assert!(correct >= 180, "{correct}");
    }

    #[test]
    fn gate_calibration_keeps_missed_insufficiency_near_the_target() {
        let mut records = Vec::new();
        for i in 0..200 {
            let good = i % 2 == 0;
            records.push(QueryRecord {
                id: i.to_string(),
                tier: "tier1".into(),
                tier1_recall: Some(if good { 1.0 } else { 0.3 }),
                signals: Some(Signals {
                    top_stable: good,
                    score_gap: if good { 0.6 } else { 0.1 },
                    agreement: if good { 0.8 } else { 0.2 } + (i % 7) as f32 / 50.0,
                    identifier_coverage: 1.0,
                    evidence: 6,
                    dangling: 0,
                    list_coverage: None,
                    top_probability: if good { 0.9 } else { 0.4 },
                    relevant_unpacked: 0.0,
                }),
                ..QueryRecord::default()
            });
        }
        let score = calibrate_gate(&records).unwrap();
        let missed = records
            .iter()
            .filter(|r| r.tier1_recall < Some(INSUFFICIENT_BELOW))
            .filter(|r| score.probability(r.signals.as_ref().unwrap()) >= score.accept)
            .count();
        assert!(missed <= 5, "{missed} missed at accept {}", score.accept);
        assert!(score.reject < score.accept);
        assert!(calibrate_gate(&records[..1]).is_none());
    }
}
