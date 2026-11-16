use std::sync::Arc;

use harvest_db::Db;

use super::{Toolset, TurnContext};
use crate::agent::graph_tools::{self, SemanticHandle};
use crate::agent::tool::Tool;
use crate::config::SearchMode;
use crate::search::code_tool::{self, SearchToolContext};
use crate::search::SearchService;

pub struct KnowledgeToolset {
    pub db: Arc<Db>,
    pub semantic: Option<Arc<SemanticHandle>>,
    pub search: Option<Arc<SearchService>>,
}

impl Toolset for KnowledgeToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let context = SearchToolContext::with_llm(Arc::clone(&turn.llm))
            .in_conversation(&turn.conversation_id);
        match &self.search {
            Some(service) if service.mode() == SearchMode::Tiered => {
                code_tool::tiered_tools(Arc::clone(&self.db), Arc::clone(service), context)
            }
            _ => graph_tools::all_tools_with_semantic(Arc::clone(&self.db), self.semantic.clone()),
        }
    }
}
