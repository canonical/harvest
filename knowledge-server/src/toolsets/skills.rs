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
        let owner = turn.space.owner();
        vec![
            Box::new(ListSkillsTool {
                store: Arc::clone(&self.store),
                owner: owner.clone(),
            }),
            Box::new(LoadSkillTool {
                store: Arc::clone(&self.store),
                owner,
            }),
        ]
    }
}
