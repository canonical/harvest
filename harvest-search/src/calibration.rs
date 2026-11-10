use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::context::SearchContext;
use crate::domain::{Query, UnitId};
use crate::gate::score::{sigmoid, CalibratedScore};
use crate::gate::signals::Signals;
use crate::gate::{SufficiencyCheck, Verdict};
use crate::index::corpus::{Relation, Unit, UnitKind};
use crate::index::graph::Direction;
use crate::index::IndexSnapshot;
use crate::question::Question;
use crate::retrieve::rerank::{feature_matrix, LinearReranker, Reranker};
use crate::retrieve::Candidate;
use crate::tiers::rerank_and_pack;
use crate::tiers::rounds::Rounds;
use crate::Deadline;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Calibration {
    pub reranker: LinearReranker,
    pub gate: CalibratedScore,
}

pub fn defaults() -> Calibration {
    static DEFAULTS: std::sync::OnceLock<Calibration> = std::sync::OnceLock::new();
    DEFAULTS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../defaults/calibration.json")).unwrap_or_else(
                |_| Calibration {
                    reranker: LinearReranker::default(),
                    gate: CalibratedScore::default(),
                },
            )
        })
        .clone()
}

pub struct ScopedReranker {
    fallback: Arc<dyn Reranker>,
}

impl ScopedReranker {
    pub fn new(fallback: Arc<dyn Reranker>) -> Arc<Self> {
        Arc::new(Self { fallback })
    }
}

#[async_trait]
impl Reranker for ScopedReranker {
    async fn rerank(
        &self,
        question: &Question,
        index: &IndexSnapshot,
        candidates: Vec<Candidate>,
    ) -> Result<Vec<Candidate>> {
        match &index.calibration {
            Some(calibration) => {
                calibration
                    .reranker
                    .rerank(question, index, candidates)
                    .await
            }
            None => self.fallback.rerank(question, index, candidates).await,
        }
    }
}

pub struct ScopedScore {
    fallback: CalibratedScore,
}

impl ScopedScore {
    pub fn new(fallback: CalibratedScore) -> Self {
        Self { fallback }
    }
}

