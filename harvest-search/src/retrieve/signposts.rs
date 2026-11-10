use crate::domain::Source;
use crate::index::corpus::Relation;
use crate::index::graph::Direction;
use crate::index::IndexSnapshot;
use crate::retrieve::Candidate;

pub const SIGNPOST_DECAY: f32 = 0.8;
pub const MAX_FOLLOWED: usize = 10;

pub struct Split {
    pub code: Vec<Candidate>,
    pub documentation: Vec<Candidate>,
}

pub fn split_by_trust(index: &IndexSnapshot, candidates: Vec<Candidate>) -> Split {
    let (documentation, code) = candidates
        .into_iter()
        .partition(|c| index.unit(c.unit).kind().is_documentation());
    Split {
        code,
        documentation,
    }
}

fn pointed_from(doc: &Candidate) -> Candidate {
    Candidate {
        unit: doc.unit,
        score: doc.score * SIGNPOST_DECAY,
        rrf: doc.rrf * SIGNPOST_DECAY,
        lexical: doc.lexical * SIGNPOST_DECAY,
        dense: doc.dense * SIGNPOST_DECAY,
        graph: doc.graph,
        sources: vec![Source::Signpost],
        ..Candidate::default()
    }
}

pub fn follow(
    index: &IndexSnapshot,
    mut code: Vec<Candidate>,
    documentation: &[Candidate],
) -> Vec<Candidate> {
    for doc in documentation.iter().take(MAX_FOLLOWED) {
        for target in index
            .graph
            .neighbors(doc.unit, Direction::Out, &[Relation::Documents])
        {
            if index.unit(target).kind().is_documentation() {
                continue;
            }
            match code.iter_mut().find(|c| c.unit == target) {
                Some(existing) => {
                    if !existing.sources.contains(&Source::Signpost) {
                        existing.sources.push(Source::Signpost);
                    }
                }
                None => code.push(Candidate {
                    unit: target,
                    ..pointed_from(doc)
                }),
            }
        }
    }
    code
}

pub fn merge(kept: &mut Vec<Candidate>, found: Vec<Candidate>) {
    for candidate in found {
        match kept.iter_mut().find(|k| k.unit == candidate.unit) {
            Some(existing) if candidate.rrf > existing.rrf => *existing = candidate,
            Some(_) => {}
            None => kept.push(candidate),
        }
    }
    kept.sort_by(|a, b| b.rrf.total_cmp(&a.rrf).then(a.unit.cmp(&b.unit)));
}

pub fn is_signposted(candidate: &Candidate) -> bool {
    candidate.sources.contains(&Source::Signpost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::{Corpus, Unit, UnitKind};
    use crate::index::vectors::HashingEmbedder;

    fn unit(name: &str, kind: UnitKind, body: &str) -> Unit {
        Unit {
            name: name.into(),
            qualified_name: name.into(),
            kind: Some(kind),
            file: format!("{name}.src"),
            body: body.into(),
            ..Unit::default()
        }
    }

    fn candidate(unit: u32, rrf: f32) -> Candidate {
        Candidate {
            unit,
            rrf,
            ..Candidate::default()
        }
    }

    async fn index() -> IndexSnapshot {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        corpus.push_unit(unit(
            "Install guide",
            UnitKind::Doc,
            "Run `install_cluster` with `MaasClient`.",
        ));
        corpus.push_unit(unit("install_cluster", UnitKind::Function, ""));
        corpus.push_unit(unit("MaasClient", UnitKind::Class, ""));
        corpus.push_unit(unit("unrelated", UnitKind::Function, ""));
        IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn documentation_is_split_out_and_followed_to_the_code_it_names() {
        let index = index().await;
        let Split {
            code,
            documentation,
        } = split_by_trust(
            &index,
            vec![candidate(0, 0.05), candidate(2, 0.01), candidate(3, 0.02)],
        );
        assert_eq!(
            documentation.iter().map(|c| c.unit).collect::<Vec<_>>(),
            vec![0]
        );
        let followed = follow(&index, code, &documentation);
        let units: Vec<u32> = followed.iter().map(|c| c.unit).collect();
        assert!(units.contains(&1), "the named function is added");
        assert!(
            !units.contains(&0),
            "the documentation itself is never code evidence"
        );
        let maas = followed.iter().find(|c| c.unit == 2).unwrap();
        assert!(
            is_signposted(maas),
            "code already found keeps its scores and gains the signpost source"
        );
        assert_eq!(maas.rrf, 0.01);
        let added = followed.iter().find(|c| c.unit == 1).unwrap();
        assert!((added.rrf - 0.05 * SIGNPOST_DECAY).abs() < 1e-6);
        assert!(!is_signposted(
            followed.iter().find(|c| c.unit == 3).unwrap()
        ));
    }

    #[test]
    fn merging_keeps_the_best_score_per_document() {
        let mut kept = vec![candidate(0, 0.02)];
        merge(&mut kept, vec![candidate(0, 0.05), candidate(4, 0.03)]);
        assert_eq!(
            kept.iter().map(|c| (c.unit, c.rrf)).collect::<Vec<_>>(),
            vec![(0, 0.05), (4, 0.03)]
        );
    }
}
