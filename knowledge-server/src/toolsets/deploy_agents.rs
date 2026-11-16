use std::sync::Arc;

use harvest_db::Db;

use super::{Toolset, TurnContext};
use crate::agent::lxd_tools::{CreateLxdAgentTool, DeleteAgentTool};
use crate::agent::machine_tools::{ListAgentsTool, RunCommandTool};
use crate::agent::port_forward_tools::{
    CreatePortForwardTool, DeletePortForwardTool, ListPortForwardsTool, UpdatePortForwardTool,
};
use crate::agent::tool::Tool;
use crate::lxd::LxdClient;
use crate::machines::MachineRegistry;

pub struct DeployAgentsToolset {
    pub db: Arc<Db>,
    pub registry: Arc<MachineRegistry>,
    pub lxd: Option<Arc<LxdClient>>,
    pub server_url: String,
}

impl DeployAgentsToolset {
    fn agent_tools(&self, project_id: &str) -> Vec<Box<dyn Tool>> {
        let mut tools: Vec<Box<dyn Tool>> = vec![
            Box::new(ListAgentsTool {
                registry: Arc::clone(&self.registry),
                project_id: project_id.to_string(),
            }),
            Box::new(RunCommandTool {
                registry: Arc::clone(&self.registry),
                project_id: project_id.to_string(),
            }),
        ];
        if let Some(lxd) = &self.lxd {
            tools.push(Box::new(CreateLxdAgentTool {
                db: Arc::clone(&self.db),
                lxd: Arc::clone(lxd),
                server_url: self.server_url.clone(),
                project_id: project_id.to_string(),
            }));
        }
        tools.push(Box::new(DeleteAgentTool {
            db: Arc::clone(&self.db),
            lxd: self.lxd.clone(),
            registry: Arc::clone(&self.registry),
            project_id: project_id.to_string(),
        }));
        tools
    }

    fn port_forward_tools(&self, project_id: &str) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ListPortForwardsTool {
                db: Arc::clone(&self.db),
                project_id: project_id.to_string(),
            }),
            Box::new(CreatePortForwardTool {
                db: Arc::clone(&self.db),
                project_id: project_id.to_string(),
            }),
            Box::new(UpdatePortForwardTool {
                db: Arc::clone(&self.db),
                project_id: project_id.to_string(),
            }),
            Box::new(DeletePortForwardTool {
                db: Arc::clone(&self.db),
                project_id: project_id.to_string(),
            }),
        ]
    }
}

impl Toolset for DeployAgentsToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let Some(project_id) = turn.space.deploy_project_id() else {
            return Vec::new();
        };
        let mut tools = self.agent_tools(project_id);
        tools.extend(self.port_forward_tools(project_id));
        tools
    }
}
