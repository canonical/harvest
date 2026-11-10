pub mod expand;
pub mod extract;
pub mod fuse;
pub mod pack;
pub mod rerank;

use serde::{Deserialize, Serialize};

use crate::domain::{Source, UnitId};
use crate::index::tokenizer::{identifiers, query_tokens, tokens};
use crate::index::IndexSnapshot;

const EXPANSION_WEIGHT: f32 = 0.4;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Terms {
    text: String,
    weighted: Vec<(String, f32)>,
    identifiers: Vec<String>,
    expansions: Vec<String>,
}

impl Terms {
    pub fn from_question(text: &str) -> Self {
        Self {
            text: text.to_string(),
            weighted: query_tokens(text).into_iter().map(|t| (t, 1.0)).collect(),
            identifiers: identifiers(text),
            expansions: Vec::new(),
        }
    }

    pub fn weighted(&self) -> &[(String, f32)] {
        &self.weighted
    }

    pub fn identifiers(&self) -> &[String] {
        &self.identifiers
    }

    pub fn expansions(&self) -> &[String] {
        &self.expansions
    }

    pub fn extend(&mut self, names: &[String]) -> usize {
        let mut added = 0;
        for name in names {
            if self.expansions.contains(name) || self.identifiers.contains(name) {
                continue;
            }
            self.expansions.push(name.clone());
            added += 1;
            for token in tokens(name) {
                if !self.weighted.iter().any(|(t, _)| *t == token) {
                    self.weighted.push((token, EXPANSION_WEIGHT));
                }
            }
        }
        added
    }

    pub fn embedding_text(&self) -> String {
        if self.expansions.is_empty() {
            return self.text.clone();
        }
        format!("{} {}", self.text, self.expansions.join(" "))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub unit: UnitId,
    pub score: f32,
    pub source: Source,
}

pub struct RetrievalInput<'a> {
    pub terms: &'a Terms,
    pub vector: Option<&'a [f32]>,
}

pub trait Retriever: Send + Sync {
    fn source(&self) -> Source;
    fn retrieve(&self, index: &IndexSnapshot, input: &RetrievalInput<'_>, k: usize) -> Vec<Hit>;
}

fn hits(scored: Vec<(UnitId, f32)>, source: Source) -> Vec<Hit> {
    scored
        .into_iter()
        .map(|(unit, score)| Hit {
            unit,
            score,
            source,
        })
        .collect()
}

pub struct Bm25Retriever;

impl Retriever for Bm25Retriever {
    fn source(&self) -> Source {
        Source::Lexical
    }

    fn retrieve(&self, index: &IndexSnapshot, input: &RetrievalInput<'_>, k: usize) -> Vec<Hit> {
        hits(
            index.bm25.search(input.terms.weighted(), k),
            Source::Lexical,
        )
    }
}

pub struct DenseRetriever;

impl Retriever for DenseRetriever {
    fn source(&self) -> Source {
        Source::Dense
    }

    fn retrieve(&self, index: &IndexSnapshot, input: &RetrievalInput<'_>, k: usize) -> Vec<Hit> {
        match input.vector {
            Some(vector) => hits(index.vectors.search(vector, k), Source::Dense),
            None => Vec::new(),
        }
    }
}

pub struct ExactRetriever;

impl Retriever for ExactRetriever {
    fn source(&self) -> Source {
        Source::Exact
    }

    fn retrieve(&self, index: &IndexSnapshot, input: &RetrievalInput<'_>, k: usize) -> Vec<Hit> {
        let mut found: Vec<Hit> = Vec::new();
        let names = input
            .terms
            .identifiers()
            .iter()
            .map(|n| (n, 1.0))
            .chain(input.terms.expansions().iter().map(|n| (n, 0.5)));
        for (name, score) in names {
            for unit in index.find_by_name(name) {
                if !found.iter().any(|h| h.unit == *unit) {
                    found.push(Hit {
                        unit: *unit,
                        score,
                        source: Source::Exact,
                    });
                }
            }
        }
        found.truncate(k);
        found
    }
}

pub struct CacheRetriever {
    pub min_overlap: f32,
}

impl Default for CacheRetriever {
    fn default() -> Self {
        Self { min_overlap: 0.7 }
    }
}

impl Retriever for CacheRetriever {
    fn source(&self) -> Source {
        Source::Cache
    }

