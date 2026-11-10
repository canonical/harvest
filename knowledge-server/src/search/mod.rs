pub mod code_tool;
pub mod index_sync;
pub mod legacy_engine;
pub mod llm_adapter;
pub mod record;
pub mod shadow;
pub mod wiring;

use std::sync::Arc;
use std::time::Duration;

use harvest_search::index::IndexRegistry;
use harvest_search::{
    Deadline, Evidence, Query, Scope, SearchEngine, SearchError, SearchResult, Source, TokenBudget,
};

use crate::config::{SearchConfig, SearchMode};
use crate::llm::LlmProvider;
use record::{Recorder, TraceContext};

pub struct SearchService {
    pub engine: Arc<dyn SearchEngine>,
    pub registry: Arc<IndexRegistry>,
    pub recorder: Recorder,
    pub config: SearchConfig,
    pub models: wiring::ModelEngines,
    pub calibration_base: harvest_search::calibration::Calibration,
}

impl SearchService {
    pub fn mode(&self) -> SearchMode {
        self.config.engine
    }

    pub fn deadline(&self) -> Deadline {
        Deadline::after(Duration::from_millis(self.config.deadline_ms))
    }

    pub fn query(&self, text: &str, repo: &str, version: &str) -> Query {
        let mut query = Query::new(text, Scope::new(repo, version));
        query.budget = TokenBudget(self.config.token_budget);
        query
    }

    pub fn engine_for(&self, llm: Option<&Arc<dyn LlmProvider>>) -> Arc<dyn SearchEngine> {
        match llm {
            Some(llm) => self.models.engine(&self.config, llm),
            None => Arc::clone(&self.engine),
        }
    }

    pub async fn search(
        &self,
        llm: Option<&Arc<dyn LlmProvider>>,
        query: &Query,
        context: TraceContext,
    ) -> Result<SearchResult, SearchError> {
        let result = self.engine_for(llm).search(query, self.deadline()).await;
        match &result {
            Ok(result) => self.recorder.record(query, result, context),
            Err(e) => tracing::warn!(error = %e, scope = ?query.scope, "code search failed"),
        }
        result
    }

    pub async fn focus(
        &self,
        scope: &Scope,
        items: &[String],
        budget: TokenBudget,
    ) -> Result<Vec<Evidence>, SearchError> {
        let Some(index) = self.registry.ensure(scope).await? else {
            return Err(SearchError::IndexCold(scope.clone()));
        };
        let mut remaining = budget.0 * 4;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for item in items {
            for unit in focus_units(&index, item) {
                if remaining < 200 || !seen.insert(unit) {
                    continue;
                }
                let text = harvest_search::retrieve::pack::render_whole(
                    &index,
                    unit,
                    remaining.min(12_000),
                );
                remaining = remaining.saturating_sub(text.len());
                out.push(harvest_search::retrieve::pack::evidence_for(
                    index.unit(unit),
                    text,
                    1.0,
                    Source::Exact,
                ));
            }
        }
        Ok(out)
    }

    pub async fn resolve_scope(
        db: &harvest_db::Db,
        question: &str,
        earlier: &str,
    ) -> Option<Scope> {
        let rows = db
            .query(INGESTED_SQL, serde_json::json!({}))
            .await
            .map_err(|e| tracing::warn!(error = %e, "could not list ingested versions"))
            .ok()?;
        let versions: Vec<(String, String)> = rows
            .iter()
            .filter_map(|r| {
                Some((
                    r["repo"].as_str()?.to_string(),
                    r["version"].as_str()?.to_string(),
                ))
            })
            .collect();
        pick_scope(&versions, question).or_else(|| pick_named_scope(&versions, earlier))
    }
}

const INGESTED_SQL: &str = r#"
SELECT r.name AS repo, v.tag AS version
FROM versions v
JOIN repositories r ON r.id = v.repository_id
WHERE v.ingested
ORDER BY v.timestamp DESC, v.id DESC
"#;

const MAX_FOCUS_UNITS: usize = 12;

