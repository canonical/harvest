use std::sync::Arc;

use harvest_db::Db;

use super::repo_guard::RepoGuard;
use super::{Toolset, TurnContext};
use crate::agent::graph_tools::{self, SemanticHandle};
use crate::agent::tool::Tool;
use crate::code::overlay::WorkspaceOverlay;
use crate::code::CodeEnv;
use crate::config::SearchMode;
use crate::search::code_tool::{self, SearchToolContext};
use crate::search::SearchService;

pub struct KnowledgeToolset {
    pub db: Arc<Db>,
    pub semantic: Option<Arc<SemanticHandle>>,
    pub search: Option<Arc<SearchService>>,
    pub workspace: Option<CodeEnv>,
}

impl Toolset for KnowledgeToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let context = SearchToolContext::with_llm(Arc::clone(&turn.llm))
            .in_conversation(&turn.conversation_id)
            .focused_on(turn.focus.clone());
        let tools = match &self.search {
            Some(service) if service.mode() == SearchMode::Tiered => {
                code_tool::tiered_tools(Arc::clone(&self.db), Arc::clone(service), context)
            }
            _ => graph_tools::all_tools_with_semantic(Arc::clone(&self.db), self.semantic.clone()),
        };
        let guarded = RepoGuard::wrap(tools, &turn.repos);
        match (&self.workspace, turn.space.code_session_id()) {
            (Some(env), Some(session_id)) => WorkspaceOverlay::wrap(guarded, env, session_id),
            _ => guarded,
        }
    }
}