#[async_trait]
impl SufficiencyCheck for ScopedScore {
    fn name(&self) -> &'static str {
        "calibrated_score"
    }

    async fn check(&self, ctx: &SearchContext) -> Verdict {
        match &ctx.index.calibration {
            Some(calibration) => calibration.gate.check(ctx).await,
            None => self.fallback.check(ctx).await,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Example {
    pub kind: String,
    pub question: String,
    pub essential: Vec<UnitId>,
    pub related: Vec<UnitId>,
}

pub fn first_sentence(docstring: &str) -> Option<String> {
    let text = docstring.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = text
        .find(". ")
        .map(|i| i + 1)
        .or_else(|| text.ends_with('.').then_some(text.len()))
        .unwrap_or(text.len());
    let sentence = text[..end].trim().trim_end_matches('.').trim();
    let words = sentence.split_whitespace().count();
    (4..=40).contains(&words).then(|| sentence.to_string())
}

fn spread(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn eligible(unit: &Unit) -> bool {
    unit.kind().is_symbol()
        && !unit.is_test()
        && !unit.is_vendored()
        && !unit.is_generated()
        && unit.name.chars().filter(|c| c.is_alphanumeric()).count() >= 3
}

fn neighbors(
    index: &IndexSnapshot,
    unit: UnitId,
    direction: Direction,
    relations: &[Relation],
) -> Vec<UnitId> {
    let mut found: Vec<UnitId> = index
        .graph
        .neighbors(unit, direction, relations)
        .into_iter()
        .filter(|n| *n != unit && eligible(index.unit(*n)))
        .collect();
    found.sort_unstable();
    found.dedup();
    found
}

pub fn synthesize(index: &IndexSnapshot, per_kind: usize) -> Vec<Example> {
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for unit in index.units.iter().filter(|u| u.kind().is_symbol()) {
        *by_name
            .entry(unit.qualified_name.to_lowercase())
            .or_default() += 1;
    }
    let mut pool: Vec<&Unit> = index
        .units
        .iter()
        .filter(|u| eligible(u) && by_name.get(&u.qualified_name.to_lowercase()) == Some(&1))
        .collect();
    pool.sort_by_key(|u| spread(&format!("{}:{}", u.file, u.qualified_name)));
    let mut sentences: HashMap<String, usize> = HashMap::new();
    for unit in &pool {
        if let Some(sentence) = first_sentence(&unit.docstring) {
            *sentences.entry(sentence.to_lowercase()).or_default() += 1;
        }
    }

    let mut examples = Vec::new();
    let take = |kind: &str, examples: &mut Vec<Example>, made: Example| {
        if examples.iter().filter(|e| e.kind == kind).count() < per_kind {
            examples.push(made);
        }
    };
    for unit in &pool {
        let what = if unit.kind() == UnitKind::Class {
            "class"
        } else {
            "function"
        };
        take(
            "lookup",
            &mut examples,
            Example {
                kind: "lookup".into(),
                question: format!(
                    "Where is the {what} {} defined and what does it do?",
                    unit.qualified_name
                ),
                essential: vec![unit.id],
                related: Vec::new(),
            },
        );
        if let Some(sentence) = first_sentence(&unit.docstring) {
            if sentences.get(&sentence.to_lowercase()) == Some(&1) {
                take(
                    "describe",
                    &mut examples,
                    Example {
                        kind: "describe".into(),
                        question: format!("Find the code that does this: {sentence}"),
                        essential: vec![unit.id],
                        related: Vec::new(),
                    },
                );
            }
        }
        if unit.kind() == UnitKind::Class {
            let children = neighbors(
                index,
                unit.id,
                Direction::In,
                &[Relation::Inherits, Relation::Implements],
            );
            if (2..=12).contains(&children.len()) {
                take(
                    "list",
                    &mut examples,
                    Example {
                        kind: "list".into(),
                        question: format!("Which classes extend {}?", unit.qualified_name),
                        essential: children,
                        related: vec![unit.id],
                    },
                );
            }
            continue;
        }
        let callers = neighbors(index, unit.id, Direction::In, &[Relation::Calls]);
        if (2..=8).contains(&callers.len()) {
            take(
                "callers",
                &mut examples,
                Example {
                    kind: "callers".into(),
                    question: format!("What calls {}?", unit.qualified_name),
                    essential: callers,
                    related: vec![unit.id],
                },
            );
        }
        let callees = neighbors(index, unit.id, Direction::Out, &[Relation::Calls]);
        if (1..=10).contains(&callees.len()) {
            take(
                "flow",
                &mut examples,
                Example {
                    kind: "flow".into(),
                    question: format!("What happens when {} is called?", unit.qualified_name),
                    essential: vec![unit.id],
                    related: callees,
                },
            );
        }
    }
    examples
}

const EPOCHS: usize = 400;
const LEARNING_RATE: f32 = 0.1;
const L2: f32 = 0.001;
const MISSED_TARGET: f64 = 5.0;
const STRUCTURAL_COVERAGE: f32 = 0.9;
pub const SUFFICIENT_RECALL: f64 = 0.9;
const MIN_EXAMPLES: usize = 20;
const MAX_REGRESSION: f64 = 0.01;

pub fn logistic(rows: &[Vec<f32>], labels: &[bool], balance: bool) -> (Vec<f32>, f32) {
    let width = rows.first().map_or(0, Vec::len);
    let mut weights = vec![0f32; width];
    let mut bias = 0f32;
    if rows.is_empty() {
        return (weights, bias);
    }
    let positives = labels.iter().filter(|l| **l).count().max(1) as f32;
    let negatives = labels.iter().filter(|l| !**l).count().max(1) as f32;
    let (pos_weight, neg_weight) = if balance {
        let total = positives + negatives;
        (total / (2.0 * positives), total / (2.0 * negatives))
    } else {
        (1.0, 1.0)
    };
    let n = rows.len() as f32;
    for _ in 0..EPOCHS {
        let mut grad = vec![0f32; width];
        let mut grad_bias = 0f32;
        for (row, label) in rows.iter().zip(labels) {
            let z = bias + row.iter().zip(&weights).map(|(x, w)| x * w).sum::<f32>();
            let target = if *label { 1.0 } else { 0.0 };
            let weight = if *label { pos_weight } else { neg_weight };
            let error = (sigmoid(z) - target) * weight;
            for (g, x) in grad.iter_mut().zip(row) {
                *g += error * x;
            }
            grad_bias += error;
        }
        for (w, g) in weights.iter_mut().zip(&grad) {
            *w -= LEARNING_RATE * (g / n + L2 * *w);
        }
        bias -= LEARNING_RATE * grad_bias / n;
    }
    (weights, bias)
}

pub fn train_reranker(rows: &[Vec<f32>], labels: &[bool]) -> LinearReranker {
    let (weights, bias) = logistic(rows, labels, true);
    LinearReranker { weights, bias }
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

pub fn calibrate_gate(rows: &[Vec<f32>], sufficient: &[bool]) -> Option<CalibratedScore> {
    if !sufficient.iter().any(|l| *l) || !sufficient.iter().any(|l| !*l) {
        return None;
    }
    let (weights, bias) = logistic(rows, sufficient, true);
    let probability = |row: &Vec<f32>| {
        f64::from(sigmoid(
            bias + row.iter().zip(&weights).map(|(x, w)| x * w).sum::<f32>(),
        ))
    };
    let insufficient: Vec<f64> = rows
        .iter()
        .zip(sufficient)
        .filter(|(_, l)| !**l)
        .map(|(r, _)| probability(r))
        .collect();
    let enough: Vec<f64> = rows
        .iter()
        .zip(sufficient)
        .filter(|(_, l)| **l)
        .map(|(r, _)| probability(r))
        .collect();
    let accept = (percentile(&insufficient, 100.0 - MISSED_TARGET) + 1e-4).min(0.999);
    let reject = (percentile(&enough, MISSED_TARGET) - 1e-4)
        .max(0.0)
        .min(accept - 0.01);
    Some(CalibratedScore {
        weights,
        bias,
        accept: accept as f32,
        reject: reject.max(0.0) as f32,
    })
}

pub fn gate_row(signals: &Signals) -> Option<Vec<f32>> {
    let undecided = signals.dangling == 0
        && signals
            .list_coverage
            .is_none_or(|c| c >= STRUCTURAL_COVERAGE);
    undecided.then(|| signals.features().to_vec())
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub examples: usize,
    pub train: usize,
    pub holdout: usize,
    pub base_recall: f64,
    pub trained_recall: f64,
    pub kept_trained_reranker: bool,
    pub gate_calibrated: bool,
}

fn query(index: &IndexSnapshot, example: &Example) -> Query {
    Query::new(example.question.clone(), index.scope.clone())
}

pub struct Calibrator<F>
where
    F: Fn(Arc<dyn Reranker>) -> Rounds,
{
    rounds: F,
    per_kind: usize,
}

impl<F> Calibrator<F>
where
    F: Fn(Arc<dyn Reranker>) -> Rounds,
{
    pub fn new(rounds: F) -> Self {
        Self {
            rounds,
            per_kind: 60,
        }
    }

    pub fn with_per_kind(mut self, per_kind: usize) -> Self {
        self.per_kind = per_kind.max(1);
        self
    }

    async fn context(&self, index: &Arc<IndexSnapshot>, example: &Example) -> SearchContext {
        SearchContext::new(
            &query(index, example),
            Arc::clone(index),
            Deadline::after(Duration::from_secs(30)),
        )
    }

    async fn recall(
        &self,
        index: &Arc<IndexSnapshot>,
        reranker: Arc<dyn Reranker>,
        examples: &[&Example],
    ) -> (f64, Vec<(Signals, f64)>) {
        let rounds = (self.rounds)(Arc::clone(&reranker));
        let mut total = 0.0;
        let mut outcomes = Vec::new();
        for example in examples {
            let mut ctx = self.context(index, example).await;
            rounds.gather(&mut ctx).await;
            if rerank_and_pack(&mut ctx, reranker.as_ref()).await.is_err() {
                continue;
            }
            let found = example
                .essential
                .iter()
                .filter(|u| ctx.has_unit(**u))
                .count();
            let recall = found as f64 / example.essential.len().max(1) as f64;
            total += recall;
            outcomes.push((Signals::compute(&ctx), recall));
        }
        (total / examples.len().max(1) as f64, outcomes)
    }

    pub async fn calibrate(
        &self,
        index: Arc<IndexSnapshot>,
        base: LinearReranker,
    ) -> Option<(Calibration, Report)> {
        let examples = synthesize(&index, self.per_kind);
        if examples.len() < MIN_EXAMPLES {
            return None;
        }
        let (train, holdout): (Vec<&Example>, Vec<&Example>) =
            examples.iter().partition(|e| spread(&e.question) % 10 < 7);

        let gather = (self.rounds)(Arc::new(base.clone()));
        let mut rows = Vec::new();
        let mut labels = Vec::new();
        for example in &train {
            let mut ctx = self.context(&index, example).await;
            gather.gather(&mut ctx).await;
            let wanted: HashSet<UnitId> = example
                .essential
                .iter()
                .chain(&example.related)
                .copied()
                .collect();
            let matrix = feature_matrix(&ctx.question, &ctx.index, &ctx.candidates);
            for (candidate, features) in ctx.candidates.iter().zip(matrix) {
                rows.push(features.to_vec());
                labels.push(wanted.contains(&candidate.unit));
            }
        }
        if !labels.iter().any(|l| *l) {
            return None;
        }
        let trained = train_reranker(&rows, &labels);
        let (base_recall, _) = self.recall(&index, Arc::new(base.clone()), &holdout).await;
        let (trained_recall, _) = self
            .recall(&index, Arc::new(trained.clone()), &holdout)
            .await;
        let keep = trained_recall + MAX_REGRESSION >= base_recall;
        let reranker = if keep { trained } else { base };

        let (_, outcomes) = self
            .recall(&index, Arc::new(reranker.clone()), &train)
            .await;
        let mut gate_rows = Vec::new();
        let mut sufficient = Vec::new();
        for (signals, recall) in outcomes {
            if let Some(row) = gate_row(&signals) {
                gate_rows.push(row);
                sufficient.push(recall >= SUFFICIENT_RECALL);
            }
        }
        let gate = calibrate_gate(&gate_rows, &sufficient);
        let report = Report {
            examples: examples.len(),
            train: train.len(),
            holdout: holdout.len(),
            base_recall,
            trained_recall,
            kept_trained_reranker: keep,
            gate_calibrated: gate.is_some(),
        };
        Some((
            Calibration {
                reranker,
                gate: gate.unwrap_or_else(|| defaults().gate),
            },
            report,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Scope;
    use crate::index::corpus::Corpus;
    use crate::index::vectors::HashingEmbedder;

    fn unit(name: &str, parent: Option<&str>, kind: UnitKind, doc: &str) -> Unit {
        Unit {
            name: name.into(),
            qualified_name: match parent {
                Some(p) => format!("{p}.{name}"),
                None => name.into(),
            },
            parent: parent.map(str::to_string),
            kind: Some(kind),
            file: format!("src/{}.rs", name.to_lowercase()),
            docstring: doc.into(),
            body: format!("fn {name}() {{}}"),
            start_line: 1,
            end_line: 3,
            ..Unit::default()
        }
    }

    #[tokio::test]
    async fn examples_come_from_names_docstrings_callers_and_hierarchies() {
        let mut corpus = Corpus::new(Scope::new("r", "v"), "g");
        let base = corpus.push_unit(unit("Driver", None, UnitKind::Class, ""));
        let a = corpus.push_unit(unit("LvmDriver", None, UnitKind::Class, ""));
        let b = corpus.push_unit(unit("NfsDriver", None, UnitKind::Class, ""));
        corpus.link(a, base, Relation::Inherits);
        corpus.link(b, base, Relation::Inherits);
        let target = corpus.push_unit(unit(
            "attach_volume",
            None,
            UnitKind::Function,
            "Attach a volume to a running instance.",
        ));
        let c1 = corpus.push_unit(unit("api_attach", None, UnitKind::Function, ""));
        let c2 = corpus.push_unit(unit("rpc_attach", None, UnitKind::Function, ""));
        corpus.link(c1, target, Relation::Calls);
        corpus.link(c2, target, Relation::Calls);
        let mut test = unit("test_attach", None, UnitKind::Function, "");
        test.test = true;
        let t = corpus.push_unit(test);
        corpus.link(t, target, Relation::Calls);
        let index = IndexSnapshot::build(corpus, &HashingEmbedder::default())
            .await
            .unwrap();
        let examples = synthesize(&index, 10);
        let kinds: HashSet<&str> = examples.iter().map(|e| e.kind.as_str()).collect();
        for kind in ["lookup", "describe", "list", "callers", "flow"] {
            assert!(kinds.contains(kind), "missing {kind}: {examples:#?}");
        }
        let callers = examples.iter().find(|e| e.kind == "callers").unwrap();
        assert_eq!(
            callers.essential,
            vec![c1, c2],
            "tests are not expected callers"
        );
        assert!(!examples.iter().any(|e| e.question.contains("test_attach")));
    }

    #[test]
    fn logistic_regression_separates_a_simple_rule() {
        let rows: Vec<Vec<f32>> = (0..200)
            .map(|i| vec![(i % 10) as f32 / 10.0, ((i * 7) % 3) as f32])
            .collect();
        let labels: Vec<bool> = rows.iter().map(|r| r[0] > 0.45).collect();
        let (weights, _) = logistic(&rows, &labels, false);
        assert!(weights[0] > 1.0, "{weights:?}");
    }

    #[test]
    fn defaults_parse() {
        let d = defaults();
        assert!(!d.reranker.weights.is_empty());
        assert!(d.gate.accept > d.gate.reject);
    }
}
