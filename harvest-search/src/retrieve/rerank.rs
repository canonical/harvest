use std::collections::HashSet;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::index::corpus::UnitKind;
use crate::index::tokenizer::tokens;
use crate::index::IndexSnapshot;
use crate::question::{Question, QuestionKind};
use crate::retrieve::{sort_candidates, Candidate};

pub const FEATURES: usize = 12;

pub const FEATURE_NAMES: [&str; FEATURES] = [
    "rrf",
    "lexical",
    "dense",
    "exact",
    "graph",
    "name_overlap",
    "path_overlap",
    "kind_prior",
    "is_test",
    "pinned",
    "is_symbol",
    "is_doc_like",
];

fn overlap(vocabulary: &HashSet<String>, text: &str) -> f32 {
    if vocabulary.is_empty() {
        return 0.0;
    }
    let found: HashSet<String> = tokens(text).into_iter().collect();
    vocabulary.iter().filter(|w| found.contains(*w)).count() as f32 / vocabulary.len() as f32
}

fn kind_prior(kind: QuestionKind, unit_kind: UnitKind) -> f32 {
    let matches = match kind {
        QuestionKind::Hierarchy => unit_kind == UnitKind::Class,
        QuestionKind::Flow | QuestionKind::Errors => {
            matches!(unit_kind, UnitKind::Function | UnitKind::Method)
        }
        QuestionKind::Config => matches!(unit_kind, UnitKind::Config | UnitKind::Doc),
        QuestionKind::Lookup => unit_kind.is_symbol(),
    };
    if matches {
        1.0
    } else {
        0.0
    }
}

pub fn features(
    question: &Question,
    index: &IndexSnapshot,
    candidate: &Candidate,
    max_lexical: f32,
) -> [f32; FEATURES] {
    let unit = index.unit(candidate.unit);
    let vocabulary: HashSet<String> = question.vocabulary().into_iter().collect();
    [
        candidate.rrf * 60.0,
        if max_lexical > 0.0 {
            candidate.lexical / max_lexical
        } else {
            0.0
        },
        candidate.dense,
        f32::from(u8::from(candidate.exact)),
        candidate.graph,
        overlap(&vocabulary, &unit.qualified_name),
        overlap(&vocabulary, &unit.file),
        kind_prior(question.kind, unit.kind()),
        f32::from(u8::from(unit.is_test())),
        f32::from(u8::from(candidate.pinned)),
        f32::from(u8::from(unit.kind().is_symbol())),
        f32::from(u8::from(matches!(
            unit.kind(),
            UnitKind::Doc | UnitKind::Config | UnitKind::Module
        ))),
    ]
}

pub fn feature_matrix(
    question: &Question,
    index: &IndexSnapshot,
    candidates: &[Candidate],
) -> Vec<[f32; FEATURES]> {
    let max_lexical = candidates.iter().map(|c| c.lexical).fold(0.0, f32::max);
    candidates
        .iter()
        .map(|c| features(question, index, c, max_lexical))
        .collect()
}

#[async_trait]
pub trait Reranker: Send + Sync {
    async fn rerank(
        &self,
        question: &Question,
        index: &IndexSnapshot,
        candidates: Vec<Candidate>,
    ) -> Result<Vec<Candidate>>;
}

pub struct PassThrough;

#[async_trait]
impl Reranker for PassThrough {
    async fn rerank(
        &self,
        _question: &Question,
        _index: &IndexSnapshot,
        mut candidates: Vec<Candidate>,
    ) -> Result<Vec<Candidate>> {
        sort_candidates(&mut candidates);
        Ok(candidates)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinearReranker {
    pub weights: Vec<f32>,
    pub bias: f32,
}

impl Default for LinearReranker {
    fn default() -> Self {
        Self {
            weights: vec![1.0, 0.8, 0.6, 1.2, 0.8, 1.0, 0.3, 0.3, -0.8, 3.0, 0.3, -0.2],
            bias: 0.0,
        }
    }
}

impl LinearReranker {
    pub fn score(&self, features: &[f32; FEATURES]) -> f32 {
        self.bias
            + self
                .weights
                .iter()
                .zip(features.iter())
                .map(|(w, f)| w * f)
                .sum::<f32>()
    }
}

#[async_trait]
impl Reranker for LinearReranker {
    async fn rerank(
        &self,
        question: &Question,
        index: &IndexSnapshot,
        mut candidates: Vec<Candidate>,
    ) -> Result<Vec<Candidate>> {
        let matrix = feature_matrix(question, index, &candidates);
        for (candidate, features) in candidates.iter_mut().zip(matrix) {
            candidate.score = crate::gate::score::sigmoid(self.score(&features));
        }
        sort_candidates(&mut candidates);
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::{Corpus, Unit};
    use crate::index::vectors::HashingEmbedder;

    #[tokio::test]
    async fn names_matching_the_question_beat_tests_with_equal_retrieval_scores() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        corpus.push_unit(Unit {
            name: "test_detach".into(),
            file: "tests/test_volume.py".into(),
            kind: Some(UnitKind::Function),
            ..Unit::default()
        });
        corpus.push_unit(Unit {
            name: "detach_volume".into(),
            file: "volume/manager.py".into(),
            kind: Some(UnitKind::Method),
            ..Unit::default()
        });
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let question = Question::analyze("What happens when a volume is detached? detach_volume");
        let candidates = vec![
            Candidate {
                unit: 0,
                rrf: 0.02,
                lexical: 3.0,
                ..Candidate::default()
            },
            Candidate {
                unit: 1,
                rrf: 0.02,
                lexical: 3.0,
                ..Candidate::default()
            },
        ];
        let ranked = LinearReranker::default()
            .rerank(&question, &index, candidates)
            .await
            .unwrap();
        assert_eq!(ranked[0].unit, 1);
        let matrix = feature_matrix(&question, &index, &ranked);
        assert_eq!(matrix[1][8], 1.0);
        assert!(matrix[0][5] > 0.0);
    }

    #[test]
    fn default_weights_cover_every_feature() {
        assert_eq!(LinearReranker::default().weights.len(), FEATURES);
    }
}
