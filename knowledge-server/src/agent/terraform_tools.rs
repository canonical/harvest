use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::artifacts::{bundle, handlers::{get_artifact_in_project, ArtifactKind}};
use crate::llm::types::ToolDefinition;
use crate::machines::{MachineRegistry, TerraformAction, TerraformFlavor};
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

const RUN_TERRAFORM_PREVIEW_CHARS: usize = 2000;
const DEFAULT_TERRAFORM_TIMEOUT_SECS: u64 = 300;
const MAX_TERRAFORM_TIMEOUT_SECS: u64 = 1800;

fn required_str(params: &Value, key: &str) -> Result<String> {
    params[key].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("{key} is required"))
}

fn validate_run_params(params: &Value) -> Result<(String, String, u64)> {
    let artifact_id = required_str(params, "artifact_id")?;
    let agent_id = required_str(params, "agent_id")?;
    let timeout_secs = params["timeout_secs"]
        .as_u64()
        .unwrap_or(DEFAULT_TERRAFORM_TIMEOUT_SECS)
        .min(MAX_TERRAFORM_TIMEOUT_SECS);
    Ok((artifact_id, agent_id, timeout_secs))
}

fn agent_belongs_to_project(registry: &MachineRegistry, agent_id: &str, project_id: &str) -> bool {
    registry.agents
        .get(agent_id)
        .map(|a| a.project_id == project_id)
        .unwrap_or(false)
}

async fn run_terraform(
    neo4j: &Neo4jClient,
    registry: &MachineRegistry,
    project_id: &str,
    params: Value,
    action: TerraformAction,
) -> Result<String> {
    let (artifact_id, agent_id, timeout_secs) = validate_run_params(&params)?;

    if !agent_belongs_to_project(registry, &agent_id, project_id) {
        anyhow::bail!("agent {agent_id} not found in this project");
    }

    let artifact = get_artifact_in_project(neo4j, project_id, &artifact_id)
        .await?
        .ok_or_else(|| anyhow!("artifact {artifact_id} not found in this project"))?;

    let flavor = match ArtifactKind::parse(artifact["kind"].as_str().unwrap_or("")) {
        Some(ArtifactKind::Terraform)  => TerraformFlavor::Terraform,
        Some(ArtifactKind::Terragrunt) => TerraformFlavor::Terragrunt,
        _ => anyhow::bail!("artifact {artifact_id} is not a terraform or terragrunt artifact"),
    };

    let files = bundle::parse_bundle(artifact["content"].as_str().unwrap_or(""))
        .map_err(|e| anyhow!(e))?;

    match registry.execute_terraform(&agent_id, artifact_id, flavor, action, files, timeout_secs).await {
        Ok(r) => Ok(serde_json::to_string_pretty(&json!({
            "stdout":    r.stdout,
            "stderr":    r.stderr,
            "exit_code": r.exit_code,
        }))?),
        Err(e) => anyhow::bail!("terraform run failed: {e}"),
    }
}

fn terraform_run_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "artifact_id": {
                "type":        "string",
                "description": "The terraform/terragrunt artifact ID (from generate_artifact or the artifacts list)"
            },
            "agent_id": {
                "type":        "string",
                "description": "The agent machine ID (from list_agents)"
            },
            "timeout_secs": {
                "type":        "integer",
                "description": "Timeout in seconds (default 300, max 1800)",
                "default":     DEFAULT_TERRAFORM_TIMEOUT_SECS
            }
        },
        "required": ["artifact_id", "agent_id"]
    })
}

pub struct RunTerraformPlanTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for RunTerraformPlanTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_terraform_plan".into(),
            description: "Run 'terraform plan' (or 'terragrunt plan') for a Terraform/Terragrunt \
                          artifact on a connected agent machine. Read-only: shows what would \
                          change without applying it. Missing terraform/terragrunt binaries are \
                          installed automatically on the agent. Use list_agents first to find the \
                          agent_id."
                .into(),
            parameters: terraform_run_parameters(),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        run_terraform(&self.neo4j, &self.registry, &self.project_id, params, TerraformAction::Plan).await
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(RUN_TERRAFORM_PREVIEW_CHARS).collect()
    }
}

pub struct RunTerraformApplyTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for RunTerraformApplyTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_terraform_apply".into(),
            description: "Run 'terraform apply' (or 'terragrunt apply') for a Terraform/Terragrunt \
                          artifact on a connected agent machine. This creates, changes, or updates \
                          real infrastructure and must only be called after the user has explicitly \
                          confirmed."
                .into(),
            parameters: terraform_run_parameters(),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        run_terraform(&self.neo4j, &self.registry, &self.project_id, params, TerraformAction::Apply).await
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(RUN_TERRAFORM_PREVIEW_CHARS).collect()
    }
}

pub struct RunTerraformDestroyTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for RunTerraformDestroyTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_terraform_destroy".into(),
            description: "Run 'terraform destroy' (or 'terragrunt destroy') for a Terraform/Terragrunt \
                          artifact on a connected agent machine. This permanently destroys real \
                          infrastructure and cannot be undone; must only be called after the user \
                          has explicitly confirmed."
                .into(),
            parameters: terraform_run_parameters(),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        run_terraform(&self.neo4j, &self.registry, &self.project_id, params, TerraformAction::Destroy).await
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(RUN_TERRAFORM_PREVIEW_CHARS).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> Arc<MachineRegistry> {
        MachineRegistry::new()
    }

    #[test]
    fn validate_run_params_requires_artifact_id() {
        let e = validate_run_params(&json!({ "agent_id": "a1" })).unwrap_err();
        assert!(e.to_string().contains("artifact_id"));
    }

    #[test]
    fn validate_run_params_requires_agent_id() {
        let e = validate_run_params(&json!({ "artifact_id": "art1" })).unwrap_err();
        assert!(e.to_string().contains("agent_id"));
    }

    #[test]
    fn validate_run_params_defaults_timeout() {
        let (_, _, timeout) = validate_run_params(&json!({ "artifact_id": "art1", "agent_id": "a1" })).unwrap();
        assert_eq!(timeout, DEFAULT_TERRAFORM_TIMEOUT_SECS);
    }

    #[test]
    fn validate_run_params_clamps_timeout_to_max() {
        let (_, _, timeout) = validate_run_params(&json!({
            "artifact_id": "art1", "agent_id": "a1", "timeout_secs": 999_999
        })).unwrap();
        assert_eq!(timeout, MAX_TERRAFORM_TIMEOUT_SECS);
    }

    #[test]
    fn agent_belongs_to_project_false_for_unknown_agent() {
        let registry = make_registry();
        assert!(!agent_belongs_to_project(&registry, "nonexistent", "proj-1"));
    }

    #[test]
    fn agent_belongs_to_project_false_for_cross_project_agent() {
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

        assert!(!agent_belongs_to_project(&registry, "agent-x", "proj-1"));
    }

    #[test]
    fn agent_belongs_to_project_true_for_matching_project() {
        use crate::machines::{ConnectedAgent, ServerToAgent};
        use tokio::sync::mpsc;
        use chrono::Utc;

        let registry = make_registry();
        let (tx, _rx) = mpsc::channel::<ServerToAgent>(8);
        registry.agents.insert("agent-x".into(), ConnectedAgent {
            id:           "agent-x".into(),
            project_id:   "proj-1".into(),
            hostname:     "host".into(),
            connected_at: Utc::now(),
            sender:       tx,
        });

        assert!(agent_belongs_to_project(&registry, "agent-x", "proj-1"));
    }
}