    fn retrieve(&self, index: &IndexSnapshot, input: &RetrievalInput<'_>, k: usize) -> Vec<Hit> {
        let Some(cache) = &index.cache else {
            return Vec::new();
        };
        let asked: Vec<&String> = input
            .terms
            .weighted()
            .iter()
            .filter(|(_, w)| *w >= 1.0)
            .map(|(t, _)| t)
            .collect();
        if asked.is_empty() {
            return Vec::new();
        }
        let query: Vec<(String, f32)> = asked.iter().map(|t| ((*t).clone(), 1.0)).collect();
        let Some((best, _)) = cache.bm25.search(&query, 1).into_iter().next() else {
            return Vec::new();
        };
        let cached: std::collections::HashSet<String> =
            query_tokens(&cache.questions[best as usize])
                .into_iter()
                .collect();
        let overlap =
            asked.iter().filter(|t| cached.contains(**t)).count() as f32 / asked.len() as f32;
        if overlap < self.min_overlap {
            return Vec::new();
        }
        cache.units[best as usize]
            .iter()
            .take(k)
            .map(|unit| Hit {
                unit: *unit,
                score: overlap,
                source: Source::Cache,
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub unit: UnitId,
    pub score: f32,
    pub rrf: f32,
    pub lexical: f32,
    pub dense: f32,
    pub exact: bool,
    pub graph: f32,
    pub pinned: bool,
    pub sources: Vec<Source>,
}

impl Candidate {
    pub fn pinned(unit: UnitId, source: Source) -> Self {
        Self {
            unit,
            pinned: true,
            sources: vec![source],
            ..Self::default()
        }
    }
}

pub fn sort_candidates(candidates: &mut [Candidate]) {
    candidates.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.score.total_cmp(&a.score))
            .then(a.unit.cmp(&b.unit))
    });
}

pub fn merge_pinned(candidates: &mut Vec<Candidate>, pinned: Vec<Candidate>) {
    for pin in pinned {
        match candidates.iter_mut().find(|c| c.unit == pin.unit) {
            Some(existing) => {
                existing.pinned = true;
                for source in pin.sources {
                    if !existing.sources.contains(&source) {
                        existing.sources.push(source);
                    }
                }
            }
            None => candidates.push(pin),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expansions_add_lower_weight_tokens_once() {
        let mut terms = Terms::from_question("how is a volume attached");
        assert_eq!(
            terms.extend(&["VolumeAttachment".into(), "VolumeAttachment".into()]),
            1
        );
        assert!(terms
            .weighted()
            .iter()
            .any(|(t, w)| t == "attachment" && (*w - EXPANSION_WEIGHT).abs() < 1e-6));
        assert!(terms
            .weighted()
            .iter()
            .any(|(t, w)| t == "volume" && (*w - 1.0).abs() < 1e-6));
        assert!(terms.embedding_text().ends_with("VolumeAttachment"));
    }

    #[tokio::test]
    async fn cached_evidence_answers_close_paraphrases_only() {
        use crate::domain::Scope;
        use crate::index::corpus::{CachedEvidence, Corpus, Unit};
        use crate::index::vectors::HashingEmbedder;
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        for name in ["a", "b", "c"] {
            corpus.push_unit(Unit {
                name: name.into(),
                ..Unit::default()
            });
        }
        corpus.cached.push(CachedEvidence {
            question: "Which volume drivers support multiattach?".into(),
            units: vec![1, 2],
        });
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let retrieve = |text: &str| {
            let terms = Terms::from_question(text);
            CacheRetriever::default()
                .retrieve(
                    &index,
                    &RetrievalInput {
                        terms: &terms,
                        vector: None,
                    },
                    10,
                )
                .iter()
                .map(|h| h.unit)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            retrieve("which drivers support multiattach for volumes"),
            vec![1, 2]
        );
        assert!(retrieve("which drivers support uploads").is_empty());
    }

    #[test]
    fn pinned_candidates_sort_first_and_merge_into_existing_ones() {
        let mut list = vec![
            Candidate {
                unit: 1,
                score: 5.0,
                ..Candidate::default()
            },
            Candidate {
                unit: 2,
                score: 1.0,
                ..Candidate::default()
            },
        ];
        merge_pinned(
            &mut list,
            vec![
                Candidate::pinned(2, Source::Plan),
                Candidate::pinned(3, Source::Plan),
            ],
        );
        sort_candidates(&mut list);
        assert_eq!(
            list.iter().map(|c| c.unit).collect::<Vec<_>>(),
            vec![2, 3, 1]
        );
        assert!(list[0].sources.contains(&Source::Plan));
    }
}
