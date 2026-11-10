use crate::index::corpus::Relation;
use crate::index::graph::Direction;
use crate::index::IndexSnapshot;
use crate::retrieve::Candidate;

const MAX_NAMES: usize = 12;

const FOLLOWED: [Relation; 7] = [
    Relation::Calls,
    Relation::Inherits,
    Relation::Implements,
    Relation::Uses,
    Relation::ReadsConfig,
    Relation::Instantiates,
    Relation::Overrides,
];

pub fn extract(index: &IndexSnapshot, candidates: &[Candidate], top: usize) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let push = |name: &str, names: &mut Vec<String>| {
        let name = name.trim();
        if name.len() >= 3 && !names.iter().any(|n| n == name) && names.len() < MAX_NAMES {
            names.push(name.to_string());
        }
    };
    for candidate in candidates.iter().take(top) {
        let unit = index.unit(candidate.unit);
        push(&unit.qualified_name, &mut names);
        for base in &unit.bases {
            push(base, &mut names);
        }
        for decorator in &unit.decorators {
            push(decorator.trim_start_matches('@'), &mut names);
        }
        for neighbor in index
            .graph
            .neighbors(candidate.unit, Direction::Out, &FOLLOWED)
        {
            push(&index.unit(neighbor).qualified_name, &mut names);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::{Corpus, Unit};
    use crate::index::vectors::HashingEmbedder;

    #[tokio::test]
    async fn names_come_from_top_hits_their_bases_and_their_callees() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        corpus.push_unit(Unit {
            name: "LVMDriver".into(),
            bases: vec!["VolumeDriver".into()],
            decorators: vec!["@interface.volumedriver".into()],
            ..Unit::default()
        });
        corpus.push_unit(Unit {
            name: "create_lv".into(),
            ..Unit::default()
        });
        corpus.push_unit(Unit {
            name: "ignored".into(),
            ..Unit::default()
        });
        corpus.link(0, 1, Relation::Calls);
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let candidates = vec![Candidate {
            unit: 0,
            ..Candidate::default()
        }];
        assert_eq!(
            extract(&index, &candidates, 5),
            vec![
                "LVMDriver",
                "VolumeDriver",
                "interface.volumedriver",
                "create_lv"
            ]
        );
    }
}
