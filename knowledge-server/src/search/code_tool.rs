use std::sync::Arc;

use anyhow::{bail, Result};
use async_trait::async_trait;
use harvest_db::Db;
use harvest_search::{Degraded, Evidence, Query, Scope, SearchResult};
use serde_json::{json, Value};

use crate::agent::tool::{Tool, ToolOutput};
use crate::llm::types::ToolDefinition;
use crate::llm::LlmProvider;
use crate::search::record::TraceContext;
use crate::search::view::{focus_view, search_view, SearchView};
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

    fn with_view(text: String, view: SearchView) -> ToolOutput {
        match serde_json::to_value(view) {
            Ok(view) => ToolOutput::with_view(text, view),
            Err(_) => ToolOutput::text(text),
        }
    }

    async fn search(
        &self,
        question: &str,
        scope: &Scope,
        budget: Option<u64>,
    ) -> Result<ToolOutput> {
        let mut query = self.service.query(question, &scope.repo, &scope.version);
        if let Some(budget) = budget {
            query.budget = harvest_search::TokenBudget((budget as usize).clamp(1_000, 32_000));
        }
        let result = self
            .service
            .search(self.context.llm.as_ref(), &query, self.trace())
            .await?;
        let index = self.service.registry.get(scope);
        let view = search_view(index.as_deref(), &query, &result);
        Ok(Self::with_view(render(&query, &result), view))
    }

    async fn read_focus(&self, scope: &Scope, focus: &[String]) -> Result<ToolOutput> {
        let budget = harvest_search::TokenBudget(self.service.config.token_budget);
        let evidence = self.service.focus(scope, focus, budget).await?;
        let index = self.service.registry.get(scope);
        let view = focus_view(index.as_deref(), scope, focus, &evidence);
        Ok(Self::with_view(render_focus(scope, focus, &evidence), view))
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

pub const CODE_HEADING: &str = "## Code (authoritative: base the answer on this)";
pub const DOCUMENTATION_HEADING: &str = "## Documentation (not verified: it shows where to look in the code and may be outdated or describe plans)";

fn push_item(out: &mut String, number: &str, evidence: &Evidence, scope: &Scope) {
    let file = evidence.key.file();
    if file.is_empty() {
        out.push_str(&format!(
            "\n[{number}] {}\n{}\n",
            evidence.label, evidence.text
        ));
    } else {
        out.push_str(&format!(
            "\n[{number}] {} [{}:{}:{}:{}-{}]\n{}\n",
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
    out.push_str(&format!("\n{CODE_HEADING}\n"));
    for (i, evidence) in result.evidence.iter().enumerate() {
        push_item(&mut out, &(i + 1).to_string(), evidence, scope);
    }
    if result.evidence.is_empty() {
        out.push_str(if result.documentation.is_empty() {
            "\nNo matching code was found.\n"
        } else {
            "\nNo matching code was found; only documentation matched. Say so in the answer.\n"
        });
    }
    if !result.documentation.is_empty() {
        out.push_str(&format!("\n{DOCUMENTATION_HEADING}\n"));
        for (i, evidence) in result.documentation.iter().enumerate() {
            push_item(&mut out, &format!("D{}", i + 1), evidence, scope);
        }
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
        Ok(self.run(params).await?.text)
    }

    async fn run(&self, params: Value) -> Result<ToolOutput> {
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
            return self.read_focus(&scope, &focus).await;
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

    async fn prefetch(&self, question: &str, earlier: &str) -> Option<(Value, ToolOutput)> {
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
        push_item(&mut out, &(i + 1).to_string(), item, scope);
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
            documentation: Vec::new(),
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

    #[test]
    fn code_and_documentation_render_as_separate_sections() {
        let query = Query::new("How is it installed?", Scope::new("amafu", "main"));
        let item = |file: &str, label: &str| Evidence {
            key: EvidenceKey::Symbol {
                file: file.into(),
                name: label.into(),
            },
            unit: None,
            label: label.into(),
            start_line: 1,
            end_line: 9,
            text: "body".into(),
            score: 1.0,
            found_by: Source::Lexical,
            covers: Vec::new(),
        };
        let result = SearchResult {
            evidence: vec![item("crates/installer/src/maas.rs", "class MaasClient")],
            documentation: vec![item("README.md", "doc Install with MAAS (planned)")],
            gaps: Vec::new(),
            tier_reached: TierId::Rounds,
            degraded: None,
            generation: None,
            trace: Default::default(),
            answer: None,
        };
        let text = render(&query, &result);
        let code = text.find(CODE_HEADING).unwrap();
        let docs = text.find(DOCUMENTATION_HEADING).unwrap();
        assert!(code < text.find("class MaasClient").unwrap());
        assert!(docs > text.find("class MaasClient").unwrap());
        assert!(text.contains("[D1] doc Install with MAAS (planned) [amafu:main:README.md:1-9]"));

        let only_docs = SearchResult {
            evidence: Vec::new(),
            ..result
        };
        assert!(render(&query, &only_docs).contains("only documentation matched"));
    }
}
