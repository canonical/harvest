mod artifacts;
mod deploy_agents;
mod knowledge;
mod repo_guard;
mod sandboxes;
mod skills;
mod terraform;

use std::sync::Arc;

pub use artifacts::ArtifactsToolset;
pub use deploy_agents::DeployAgentsToolset;
pub use knowledge::KnowledgeToolset;
pub use sandboxes::SandboxesToolset;
pub use skills::SkillsToolset;
pub use terraform::TerraformToolset;

use harvest_db::Db;

use crate::access::repos::{RepoAccess, RepoScope};
use crate::agent::graph_tools::SemanticHandle;
use crate::agent::tool::Tool;
use crate::collocate::client::CollocateHandle;
use crate::collocate::sessions::SessionContainerRegistry;
use crate::llm::LlmProvider;
use crate::lxd::LxdClient;
use crate::machines::MachineRegistry;
use crate::modes::{Mode, Space};
use crate::search::SearchService;
use crate::skills::SkillStore;

pub struct TurnContext {
    pub user_id: String,
    pub space: Space,
    pub repos: RepoScope,
    pub conversation_id: String,
    pub llm: Arc<dyn LlmProvider>,
}

impl TurnContext {
    pub async fn load(
        db: &Db,
        user_id: &str,
        space: Space,
        conversation_id: impl Into<String>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        let repos = RepoAccess::new(db)
            .scope_for(user_id)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = %e, user_id, "could not load repository access; allowing none");
                RepoScope::only(Vec::<String>::new())
            });
        Self {
            user_id: user_id.to_string(),
            space,
            repos,
            conversation_id: conversation_id.into(),
            llm,
        }
    }

    pub fn unrestricted(
        user_id: &str,
        space: Space,
        conversation_id: impl Into<String>,
        llm: Arc<dyn LlmProvider>,
    ) -> Self {
        Self {
            user_id: user_id.to_string(),
            space,
            repos: RepoScope::All,
            conversation_id: conversation_id.into(),
            llm,
        }
    }
}

pub trait Toolset: Send + Sync {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolsetId {
    Knowledge,
    Artifacts,
    Skills,
    DeployAgents,
    Terraform,
    Sandboxes,
}

pub fn for_mode(mode: Mode) -> &'static [ToolsetId] {
    use ToolsetId::*;
    match mode {
        Mode::Chat => &[Knowledge, Artifacts, Skills],
        Mode::Code => &[Knowledge],
        Mode::Deploy => &[
            Knowledge,
            Artifacts,
            Skills,
            DeployAgents,
            Terraform,
            Sandboxes,
        ],
    }
}

pub struct ToolboxDeps {
    pub db: Arc<Db>,
    pub semantic: Option<Arc<SemanticHandle>>,
    pub search: Option<Arc<SearchService>>,
    pub skills: Arc<SkillStore>,
    pub registry: Arc<MachineRegistry>,
    pub lxd: Option<Arc<LxdClient>>,
    pub collocate: Option<CollocateHandle>,
    pub collocate_registry: Arc<SessionContainerRegistry>,
    pub server_url: String,
}

pub fn build_toolbox(deps: ToolboxDeps) -> Toolbox {
    let toolbox = Toolbox::default()
        .with(
            ToolsetId::Knowledge,
            KnowledgeToolset {
                db: Arc::clone(&deps.db),
                semantic: deps.semantic,
                search: deps.search,
            },
        )
        .with(
            ToolsetId::Artifacts,
            ArtifactsToolset {
                db: Arc::clone(&deps.db),
                server_url: deps.server_url.clone(),
            },
        )
        .with(ToolsetId::Skills, SkillsToolset { store: deps.skills })
        .with(
            ToolsetId::DeployAgents,
            DeployAgentsToolset {
                db: Arc::clone(&deps.db),
                registry: Arc::clone(&deps.registry),
                lxd: deps.lxd,
                server_url: deps.server_url,
            },
        )
        .with(
            ToolsetId::Terraform,
            TerraformToolset {
                db: Arc::clone(&deps.db),
                registry: deps.registry,
            },
        );
    match deps.collocate {
        Some(handle) => toolbox.with(
            ToolsetId::Sandboxes,
            SandboxesToolset {
                handle,
                registry: deps.collocate_registry,
            },
        ),
        None => toolbox,
    }
}

#[derive(Default)]
pub struct Toolbox {
    toolsets: Vec<(ToolsetId, Box<dyn Toolset>)>,
}

impl Toolbox {
    pub fn with(mut self, id: ToolsetId, toolset: impl Toolset + 'static) -> Self {
        self.toolsets.push((id, Box::new(toolset)));
        self
    }

    pub fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        self.tools_from(for_mode(turn.space.mode()), turn)
    }

    pub fn tools_from(&self, ids: &[ToolsetId], turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        ids.iter()
            .flat_map(|id| {
                self.toolsets
                    .iter()
                    .filter(move |(registered, _)| registered == id)
                    .flat_map(|(_, toolset)| toolset.tools(turn))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockProvider;
    use crate::llm::types::ToolDefinition;
    use async_trait::async_trait;
    use serde_json::{json, Value};

    struct Stub(&'static str);

    #[async_trait]
    impl Tool for Stub {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.0.into(),
                description: String::new(),
                parameters: json!({}),
            }
        }

        async fn execute(&self, _params: Value) -> anyhow::Result<String> {
            Ok(String::new())
        }
    }

    struct Named(&'static str);

    impl Toolset for Named {
        fn tools(&self, _turn: &TurnContext) -> Vec<Box<dyn Tool>> {
            vec![Box::new(Stub(self.0))]
        }
    }

    fn turn(space: Space) -> TurnContext {
        TurnContext {
            user_id: "u".into(),
            space,
            repos: RepoScope::All,
            conversation_id: "c".into(),
            llm: Arc::new(MockProvider::new(
                "m".into(),
                Default::default(),
                Default::default(),
            )),
        }
    }

    fn names(tools: &[Box<dyn Tool>]) -> Vec<String> {
        tools.iter().map(|t| t.definition().name).collect()
    }

    #[test]
    fn a_mode_only_gets_the_toolsets_listed_for_it() {
        let toolbox = Toolbox::default()
            .with(ToolsetId::Knowledge, Named("search"))
            .with(ToolsetId::DeployAgents, Named("run_command"));
        assert_eq!(
            names(&toolbox.tools(&turn(Space::personal("u")))),
            vec!["search"]
        );
        assert_eq!(
            names(&toolbox.tools(&turn(Space::deploy_project("p")))),
            vec!["search", "run_command"]
        );
    }

    #[test]
    fn every_mode_can_search_code() {
        for mode in [Mode::Chat, Mode::Code, Mode::Deploy] {
            assert!(for_mode(mode).contains(&ToolsetId::Knowledge));
        }
    }

    #[test]
    fn only_deploy_mode_reaches_deployment_agents() {
        assert!(!for_mode(Mode::Chat).contains(&ToolsetId::DeployAgents));
        assert!(!for_mode(Mode::Code).contains(&ToolsetId::DeployAgents));
        assert!(for_mode(Mode::Deploy).contains(&ToolsetId::DeployAgents));
    }
}
