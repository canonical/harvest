use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::llm::types::ToolDefinition;
use crate::lxd::{Flavor, LxdClient};
use crate::machines::{handlers::delete_agent_core, lxd_provision, MachineRegistry};
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

const CREATE_PREVIEW_CHARS: usize = 500;
const DELETE_PREVIEW_CHARS: usize = 500;

fn validate_create_params(params: &Value) -> Result<(String, String, Flavor)> {
    let name = params["name"].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("name is required"))?
        .to_string();
    let description = params["description"].as_str().unwrap_or_default().to_string();
    let flavor_id = params["flavor"].as_str()
        .ok_or_else(|| anyhow!("flavor is required"))?;
    let flavor = Flavor::from_id(flavor_id)
        .ok_or_else(|| anyhow!("unknown flavor '{flavor_id}'"))?;
    Ok((name, description, flavor))
}

fn validate_delete_params(params: &Value) -> Result<String> {
    params["agent_id"].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("agent_id is required"))
}

pub struct CreateLxdAgentTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub lxd:        Arc<LxdClient>,
    pub server_url: String,
    pub project_id: String,
}

#[async_trait]
impl Tool for CreateLxdAgentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_lxd_agent".into(),
            description: "Provision a new LXD-managed agent machine for this project. \
                          This creates real infrastructure (an LXD container) and must only be \
                          called after the user has explicitly confirmed they want a new agent."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "A short name for the agent, e.g. 'build-runner'"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional description of what this agent is for"
                    },
                    "flavor": {
                        "type": "string",
                        "enum": Flavor::all().iter().map(|f| f.id()).collect::<Vec<_>>(),
                        "description": "Size of the container to provision"
                    }
                },
                "required": ["name", "flavor"]
            }),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (name, description, flavor) = validate_create_params(&params)?;

        let (tx, mut rx) = mpsc::channel::<String>(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        lxd_provision::create_lxd_agent(
            &self.neo4j, &self.lxd, &self.server_url, &self.project_id,
            &name, &description, flavor, tx,
        ).await?;

        Ok(format!("LXD agent '{name}' created successfully."))
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(CREATE_PREVIEW_CHARS).collect()
    }
}

pub struct DeleteAgentTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub lxd:        Option<Arc<LxdClient>>,
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for DeleteAgentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_agent".into(),
            description: "Permanently delete a registered agent machine from this project. \
                          If the agent is LXD-managed, its container is destroyed too. \
                          Use list_agents first to find the agent_id. This cannot be undone \
                          and must only be called after the user has explicitly confirmed."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "The agent machine ID (from list_agents)"
                    }
                },
                "required": ["agent_id"]
            }),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let agent_id = validate_delete_params(&params)?;
        delete_agent_core(&self.neo4j, self.lxd.as_ref(), &self.registry, &self.project_id, &agent_id)
            .await?;
        Ok(format!("Agent '{agent_id}' deleted."))
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(DELETE_PREVIEW_CHARS).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_params_requires_name() {
        let err = validate_create_params(&json!({ "flavor": "small" })).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn create_params_rejects_blank_name() {
        let err = validate_create_params(&json!({ "name": "   ", "flavor": "small" })).unwrap_err();
        assert!(err.to_string().contains("name"));
    }

    #[test]
    fn create_params_requires_flavor() {
        let err = validate_create_params(&json!({ "name": "x" })).unwrap_err();
        assert!(err.to_string().contains("flavor"));
    }

    #[test]
    fn create_params_rejects_unknown_flavor() {
        let err = validate_create_params(&json!({ "name": "x", "flavor": "huge" })).unwrap_err();
        assert!(err.to_string().contains("flavor"));
    }

    #[test]
    fn create_params_accepts_valid_input() {
        let (name, description, flavor) = validate_create_params(&json!({
            "name": "build-runner", "description": "ci box", "flavor": "medium"
        })).unwrap();
        assert_eq!(name, "build-runner");
        assert_eq!(description, "ci box");
        assert_eq!(flavor, Flavor::Medium);
    }

    #[test]
    fn create_params_description_defaults_to_empty() {
        let (_, description, _) = validate_create_params(&json!({ "name": "x", "flavor": "tiny" })).unwrap();
        assert_eq!(description, "");
    }

    #[test]
    fn create_params_trims_name() {
        let (name, _, _) = validate_create_params(&json!({ "name": "  spaced  ", "flavor": "tiny" })).unwrap();
        assert_eq!(name, "spaced");
    }

    #[test]
    fn delete_params_requires_agent_id() {
        assert!(validate_delete_params(&json!({})).is_err());
    }

    #[test]
    fn delete_params_rejects_blank_agent_id() {
        assert!(validate_delete_params(&json!({ "agent_id": "  " })).is_err());
    }

    #[test]
    fn delete_params_accepts_valid_input() {
        assert_eq!(validate_delete_params(&json!({ "agent_id": "abc" })).unwrap(), "abc");
    }
}
