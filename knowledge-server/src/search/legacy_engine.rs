use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use harvest_search::{
    Deadline, Evidence, EvidenceKey, Query, SearchEngine, SearchError, SearchResult, Source,
    TierId, Trace,
};

use crate::agent::{Agent, Source as Citation};

pub struct LegacyLoopEngine {
    agent: Arc<Agent>,
}

impl LegacyLoopEngine {
    pub fn new(agent: Arc<Agent>) -> Self {
        Self { agent }
    }
}

pub fn citation_evidence(citation: &Citation) -> Evidence {
    let end_line = citation.end_line.unwrap_or(citation.line);
    Evidence {
        key: EvidenceKey::Span {
            file: citation.file.clone(),
            start_line: citation.line,
            end_line,
        },
        unit: None,
        label: format!("{}:{}-{}", citation.file, citation.line, end_line),
        start_line: citation.line,
        end_line,
        text: String::new(),
        score: 1.0,
        found_by: Source::Agent,
        covers: Vec::new(),
    }
}

pub fn prompt(query: &Query) -> String {
    format!(
        "{}\n\nAnswer for repository {} at version {}. Cite every piece of code you rely on as [repo:version:file:start-end].",
        query.text, query.scope.repo, query.scope.version
    )
}

#[async_trait]
impl SearchEngine for LegacyLoopEngine {
    async fn search(
        &self,
        query: &Query,
        _deadline: Deadline,
    ) -> Result<SearchResult, SearchError> {
        let started = Instant::now();
        let response = self
            .agent
            .query(&prompt(query), &[], &[], None)
            .await
            .map_err(|e| SearchError::Model(e.to_string()))?;
        let evidence = response
            .sources
            .iter()
            .filter(|s| s.repo == query.scope.repo && s.version == query.scope.version)
            .map(citation_evidence)
            .collect();
        let mut trace = Trace::default();
        trace.llm_calls = response.llm_call_count as u32;
        trace.rounds = response.turns as u32;
        trace.record("loop", started.elapsed());
        trace.total_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(SearchResult {
            evidence,
            gaps: Vec::new(),
            tier_reached: TierId::Loop,
            degraded: None,
            generation: None,
            trace,
            answer: Some(response.answer),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::Scope;

    #[test]
    fn citations_become_span_evidence() {
        let evidence = citation_evidence(&Citation {
            repo: "r".into(),
            version: "v".into(),
            file: "a.py".into(),
            line: 4,
            end_line: None,
        });
        assert_eq!(
            evidence.key,
            EvidenceKey::Span {
                file: "a.py".into(),
                start_line: 4,
                end_line: 4
            }
        );
        assert!(
            prompt(&Query::new("q", Scope::new("r", "v"))).contains("repository r at version v")
        );
    }
}
