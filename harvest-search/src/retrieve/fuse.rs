use std::collections::HashMap;

use crate::domain::{Source, UnitId};
use crate::retrieve::{Candidate, Hit};

const RRF_K: f32 = 60.0;

pub fn fuse(lists: Vec<Vec<Hit>>) -> Vec<Candidate> {
    let mut merged: HashMap<UnitId, Candidate> = HashMap::new();
    for list in lists {
        for (rank, hit) in list.into_iter().enumerate() {
            let candidate = merged.entry(hit.unit).or_insert_with(|| Candidate {
                unit: hit.unit,
                ..Candidate::default()
            });
            candidate.rrf += 1.0 / (RRF_K + rank as f32 + 1.0);
            match hit.source {
                Source::Lexical => candidate.lexical = candidate.lexical.max(hit.score),
                Source::Dense => candidate.dense = candidate.dense.max(hit.score),
                Source::Exact => candidate.exact = true,
                _ => {}
            }
            if !candidate.sources.contains(&hit.source) {
                candidate.sources.push(hit.source);
            }
        }
    }
    let mut fused: Vec<Candidate> = merged
        .into_values()
        .map(|mut c| {
            c.score = c.rrf;
            c
        })
        .collect();
    fused.sort_by(|a, b| b.rrf.total_cmp(&a.rrf).then(a.unit.cmp(&b.unit)));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(unit: UnitId, score: f32, source: Source) -> Hit {
        Hit {
            unit,
            score,
            source,
        }
    }

    #[test]
    fn agreement_between_lists_wins_over_a_single_top_rank() {
        let fused = fuse(vec![
            vec![hit(1, 9.0, Source::Lexical), hit(2, 5.0, Source::Lexical)],
            vec![hit(3, 0.9, Source::Dense), hit(2, 0.8, Source::Dense)],
            vec![hit(2, 1.0, Source::Exact)],
        ]);
        assert_eq!(fused[0].unit, 2);
        assert!(fused[0].exact);
        assert_eq!(fused[0].lexical, 5.0);
        assert_eq!(fused[0].dense, 0.8);
        assert_eq!(fused[0].sources.len(), 3);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn empty_input_fuses_to_nothing() {
        assert!(fuse(vec![vec![], vec![]]).is_empty());
    }
}
