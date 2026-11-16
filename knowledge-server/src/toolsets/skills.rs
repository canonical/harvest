use std::sync::Arc;

use super::{Toolset, TurnContext};
use crate::agent::skill_tools::{ListSkillsTool, LoadSkillTool};
use crate::agent::tool::Tool;
use crate::skills::SkillStore;

pub struct SkillsToolset {
    pub store: Arc<SkillStore>,
}

impl Toolset for SkillsToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let Some(project_id) = turn.space.deploy_project_id() else {
            return Vec::new();
        };
        vec![
            Box::new(ListSkillsTool {
                store: Arc::clone(&self.store),
                project_id: project_id.to_string(),
            }),
            Box::new(LoadSkillTool {
                store: Arc::clone(&self.store),
                project_id: project_id.to_string(),
            }),
        ]
    }
}
