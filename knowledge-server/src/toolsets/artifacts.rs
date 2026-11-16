use std::sync::Arc;

use harvest_db::Db;

use super::{Toolset, TurnContext};
use crate::agent::artifact_tools::GenerateArtifactTool;
use crate::agent::tool::Tool;

pub struct ArtifactsToolset {
    pub db: Arc<Db>,
    pub server_url: String,
}

impl Toolset for ArtifactsToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let Some(project_id) = turn.space.deploy_project_id() else {
            return Vec::new();
        };
        vec![Box::new(GenerateArtifactTool {
            db: Arc::clone(&self.db),
            project_id: project_id.to_string(),
            server_url: self.server_url.clone(),
        })]
    }
}
