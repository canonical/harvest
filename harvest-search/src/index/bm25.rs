use std::collections::HashMap;

use regex::Regex;
use std::sync::LazyLock;

use crate::domain::UnitId;
use crate::index::corpus::Unit;
use crate::index::tokenizer::tokens;

const K1: f32 = 1.2;
const B: f32 = 0.75;
const MAX_BODY_CHARS: usize = 20_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Name,
    Signature,
    Docstring,
    Body,
    Strings,
    Path,
    Summary,
}

impl Field {
    const ALL: [Field; 7] = [
        Self::Name,
        Self::Signature,
        Self::Docstring,
        Self::Body,
        Self::Strings,
        Self::Path,
        Self::Summary,
    ];

    fn weight(self) -> f32 {
        match self {
            Self::Name => 3.0,
            Self::Signature => 1.5,
            Self::Docstring => 1.2,
            Self::Body => 1.0,
            Self::Strings => 1.3,
            Self::Path => 0.8,
            Self::Summary => 1.2,
        }
    }

    fn text(self, unit: &Unit) -> String {
        match self {
            Self::Name => format!(
                "{} {} {}",
                unit.name,
                unit.qualified_name,
                unit.parent.as_deref().unwrap_or("")
            ),
            Self::Signature => format!("{} {}", unit.signature, unit.decorators.join(" ")),
            Self::Docstring => unit.docstring.clone(),
            Self::Body => truncate(&unit.body, MAX_BODY_CHARS).to_string(),
            Self::Strings => string_literals(&unit.body).join(" "),
            Self::Path => unit.file.clone(),
            Self::Summary => unit.summary.clone(),
        }
    }
}

fn truncate(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

static STRING_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""((?:[^"\\\n]|\\.){2,120})"|'((?:[^'\\\n]|\\.){2,120})'"#).unwrap()
});

pub fn string_literals(source: &str) -> Vec<String> {
    STRING_LITERAL
        .captures_iter(truncate(source, MAX_BODY_CHARS))
        .filter_map(|c| c.get(1).or_else(|| c.get(2)))
        .map(|m| m.as_str().to_string())
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct Bm25Index {
    postings: HashMap<String, Vec<(UnitId, f32)>>,
    units: usize,
}

impl Bm25Index {
    pub fn build(units: &[Unit]) -> Self {
        let mut lengths: HashMap<Field, Vec<f32>> = HashMap::new();
        let mut field_tokens: Vec<Vec<(Field, Vec<String>)>> = Vec::with_capacity(units.len());
        for unit in units {
            let mut per_field = Vec::with_capacity(Field::ALL.len());
            for field in Field::ALL {
                let toks = tokens(&field.text(unit));
                lengths.entry(field).or_default().push(toks.len() as f32);
                per_field.push((field, toks));
            }
            field_tokens.push(per_field);
        }
        let averages: HashMap<Field, f32> = lengths
            .iter()
            .map(|(f, ls)| {
                let total: f32 = ls.iter().sum();
                (*f, (total / ls.len().max(1) as f32).max(1.0))
            })
            .collect();

        let mut postings: HashMap<String, Vec<(UnitId, f32)>> = HashMap::new();
        for (unit, per_field) in units.iter().zip(field_tokens) {
            let mut weighted: HashMap<String, f32> = HashMap::new();
            for (field, toks) in per_field {
                let length = toks.len() as f32;
                let norm = 1.0 - B + B * length / averages[&field];
                let mut counts: HashMap<String, f32> = HashMap::new();
                for t in toks {
                    *counts.entry(t).or_default() += 1.0;
                }
                for (t, tf) in counts {
                    *weighted.entry(t).or_default() += field.weight() * tf / norm;
                }
            }
            for (t, tf) in weighted {
                postings.entry(t).or_default().push((unit.id, tf));
            }
        }
        Self {
            postings,
            units: units.len(),
        }
    }

    pub fn idf(&self, term: &str) -> f32 {
        let df = self.postings.get(term).map_or(0, Vec::len) as f32;
        let n = self.units as f32;
        (1.0 + (n - df + 0.5) / (df + 0.5)).ln()
    }

    pub fn search(&self, terms: &[(String, f32)], k: usize) -> Vec<(UnitId, f32)> {
        let mut scores: HashMap<UnitId, f32> = HashMap::new();
        for (term, weight) in terms {
            let Some(list) = self.postings.get(term) else {
                continue;
            };
            let idf = self.idf(term) * weight;
            for (unit, tf) in list {
                *scores.entry(*unit).or_default() += idf * tf * (K1 + 1.0) / (tf + K1);
            }
        }
        top_k(scores.into_iter().collect(), k)
    }

    pub fn len(&self) -> usize {
        self.units
    }

    pub fn is_empty(&self) -> bool {
        self.units == 0
    }
}

pub fn top_k(mut scored: Vec<(UnitId, f32)>, k: usize) -> Vec<(UnitId, f32)> {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(id: UnitId, name: &str, body: &str) -> Unit {
        Unit {
            id,
            name: name.into(),
            qualified_name: name.into(),
            file: format!("src/{name}.py"),
            body: body.into(),
            ..Unit::default()
        }
    }

    fn q(words: &[&str]) -> Vec<(String, f32)> {
        words.iter().map(|w| (w.to_string(), 1.0)).collect()
    }

    #[test]
    fn names_outrank_bodies_and_rare_terms_outrank_common_ones() {
        let units = vec![
            unit(0, "RetryPolicy", "def run(self): pass"),
            unit(1, "helper", "policy = RetryPolicy() # retry"),
            unit(2, "other", "def run(self): pass"),
        ];
        let index = Bm25Index::build(&units);
        let hits = index.search(&q(&["retry", "policy"]), 10);
        assert_eq!(hits[0].0, 0);
        assert_eq!(hits.len(), 2);
        assert!(index.idf("run") < index.idf("helper"));
    }

    #[test]
    fn identifiers_used_only_in_bodies_are_found() {
        let units = vec![
            unit(0, "configure", "limit = CONF.retry_limit"),
            unit(1, "noise", "x = 1"),
        ];
        let hits = Bm25Index::build(&units).search(&q(&["retry_limit"]), 5);
        assert_eq!(hits, vec![(0, hits[0].1)]);
    }

    #[test]
    fn string_literals_are_indexed_separately() {
        assert_eq!(
            string_literals(r#"raise ValueError("volume is busy") ; x = 'abc'"#),
            vec!["volume is busy", "abc"]
        );
        let units = vec![
            unit(0, "detach", r#"raise ValueError("volume is busy")"#),
            unit(1, "noise", "busy = True"),
        ];
        let hits = Bm25Index::build(&units).search(&q(&["volume", "busy"]), 5);
        assert_eq!(hits[0].0, 0);
    }
}
