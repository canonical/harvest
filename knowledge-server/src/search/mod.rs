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
    Deadline, Query, Scope, SearchEngine, SearchError, SearchResult, TokenBudget,
};

use crate::config::{SearchConfig, SearchMode};
use record::{Recorder, TraceContext};

pub struct SearchService {
    pub engine: Arc<dyn SearchEngine>,
    pub registry: Arc<IndexRegistry>,
    pub recorder: Recorder,
    pub config: SearchConfig,
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

    pub async fn search(
        &self,
        query: &Query,
        context: TraceContext,
    ) -> Result<SearchResult, SearchError> {
        let result = self.engine.search(query, self.deadline()).await;
        match &result {
            Ok(result) => self.recorder.record(query, result, context),
            Err(e) => tracing::warn!(error = %e, scope = ?query.scope, "code search failed"),
        }
        result
    }
}
