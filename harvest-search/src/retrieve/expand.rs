use std::collections::HashMap;

use crate::domain::{Source, UnitId};
use crate::index::graph::EdgeWeights;
use crate::index::IndexSnapshot;
use crate::retrieve::Candidate;

const SEEDS: usize = 20;
const RESTART: f32 = 0.35;
const ITERATIONS: usize = 12;
const GRAPH_WEIGHT: f32 = 0.5;

pub fn expand(
    index: &IndexSnapshot,
    candidates: Vec<Candidate>,
    weights: &EdgeWeights,
    limit: usize,
) -> Vec<Candidate> {
    let seeds: Vec<(UnitId, f32)> = candidates
        .iter()
        .take(SEEDS)
        .map(|c| (c.unit, c.rrf.max(1e-4)))
        .collect();
    let ranked = index
        .graph
        .personalized_pagerank(&seeds, weights, RESTART, ITERATIONS);
    let top_graph = ranked.first().map_or(0.0, |(_, s)| *s);
    let top_rrf = candidates
        .iter()
        .map(|c| c.rrf)
        .fold(0.0f32, f32::max)
        .max(1e-6);
    let mut merged: HashMap<UnitId, Candidate> =
        candidates.into_iter().map(|c| (c.unit, c)).collect();
    if top_graph > 0.0 {
        for (unit, score) in ranked.into_iter().take(limit) {
            let candidate = merged.entry(unit).or_insert_with(|| Candidate {
                unit,
                sources: vec![Source::Graph],
                ..Candidate::default()
            });
            candidate.graph = score / top_graph;
        }
    }
    let mut expanded: Vec<Candidate> = merged
        .into_values()
        .map(|mut c| {
            c.score = c.rrf / top_rrf + GRAPH_WEIGHT * c.graph;
            c
        })
        .collect();
    expanded.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.unit.cmp(&b.unit)));
    expanded.truncate(limit);
    expanded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::{Corpus, Relation, Unit};
    use crate::index::graph::Direction;
    use crate::index::vectors::HashingEmbedder;

    #[tokio::test]
    async fn graph_neighbors_of_strong_seeds_join_the_candidates() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        for name in ["attach", "do_attach", "unrelated"] {
            corpus.push_unit(Unit {
                name: name.into(),
                ..Unit::default()
            });
        }
        corpus.link(0, 1, Relation::Calls);
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let seeds = vec![Candidate {
            unit: 0,
            rrf: 0.03,
            score: 0.03,
            ..Candidate::default()
        }];
        let weights = EdgeWeights::new(&[(Relation::Calls, Direction::Out, 1.0)]);
        let expanded = expand(&index, seeds, &weights, 10);
        let units: Vec<UnitId> = expanded.iter().map(|c| c.unit).collect();
        assert_eq!(units[0], 0);
        assert!(units.contains(&1));
        assert!(!units.contains(&2));
        let callee = expanded.iter().find(|c| c.unit == 1).unwrap();
        assert!(callee.graph > 0.0 && callee.sources == vec![Source::Graph]);
    }
}
