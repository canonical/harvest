use std::sync::Arc;

use super::{Toolset, TurnContext};
use crate::agent::tool::Tool;
use crate::collocate::client::CollocateHandle;
use crate::collocate::sessions::SessionContainerRegistry;
use crate::collocate::tools::{
    CollocateCreateSessionTool, CollocateDeleteSessionTool, CollocateExecTool,
    CollocateListContainersTool, CollocateRunTool, CollocateTransferFileTool,
};

pub struct SandboxesToolset {
    pub handle: CollocateHandle,
    pub registry: Arc<SessionContainerRegistry>,
}

impl Toolset for SandboxesToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let Some(project_id) = turn.space.deploy_project_id() else {
            return Vec::new();
        };
        let project_id = project_id.to_string();
        let conversation_id = turn.conversation_id.clone();
        vec![
            Box::new(CollocateRunTool {
                handle: self.handle.clone(),
                project_id: project_id.clone(),
            }),
            Box::new(CollocateCreateSessionTool {
                handle: self.handle.clone(),
                registry: Arc::clone(&self.registry),
                project_id: project_id.clone(),
                conversation_id: conversation_id.clone(),
            }),
            Box::new(CollocateExecTool {
                handle: self.handle.clone(),
                registry: Arc::clone(&self.registry),
                project_id: project_id.clone(),
                conversation_id: conversation_id.clone(),
            }),
            Box::new(CollocateDeleteSessionTool {
                handle: self.handle.clone(),
                registry: Arc::clone(&self.registry),
                project_id: project_id.clone(),
                conversation_id: conversation_id.clone(),
            }),
            Box::new(CollocateListContainersTool {
                registry: Arc::clone(&self.registry),
                project_id: project_id.clone(),
                conversation_id: conversation_id.clone(),
            }),
            Box::new(CollocateTransferFileTool {
                handle: self.handle.clone(),
                registry: Arc::clone(&self.registry),
                project_id,
                conversation_id,
            }),
        ]
    }
}
