use std::sync::Arc;

use anyhow::{bail, Result};
use async_trait::async_trait;
use harvest_db::Db;
use harvest_search::{Degraded, Evidence, Query, Scope, SearchResult};
use serde_json::{json, Value};

use crate::agent::tool::Tool;
use crate::llm::types::ToolDefinition;
use crate::llm::LlmProvider;
use crate::search::record::TraceContext;
use crate::search::SearchService;

#[derive(Clone, Default)]
pub struct SearchToolContext {
    pub llm: Option<Arc<dyn LlmProvider>>,
    pub conversation_id: Option<String>,
}

impl SearchToolContext {
    pub fn with_llm(llm: Arc<dyn LlmProvider>) -> Self {
        Self {
            llm: Some(llm),
            conversation_id: None,
        }
    }

    pub fn in_conversation(mut self, conversation_id: impl Into<String>) -> Self {
        let id = conversation_id.into();
        self.conversation_id = (!id.is_empty()).then_some(id);
        self
    }
}

pub struct SearchCodeTool {
    service: Arc<SearchService>,
    db: Option<Arc<Db>>,
    context: SearchToolContext,
}

impl SearchCodeTool {
    pub fn new(service: Arc<SearchService>, context: SearchToolContext) -> Self {
        Self {
            service,
            db: None,
            context,
        }
    }

    pub fn with_db(mut self, db: Arc<Db>) -> Self {
        self.db = Some(db);
        self
    }

    fn trace(&self) -> TraceContext {
        TraceContext {
            conversation_id: self.context.conversation_id.clone(),
            ..TraceContext::engine("tiered")
        }
    }

    async fn search(&self, question: &str, scope: &Scope, budget: Option<u64>) -> Result<String> {
        let mut query = self.service.query(question, &scope.repo, &scope.version);
        if let Some(budget) = budget {
            query.budget = harvest_search::TokenBudget((budget as usize).clamp(1_000, 32_000));
        }
        let result = self
            .service
            .search(self.context.llm.as_ref(), &query, self.trace())
            .await?;
        Ok(render(&query, &result))
    }
}

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "search_code".into(),
        description: "Find the code that answers a question about one repository version. \
                      Pass the user's whole question. The search runs several retrieval rounds \
                      over code, configuration and docs, follows calls, inheritance and \
                      registrations, and returns whole functions with their class and the \
                      signatures they call, with citations, plus any gaps it could not close. \
                      Call it once per question. For a follow-up the evidence or its gaps name, \
                      pass `focus` with those symbol names or file paths instead of a new query."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query":   { "type": "string", "description": "The full question in natural language" },
                "repo":    { "type": "string" },
                "version": { "type": "string" },
                "focus":   {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Symbol names (any language's syntax, e.g. Fence::commit or a.b.C) or file paths to read in full"
                },
                "budget_tokens": { "type": "integer", "description": "Evidence size, default 8000" }
            },
            "required": ["repo", "version"]
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
        if repo.is_empty() || version.is_empty() {
            bail!("search_code needs a repo and a version; use list_repositories to find them");
        }
        let scope = Scope::new(&repo, &version);
        let focus: Vec<String> = params["focus"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(|i| i.trim().to_string())
                    .filter(|i| !i.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if !focus.is_empty() {
            let budget = harvest_search::TokenBudget(self.service.config.token_budget);
            let evidence = self.service.focus(&scope, &focus, budget).await?;
            return Ok(render_focus(&scope, &focus, &evidence));
        }
        if question.is_empty() {
            bail!("search_code needs a query, or a focus list of symbols or files");
        }
        self.search(&question, &scope, params["budget_tokens"].as_u64())
            .await
    }

    fn reads_code(&self) -> bool {
        true
    }

    fn read_budget(&self) -> Option<usize> {
        Some(self.service.config.max_followups)
    }

    async fn prefetch(&self, question: &str, earlier: &str) -> Option<(Value, String)> {
        if !self.service.config.prefetch || question.trim().is_empty() {
            return None;
        }
        let scope = SearchService::resolve_scope(self.db.as_ref()?, question, earlier).await?;
        if self.service.registry.get(&scope).is_none() {
            let registry = Arc::clone(&self.service.registry);
            let warming = scope.clone();
            tokio::spawn(async move {
                if let Err(e) = registry.ensure(&warming).await {
                    tracing::warn!(error = %e, scope = ?warming, "warming the index failed");
                }
            });
            tracing::info!(
                ?scope,
                "index not loaded yet; skipping the first code search"
            );
            return None;
        }
        match self.search(question, &scope, None).await {
            Ok(result) => Some((
                json!({ "query": question, "repo": scope.repo, "version": scope.version }),
                result,
            )),
            Err(e) => {
                tracing::warn!(error = %e, "first code search failed; the agent will search itself");
                None
            }
        }
    }
}

pub fn render_focus(scope: &Scope, focus: &[String], evidence: &[Evidence]) -> String {
    let mut out = format!(
        "Focused reads of {} in {}@{}. Cite code as [{}:{}:<file>:<start>-<end>].\n",
        focus.join(", "),
        scope.repo,
        scope.version,
        scope.repo,
        scope.version
    );
    for (i, item) in evidence.iter().enumerate() {
        out.push_str(&format!(
            "\n[{}] {} [{}:{}:{}:{}-{}]\n{}\n",
            i + 1,
            item.label,
            scope.repo,
            scope.version,
            item.key.file(),
            item.start_line,
            item.end_line,
            item.text
        ));
    }
    if evidence.is_empty() {
        out.push_str("\nNone of these names or paths are in the index for this version.\n");
    }
    out
}

pub fn tiered_tools(
    db: Arc<harvest_db::Db>,
    service: Arc<SearchService>,
    context: SearchToolContext,
) -> Vec<Box<dyn Tool>> {
    use crate::agent::graph_tools::{
        CompareSymbolAcrossVersionsTool, ListRepositoriesTool, ReadSourcesTool,
    };
    vec![
        Box::new(ListRepositoriesTool(Arc::clone(&db))),
        Box::new(SearchCodeTool::new(service, context).with_db(Arc::clone(&db))),
        Box::new(ReadSourcesTool(Arc::clone(&db))),
        Box::new(CompareSymbolAcrossVersionsTool(db)),
    ]
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
