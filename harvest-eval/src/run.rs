use std::collections::BTreeMap;
use std::time::Duration;

use harvest_search::{Deadline, Query, Scope, SearchEngine, SearchResult, TierId, TokenBudget};

use crate::gold::GoldQuery;
use crate::judge::{Answerer, Judge};
use crate::metrics::quality::{ndcg, pool_recall, precision, recall};
use crate::report::QueryRecord;

const NDCG_AT: usize = 10;

pub struct Run<'a> {
    pub engine: &'a dyn SearchEngine,
    pub tier1: Option<&'a dyn SearchEngine>,
    pub budget_ms: u64,
    pub token_budget: usize,
    pub answerer: Option<&'a Answerer>,
    pub judge: Option<&'a Judge>,
    pub references: BTreeMap<String, String>,
}

pub fn query_for(gold: &GoldQuery, token_budget: usize) -> Query {
    let mut query = Query::new(&gold.question, Scope::new(&gold.repo, &gold.version));
    query.budget = TokenBudget(token_budget);
    query
}

fn tier1_said_sufficient(result: &SearchResult) -> Option<bool> {
    let tier1: Vec<_> = result
        .trace
        .gate
        .iter()
        .filter(|g| g.tier == TierId::Rounds.as_str())
        .collect();
    if tier1.is_empty() {
        return None;
    }
    Some(tier1.iter().any(|g| g.verdict == "sufficient"))
}

pub fn record_for(
    gold: &GoldQuery,
    result: &SearchResult,
    tier1: Option<&SearchResult>,
) -> QueryRecord {
    let measured = recall(gold, &result.evidence);
    let mut stages: BTreeMap<String, f64> = BTreeMap::new();
    for stage in &result.trace.stages {
        *stages.entry(stage.stage.clone()).or_default() += stage.ms;
    }
    QueryRecord {
        id: gold.id.clone(),
        kind: gold.kind.clone(),
        question: gold.question.clone(),
        tier: result.tier_reached.as_str().to_string(),
        degraded: result.degraded.map(|d| d.as_str().to_string()),
        recall_essential: measured.essential,
        recall_all: measured.all,
        pool_recall: pool_recall(gold, &result.trace.pool),
        ndcg: ndcg(gold, &result.evidence, NDCG_AT),
        precision: precision(gold, &result.evidence),
        e2e_ms: result.trace.total_ms,
        stages,
        llm_calls: result.trace.llm_calls,
        evidence: result.evidence.len(),
        tier1_said_sufficient: tier1_said_sufficient(result),
        tier1_recall: tier1.map(|t| recall(gold, &t.evidence).essential),
        signals: tier1
            .and_then(|t| t.trace.signals.clone())
            .or_else(|| result.trace.signals.clone()),
        answer: result.answer.clone(),
        judge: None,
        error: None,
    }
}

pub async fn run(gold: &[GoldQuery], run: &Run<'_>) -> Vec<QueryRecord> {
    let mut records = Vec::with_capacity(gold.len());
    for (i, item) in gold.iter().enumerate() {
        let query = query_for(item, run.token_budget);
        let deadline = Deadline::after(Duration::from_millis(run.budget_ms));
        let outcome = run.engine.search(&query, deadline).await;
        let mut record = match &outcome {
            Ok(result) => {
                let tier1 = match run.tier1 {
                    Some(engine) => engine
                        .search(
                            &query,
                            Deadline::after(Duration::from_millis(run.budget_ms)),
                        )
                        .await
                        .ok(),
                    None => None,
                };
                record_for(item, result, tier1.as_ref())
            }
            Err(e) => QueryRecord {
                id: item.id.clone(),
                kind: item.kind.clone(),
                question: item.question.clone(),
                error: Some(e.to_string()),
                ..QueryRecord::default()
            },
        };
        if let (Ok(result), Some(answerer)) = (&outcome, run.answerer) {
            if record.answer.is_none() {
                match answerer.answer(&item.question, &result.evidence).await {
                    Ok(answer) => record.answer = Some(answer),
                    Err(e) => tracing::warn!(error = %e, id = item.id, "answer generation failed"),
                }
            }
        }
        let reference = run
            .references
            .get(&item.id)
            .cloned()
            .or_else(|| item.reference_answer.clone());
        if let (Some(judge), Some(answer), Some(reference)) = (run.judge, &record.answer, reference)
        {
            match judge.compare(&item.question, answer, &reference).await {
                Ok(verdict) => record.judge = Some(verdict.as_str().to_string()),
                Err(e) => tracing::warn!(error = %e, id = item.id, "judging failed"),
            }
        }
        tracing::info!(
            n = i + 1,
            of = gold.len(),
            id = item.id,
            tier = record.tier,
            recall = record.recall_essential,
            ms = record.e2e_ms as u64,
            "evaluated"
        );
        records.push(record);
    }
    records
}
