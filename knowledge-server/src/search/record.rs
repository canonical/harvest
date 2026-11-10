use std::sync::Arc;

use harvest_db::Db;
use harvest_search::{Query, SearchResult};
use serde_json::json;

use crate::metrics::{
    SEARCH_DEGRADED, SEARCH_E2E, SEARCH_GATE, SEARCH_LLM_CALLS, SEARCH_SHADOW_RECALL, SEARCH_STAGE,
    SEARCH_TIER,
};

#[derive(Clone, Debug, Default)]
pub struct TraceContext {
    pub engine: &'static str,
    pub turn_id: Option<String>,
    pub conversation_id: Option<String>,
    pub shadow_recall: Option<f64>,
}

impl TraceContext {
    pub fn engine(engine: &'static str) -> Self {
        Self {
            engine,
            ..Self::default()
        }
    }
}

const INSERT_SQL: &str = r#"
INSERT INTO search_traces
    (turn_id, conversation_id, node_id, engine, repo, version, query, tier_reached, degraded,
     generation, total_ms, llm_calls, shadow_recall, evidence, gaps, trace)
VALUES
    ($turn_id, $conversation_id, $node_id, $engine, $repo, $version, $query, $tier, $degraded,
     $generation, $total_ms, $llm_calls, $shadow_recall, $evidence, $gaps, $trace)
"#;

#[derive(Clone)]
pub struct Recorder {
    db: Option<Arc<Db>>,
    node_id: String,
}

pub fn observe(result: &SearchResult, context: &TraceContext) {
    let tier = result.tier_reached.as_str();
    SEARCH_E2E
        .with(&[("engine", context.engine), ("tier", tier)])
        .record(result.trace.total_ms as u64);
    for stage in &result.trace.stages {
        SEARCH_STAGE
            .with(&[("stage", stage.stage.as_str())])
            .record(stage.ms as u64);
    }
    SEARCH_TIER.inc(&[("engine", context.engine), ("tier", tier)]);
    if let Some(degraded) = result.degraded {
        SEARCH_DEGRADED.inc(&[("reason", degraded.as_str())]);
    }
    if result.trace.llm_calls > 0 {
        SEARCH_LLM_CALLS.add(&[("tier", tier)], u64::from(result.trace.llm_calls));
    }
    for record in &result.trace.gate {
        SEARCH_GATE.inc(&[
            ("tier", record.tier.as_str()),
            ("check", record.check.as_str()),
            ("verdict", record.verdict.as_str()),
        ]);
    }
    if let Some(recall) = context.shadow_recall {
        SEARCH_SHADOW_RECALL
            .with(&[])
            .record((recall * 100.0).round() as u64);
    }
}

impl Recorder {
    pub fn new(db: Arc<Db>, node_id: impl Into<String>) -> Self {
        Self {
            db: Some(db),
            node_id: node_id.into(),
        }
    }

    pub fn metrics_only() -> Self {
        Self {
            db: None,
            node_id: String::new(),
        }
    }

    pub fn record(&self, query: &Query, result: &SearchResult, context: TraceContext) {
        observe(result, &context);
        let Some(db) = self.db.clone() else {
            return;
        };
        let evidence: Vec<_> = result
            .evidence
            .iter()
            .map(|e| {
                json!({
                    "key": e.key, "label": e.label, "score": e.score, "found_by": e.found_by,
                    "start_line": e.start_line, "end_line": e.end_line, "covers": e.covers,
                })
            })
            .collect();
        let params = json!({
            "turn_id": context.turn_id,
            "conversation_id": context.conversation_id,
            "node_id": self.node_id,
            "engine": context.engine,
            "repo": query.scope.repo,
            "version": query.scope.version,
            "query": query.text,
            "tier": result.tier_reached.as_str(),
            "degraded": result.degraded.map(|d| d.as_str()),
            "generation": result.generation,
            "total_ms": result.trace.total_ms,
            "llm_calls": result.trace.llm_calls as i64,
            "shadow_recall": context.shadow_recall,
            "evidence": evidence,
            "gaps": result.gaps,
            "trace": result.trace,
        });
        tokio::spawn(async move {
            if let Err(e) = db.execute(INSERT_SQL, params).await {
                tracing::debug!(error = %e, "could not store the search trace");
            }
        });
    }
}
