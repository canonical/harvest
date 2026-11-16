use std::sync::Arc;

use harvest_db::Db;

use super::{Toolset, TurnContext};
use crate::agent::terraform_tools::{
    RunTerraformApplyTool, RunTerraformDestroyTool, RunTerraformPlanTool,
};
use crate::agent::tool::Tool;
use crate::machines::MachineRegistry;

pub struct TerraformToolset {
    pub db: Arc<Db>,
    pub registry: Arc<MachineRegistry>,
}

impl Toolset for TerraformToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let Some(project_id) = turn.space.deploy_project_id() else {
            return Vec::new();
        };
        vec![
            Box::new(RunTerraformPlanTool {
                db: Arc::clone(&self.db),
                registry: Arc::clone(&self.registry),
                project_id: project_id.to_string(),
            }),
            Box::new(RunTerraformApplyTool {
                db: Arc::clone(&self.db),
                registry: Arc::clone(&self.registry),
                project_id: project_id.to_string(),
            }),
            Box::new(RunTerraformDestroyTool {
                db: Arc::clone(&self.db),
                registry: Arc::clone(&self.registry),
                project_id: project_id.to_string(),
            }),
        ]
    }
}
