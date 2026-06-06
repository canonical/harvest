use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use crate::machines::MachineRegistry;
use super::tool::Tool;

const LIST_AGENTS_PREVIEW_CHARS: usize = 500;
const RUN_COMMAND_PREVIEW_CHARS: usize = 2000;
const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 30;
const MAX_COMMAND_TIMEOUT_SECS: u64 = 300;

pub struct ListAgentsTool {
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_agents".into(),
            description: "List all connected agent machines in this project. \
                          Call this before run_command to discover available agent IDs and hostnames."
                .into(),
            parameters: json!({
                "type":       "object",
                "properties": {},
                "required":   []
            }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let agents = self.registry.agents_for_project(&self.project_id);
        Ok(serde_json::to_string_pretty(&agents)?)
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(LIST_AGENTS_PREVIEW_CHARS).collect()
    }
}

pub struct RunCommandTool {
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for RunCommandTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_command".into(),
            description: "Run a bash command on a connected agent machine in this project. \
                          Use list_agents first to discover agent IDs. \
                          Returns stdout, stderr, and exit code."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type":        "string",
                        "description": "The agent machine ID (from list_agents)"
                    },
                    "command": {
                        "type":        "string",
                        "description": "The bash command to execute"
                    },
                    "timeout_secs": {
                        "type":        "integer",
                        "description": "Timeout in seconds (default 30, max 300)",
                        "default":     DEFAULT_COMMAND_TIMEOUT_SECS
                    }
                },
                "required": ["agent_id", "command"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let agent_id = params["agent_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("agent_id is required"))?;
        let command = params["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("command is required"))?;
        let timeout_secs = params["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT_SECS)
            .min(MAX_COMMAND_TIMEOUT_SECS);

        let belongs = self.registry
            .agents
            .get(agent_id)
            .map(|a| a.project_id == self.project_id)
            .unwrap_or(false);

        if !belongs {
            anyhow::bail!("agent {agent_id} not found in this project");
        }

        match self.registry.execute(agent_id, command.to_string(), timeout_secs).await {
            Ok(r) => Ok(serde_json::to_string_pretty(&json!({
                "stdout":    r.stdout,
                "stderr":    r.stderr,
                "exit_code": r.exit_code,
            }))?),
            Err(e) => anyhow::bail!("command failed: {e}"),
        }
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(RUN_COMMAND_PREVIEW_CHARS).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> Arc<MachineRegistry> {
        MachineRegistry::new()
    }

    #[tokio::test]
    async fn list_agents_empty_project() {
        let tool = ListAgentsTool {
            registry:   make_registry(),
            project_id: "proj-1".into(),
        };
        let result = tool.execute(json!({})).await.unwrap();
        let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert!(arr.is_empty());
    }

    #[tokio::test]
    async fn run_command_unknown_agent_returns_error() {
        let tool = RunCommandTool {
            registry:   make_registry(),
            project_id: "proj-1".into(),
        };
        let result = tool.execute(json!({
            "agent_id": "nonexistent",
            "command":  "echo hi"
        })).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn run_command_cross_project_agent_rejected() {
        use crate::machines::{ConnectedAgent, ServerToAgent};
        use tokio::sync::mpsc;
        use chrono::Utc;

        let registry = make_registry();
        let (tx, _rx) = mpsc::channel::<ServerToAgent>(8);

        registry.agents.insert("agent-x".into(), ConnectedAgent {
            id:           "agent-x".into(),
            project_id:   "proj-2".into(),
            hostname:     "other-host".into(),
            connected_at: Utc::now(),
            sender:       tx,
        });

        let tool = RunCommandTool {
            registry:   Arc::clone(&registry),
            project_id: "proj-1".into(),
        };
        let result = tool.execute(json!({
            "agent_id": "agent-x",
            "command":  "echo pwned"
        })).await;
        assert!(result.is_err(), "cross-project execution must be rejected");
    }

    #[test]
    fn list_agents_definition_has_correct_name() {
        let tool = ListAgentsTool { registry: make_registry(), project_id: "p".into() };
        assert_eq!(tool.definition().name, "list_agents");
    }

    #[test]
    fn run_command_definition_has_correct_name() {
        let tool = RunCommandTool { registry: make_registry(), project_id: "p".into() };
        assert_eq!(tool.definition().name, "run_command");
    }
}