fn focus_units(
    index: &harvest_search::index::IndexSnapshot,
    item: &str,
) -> Vec<harvest_search::domain::UnitId> {
    let path = item.trim().trim_start_matches("./");
    let in_file: Vec<&harvest_search::index::corpus::Unit> = index
        .units
        .iter()
        .filter(|u| u.file == path || (path.contains('/') && u.file.ends_with(&format!("/{path}"))))
        .collect();
    if !in_file.is_empty() {
        let mut symbols: Vec<_> = in_file
            .iter()
            .filter(|u| u.kind().is_symbol() && u.parent.is_none())
            .copied()
            .collect();
        if symbols.is_empty() {
            symbols = in_file;
        }
        symbols.sort_by_key(|u| u.start_line);
        return symbols.iter().take(MAX_FOCUS_UNITS).map(|u| u.id).collect();
    }
    index
        .find_by_name(item)
        .iter()
        .copied()
        .take(harvest_search::index::MAX_NAMED_MATCHES)
        .collect()
}

fn mentioned(text: &str, word: &str) -> bool {
    let word = word.to_lowercase();
    text.match_indices(&word).any(|(i, _)| {
        let before = text[..i].chars().last();
        let after = text[i + word.len()..].chars().next();
        let boundary =
            |c: Option<char>| c.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
        boundary(before) && boundary(after)
    })
}

pub fn pick_named_scope(versions: &[(String, String)], text: &str) -> Option<Scope> {
    let lower = text.to_lowercase();
    let mut named: Vec<&str> = Vec::new();
    for (repo, _) in versions {
        if mentioned(&lower, repo) && !named.contains(&repo.as_str()) {
            named.push(repo);
        }
    }
    match named.as_slice() {
        [_] => pick_scope(versions, text),
        _ => None,
    }
}

pub fn pick_scope(versions: &[(String, String)], question: &str) -> Option<Scope> {
    let text = question.to_lowercase();
    let mut repos: Vec<&str> = Vec::new();
    for (repo, _) in versions {
        if !repos.contains(&repo.as_str()) {
            repos.push(repo);
        }
    }
    let named: Vec<&str> = repos
        .iter()
        .copied()
        .filter(|r| mentioned(&text, r))
        .collect();
    let repo = match (named.as_slice(), repos.as_slice()) {
        ([one], _) => *one,
        ([], [only]) => *only,
        _ => return None,
    };
    let of_repo: Vec<&String> = versions
        .iter()
        .filter(|(r, _)| r == repo)
        .map(|(_, v)| v)
        .collect();
    let version = of_repo
        .iter()
        .find(|v| mentioned(&text, v))
        .or_else(|| of_repo.first())?;
    Some(Scope::new(repo, version.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions() -> Vec<(String, String)> {
        vec![
            ("cinder".into(), "29.0.0".into()),
            ("cinder".into(), "28.0.0".into()),
            ("nova".into(), "30.1.0".into()),
        ]
    }

    #[test]
    fn scope_comes_from_the_question_or_the_only_repository() {
        assert_eq!(
            pick_scope(&versions(), "How does cinder attach volumes?"),
            Some(Scope::new("cinder", "29.0.0"))
        );
        assert_eq!(
            pick_scope(&versions(), "In cinder 28.0.0, what calls attach?"),
            Some(Scope::new("cinder", "28.0.0"))
        );
        assert_eq!(pick_scope(&versions(), "How are volumes attached?"), None);
        assert_eq!(
            pick_scope(&versions(), "Compare cinder and nova attach"),
            None
        );
        let single = vec![("harvest".to_string(), "main".to_string())];
        assert_eq!(
            pick_scope(&single, "How does Fence::commit work?"),
            Some(Scope::new("harvest", "main"))
        );
        assert_eq!(pick_scope(&[], "anything"), None);
        assert_eq!(
            pick_named_scope(&versions(), "What are the Cinder drivers available?"),
            Some(Scope::new("cinder", "29.0.0"))
        );
        assert_eq!(
            pick_named_scope(&versions(), "How are volumes attached?"),
            None
        );
    }
}
