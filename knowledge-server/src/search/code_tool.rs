use std::sync::Arc;

use anyhow::{bail, Result};
use async_trait::async_trait;
use harvest_search::{Degraded, Query, SearchResult};
use serde_json::{json, Value};

use crate::agent::tool::Tool;
use crate::llm::types::ToolDefinition;
use crate::search::record::TraceContext;
use crate::search::SearchService;

pub struct SearchCodeTool {
    service: Arc<SearchService>,
}

impl SearchCodeTool {
    pub fn new(service: Arc<SearchService>) -> Self {
        Self { service }
    }
}

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "search_code".into(),
        description: "Find the code that answers a question about one repository version. \
                      Pass the user's whole question. The search runs several retrieval rounds \
                      over code, configuration and docs, follows calls, inheritance and \
                      registrations, and returns the source evidence with citations, plus any \
                      gaps it could not close. Call it once per question; use read_sources only \
                      for follow-ups it names."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query":   { "type": "string", "description": "The full question in natural language" },
                "repo":    { "type": "string" },
                "version": { "type": "string" },
                "budget_tokens": { "type": "integer", "description": "Evidence size, default 8000" }
            },
            "required": ["query", "repo", "version"]
        }),
    }
}

pub fn render(query: &Query, result: &SearchResult) -> String {
    let scope = &query.scope;
    let mut out = format!(
        "Evidence for {:?} in {}@{} ({}, {:.0} ms). Cite code as [{}:{}:<file>:<start>-<end>].\n",
        query.text,
        scope.repo,
        scope.version,
        result.tier_reached.as_str(),
        result.trace.total_ms,
        scope.repo,
        scope.version,
    );
    match result.degraded {
        Some(Degraded::IndexCold) => out.push_str(
            "Note: the search index for this version is still loading, so these results come from a simpler name search.\n",
        ),
        Some(Degraded::ModelUnavailable) => out.push_str(
            "Note: the planning model was unavailable, so only the first search rounds ran.\n",
        ),
        Some(Degraded::DeadlineTruncated) => {
            out.push_str("Note: the search stopped at its time budget.\n")
        }
        None => {}
    }
    for (i, evidence) in result.evidence.iter().enumerate() {
        let file = evidence.key.file();
        if file.is_empty() {
            out.push_str(&format!(
                "\n[{}] {}\n{}\n",
                i + 1,
                evidence.label,
                evidence.text
            ));
        } else {
            out.push_str(&format!(
                "\n[{}] {} [{}:{}:{}:{}-{}]\n{}\n",
                i + 1,
                evidence.label,
                scope.repo,
                scope.version,
                file,
                evidence.start_line,
                evidence.end_line,
                evidence.text
            ));
        }
    }
    if result.evidence.is_empty() {
        out.push_str("\nNo matching code was found.\n");
    }
    if !result.gaps.is_empty() {
        out.push_str("\nGaps the search could not close:\n");
        for gap in &result.gaps {
            out.push_str(&format!("- {}\n", gap.describe()));
        }
    }
    out
}

fn text(params: &Value, key: &str) -> String {
    params[key].as_str().unwrap_or("").trim().to_string()
}

#[async_trait]
impl Tool for SearchCodeTool {
    fn definition(&self) -> ToolDefinition {
        definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (question, repo, version) = (
            text(&params, "query"),
            text(&params, "repo"),
            text(&params, "version"),
        );
        if question.is_empty() || repo.is_empty() || version.is_empty() {
            bail!("search_code needs a query, a repo and a version; use list_repositories to find them");
        }
        let mut query = self.service.query(&question, &repo, &version);
        if let Some(budget) = params["budget_tokens"].as_u64() {
            query.budget = harvest_search::TokenBudget((budget as usize).clamp(1_000, 32_000));
        }
        let result = self
            .service
            .search(&query, TraceContext::engine("tiered"))
            .await?;
        Ok(render(&query, &result))
    }
}

pub fn tiered_tools(db: Arc<harvest_db::Db>, service: Arc<SearchService>) -> Vec<Box<dyn Tool>> {
    use crate::agent::graph_tools::{
        run_sql_available, CompareSymbolAcrossVersionsTool, ListRepositoriesTool, ReadSourcesTool,
        RunSqlTool,
    };
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ListRepositoriesTool(Arc::clone(&db))),
        Box::new(SearchCodeTool::new(service)),
        Box::new(ReadSourcesTool(Arc::clone(&db))),
        Box::new(CompareSymbolAcrossVersionsTool(Arc::clone(&db))),
    ];
    if run_sql_available() {
        tools.push(Box::new(RunSqlTool(db)));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_search::{Evidence, EvidenceKey, Gap, Scope, Source, TierId};

    #[test]
    fn rendering_shows_citations_notes_and_gaps() {
        let query = Query::new("how?", Scope::new("cinder", "v1"));
        let result = SearchResult {
            evidence: vec![Evidence {
                key: EvidenceKey::Symbol {
                    file: "a.py".into(),
                    name: "f".into(),
                },
                unit: None,
                label: "function f (a.py)".into(),
                start_line: 3,
                end_line: 9,
                text: "def f(): pass".into(),
                score: 1.0,
                found_by: Source::Lexical,
                covers: Vec::new(),
            }],
            gaps: vec![Gap::Unanswered {
                hint: "who calls f".into(),
            }],
            tier_reached: TierId::Plan,
            degraded: Some(Degraded::DeadlineTruncated),
            generation: None,
            trace: Default::default(),
            answer: None,
        };
        let text = render(&query, &result);
        assert!(text.contains("[cinder:v1:a.py:3-9]"));
        assert!(text.contains("tier2"));
        assert!(text.contains("time budget"));
        assert!(text.contains("- who calls f"));
    }
}
