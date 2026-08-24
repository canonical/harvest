use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::artifacts::handlers::{ArtifactKind};
use crate::artifacts::{bundle, handlers::{get_artifact_in_project}};
use crate::deployments::handlers::{load_runnable_bundle, ApiError};
use crate::deployments::{StepNode, topological_sort};
use crate::llm::types::ToolDefinition;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

fn map_api_err((_, body): ApiError) -> anyhow::Error {
    anyhow!(body.0["error"].as_str().unwrap_or("deployment action failed").to_string())
}

fn required_str(params: &Value, key: &str) -> Result<String> {
    params[key].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("{key} is required"))
}

fn artifact_role_relationship(role: &str) -> Result<&'static str> {
    match role {
        "design"    => Ok("HAS_DESIGN_DOC"),
        "terraform" => Ok("HAS_TERRAFORM_BUNDLE"),
        "guide"     => Ok("HAS_GUIDE"),
        _           => Err(anyhow!("role must be 'design', 'terraform', or 'guide'")),
    }
}

fn role_accepts_kind(role: &str, kind: ArtifactKind) -> bool {
    match role {
        "design" | "guide" => matches!(kind, ArtifactKind::Markdown | ArtifactKind::Pdf),
        "terraform"         => matches!(kind, ArtifactKind::Terraform | ArtifactKind::Terragrunt),
        _                   => false,
    }
}

fn validate_link_params(params: &Value) -> Result<(String, String)> {
    let artifact_id = required_str(params, "artifact_id")?;
    let role = required_str(params, "role")?;
    artifact_role_relationship(&role)?;
    Ok((artifact_id, role))
}

fn validate_update_template_params(params: &Value) -> Result<String> {
    params["content"].as_str()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("content is required"))
}

pub fn action_valid_for_kind(kind: ArtifactKind, action: &str) -> bool {
    match (kind, action) {
        (ArtifactKind::Bash, "run") => true,
        (ArtifactKind::Terraform | ArtifactKind::Terragrunt, "plan" | "apply" | "destroy") => true,
        _ => false,
    }
}

#[derive(Debug, Clone)]
pub struct ParsedStep {
    pub artifact_id: String,
    pub action:      String,
    pub label:       String,
    pub depends_on:  Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct ParsedExecutionPlan {
    pub deploy_steps:  Vec<ParsedStep>,
    pub destroy_steps: Vec<ParsedStep>,
}

fn parse_step(entry: &Value) -> Result<ParsedStep> {
    let artifact_id = required_str(entry, "artifact_id")?;
    let action = required_str(entry, "action")?;
    if crate::deployments::StepAction::parse(&action).is_none() {
        anyhow::bail!("action must be 'run', 'plan', 'apply', or 'destroy'");
    }
    let label = entry.get("label")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| format!("{} {}", action, artifact_id));
    let depends_on: Vec<usize> = entry.get("depends_on")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter()
            .filter_map(|d| d.as_u64().map(|n| n as usize))
            .collect())
        .unwrap_or_default();
    Ok(ParsedStep { artifact_id, action, label, depends_on })
}

fn validate_step_list(steps: &[ParsedStep], label: &str) -> Result<()> {
    for (i, step) in steps.iter().enumerate() {
        for &dep in &step.depends_on {
            if dep >= steps.len() {
                anyhow::bail!("{} step {i} depends on out-of-bounds index {dep}", label);
            }
        }
    }
    let nodes: Vec<StepNode> = steps.iter()
        .map(|s| StepNode {
            id:         s.artifact_id.clone(),
            depends_on: s.depends_on.iter()
                .map(|&idx| steps[idx].artifact_id.clone())
                .collect(),
        })
        .collect();
    topological_sort(&nodes).map_err(|e| anyhow!("{} plan: {e}", label))?;
    Ok(())
}

pub fn validate_execution_plan_input(params: &Value) -> Result<ParsedExecutionPlan> {
    let deploy_steps = params.get("deploy_steps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("deploy_steps is required"))?;
    let destroy_steps = params.get("destroy_steps")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("destroy_steps is required"))?;

    let deploy_parsed: Vec<ParsedStep> = deploy_steps.iter()
        .map(parse_step)
        .collect::<Result<Vec<_>>>()?;
    let destroy_parsed: Vec<ParsedStep> = destroy_steps.iter()
        .map(parse_step)
        .collect::<Result<Vec<_>>>()?;

    validate_step_list(&deploy_parsed, "deploy")?;
    validate_step_list(&destroy_parsed, "destroy")?;

    Ok(ParsedExecutionPlan {
        deploy_steps:  deploy_parsed,
        destroy_steps: destroy_parsed,
    })
}

pub fn validate_terraform_destroy_coverage(plan: &ParsedExecutionPlan) -> Result<()> {
    let apply_artifacts: std::collections::HashSet<&str> = plan.deploy_steps.iter()
        .filter(|s| s.action == "apply")
        .map(|s| s.artifact_id.as_str())
        .collect();
    let destroy_artifacts: std::collections::HashSet<&str> = plan.destroy_steps.iter()
        .filter(|s| s.action == "destroy")
        .map(|s| s.artifact_id.as_str())
        .collect();
    let missing: Vec<&str> = apply_artifacts.difference(&destroy_artifacts).copied().collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "every terraform artifact with an 'apply' step in deploy_steps must have a 'destroy' step in destroy_steps — missing destroy for: {}",
            missing.join(", "),
        ))
    }
}

pub struct LinkDeploymentArtifactTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub project_id:    String,
    pub deployment_id: String,
}

#[async_trait]
impl Tool for LinkDeploymentArtifactTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "link_deployment_artifact".into(),
            description: "Associate an artifact (from generate_artifact) with this deployment's \
                          design document, Terraform/Terragrunt bundle, or deployment guide. \
                          Linking a new artifact for a role replaces the previous one for that role."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": {
                        "type":        "string",
                        "description": "The artifact ID returned by generate_artifact"
                    },
                    "role": {
                        "type":        "string",
                        "enum":        ["design", "terraform", "guide"],
                        "description": "'design' for the design document, 'terraform' for the \
                                        Terraform/Terragrunt bundle, 'guide' for the deployment guide"
                    }
                },
                "required": ["artifact_id", "role"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (artifact_id, role) = validate_link_params(&params)?;
        let relationship = artifact_role_relationship(&role)?;

        let artifact = get_artifact_in_project(&self.neo4j, &self.project_id, &artifact_id)
            .await?
            .ok_or_else(|| anyhow!("artifact {artifact_id} not found in this project"))?;
        let kind = ArtifactKind::parse(artifact["kind"].as_str().unwrap_or(""))
            .ok_or_else(|| anyhow!("artifact {artifact_id} has an unknown kind"))?;
        if !role_accepts_kind(&role, kind) {
            anyhow::bail!("a '{role}' artifact must be markdown/pdf (design, guide) or terraform/terragrunt (terraform)");
        }

        self.neo4j.query_read(
            &format!(
                "MATCH (:Project {{id: $pid}})-[:HAS_DEPLOYMENT]->(d:Deployment {{id: $did}})
                 OPTIONAL MATCH (d)-[old:{relationship}]->(:Artifact)
                 DELETE old
                 WITH d
                 MATCH (:Project {{id: $pid}})-[:HAS_ARTIFACT]->(a:Artifact {{id: $aid}})
                 CREATE (d)-[:{relationship}]->(a)"
            ),
            json!({ "pid": self.project_id, "did": self.deployment_id, "aid": artifact_id }),
        ).await?;

        Ok(serde_json::to_string(&json!({ "linked": true, "artifact_id": artifact_id, "role": role }))?)
    }
}

pub struct UpdateProductTemplateTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub group_id:      String,
    pub deployment_id: String,
}

#[async_trait]
impl Tool for UpdateProductTemplateTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_product_template".into(),
            description: "Save this deployment's design/prepare playbook as reusable knowledge for \
                          future deployments of the same product. If this deployment started from an \
                          existing template, replaces that template's content. Otherwise creates a new \
                          product template from this deployment."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": {
                        "type":        "string",
                        "description": "The full playbook content, written in Markdown, capturing \
                                        the design/prepare approach and any learnings"
                    }
                },
                "required": ["content"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let content = validate_update_template_params(&params)?;
        let now = chrono::Utc::now().to_rfc3339();

        let existing = self.neo4j.query_read(
            "MATCH (:Group {id: $gid})-[:HAS_TEMPLATE]->(t:ProductTemplate)<-[:USES_TEMPLATE]-(:Deployment {id: $did})
             RETURN t.id AS id",
            json!({ "gid": self.group_id, "did": self.deployment_id }),
        ).await?;

        if let Some(template_id) = existing.into_iter().next().and_then(|r| r["id"].as_str().map(str::to_string)) {
            self.neo4j.query_read(
                "MATCH (t:ProductTemplate {id: $tid}) SET t.content = $content, t.updated_at = $now",
                json!({ "tid": template_id, "content": content, "now": now }),
            ).await?;
            Ok(serde_json::to_string(&json!({ "template_id": template_id, "action": "updated" }))?)
        } else {
            let deployment = self.neo4j.query_read(
                "MATCH (:Group {id: $gid})-[:HAS_PROJECT]->(:Project)-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
                 RETURN d.name AS name",
                json!({ "gid": self.group_id, "did": self.deployment_id }),
            ).await?;
            let name = deployment.into_iter().next()
                .and_then(|r| r["name"].as_str().map(str::to_string))
                .unwrap_or_else(|| "Untitled product".to_string());

            let template_id = Uuid::new_v4().to_string();
            self.neo4j.query_read(
                "MATCH (g:Group {id: $gid}), (:Project)-[:HAS_DEPLOYMENT]->(d:Deployment {id: $did})
                 CREATE (t:ProductTemplate {
                     id: $tid, name: $name, description: '', content: $content,
                     created_by: 'assistant', created_at: $now, updated_at: $now
                 })
                 CREATE (g)-[:HAS_TEMPLATE]->(t)
                 CREATE (d)-[:USES_TEMPLATE]->(t)",
                json!({ "gid": self.group_id, "did": self.deployment_id, "tid": template_id, "name": name, "content": content, "now": now }),
            ).await?;
            Ok(serde_json::to_string(&json!({ "template_id": template_id, "action": "created" }))?)
        }
    }
}

pub struct ReadProvisionBundleTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub project_id:    String,
    pub deployment_id: String,
}

#[async_trait]
impl Tool for ReadProvisionBundleTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_provision_bundle".into(),
            description: "Read this deployment's current Terraform/Terragrunt bundle — every file \
                          path and its full content, as it is right now (before any proposed fix)."
                .into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let run = load_runnable_bundle(&self.neo4j, &self.project_id, &self.deployment_id).await.map_err(map_api_err)?;
        let files = bundle::parse_bundle(&run.artifact_content).map_err(|e| anyhow!(e))?;
        Ok(serde_json::to_string(&files)?)
    }
}

pub struct SetExecutionPlanTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub project_id:    String,
    pub deployment_id: String,
}

#[async_trait]
impl Tool for SetExecutionPlanTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "set_execution_plan".into(),
            description: "Define the ordered execution plan for this deployment's artifacts. Each \
                          step references an artifact (by id) and an action ('run' for bash \
                          scripts, 'plan'/'apply'/'destroy' for terraform/terragrunt). Steps can \
                          depend on other steps in the same phase (deploy or destroy) using \
                          0-based indices into depends_on. The deploy plan runs in topological \
                          order when the user triggers a deploy; the destroy plan runs when the \
                          user triggers a destroy. Call this after generating and linking all \
                          deployment artifacts."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "deploy_steps": {
                        "type": "array",
                        "description": "Ordered list of steps for deploying. Each step runs after \
                                        all steps it depends_on complete successfully.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "artifact_id": { "type": "string", "description": "The artifact to run" },
                                "action":      { "type": "string", "enum": ["run", "plan", "apply", "destroy"] },
                                "label":       { "type": "string", "description": "Short human-readable label for this step" },
                                "depends_on":  { "type": "array", "items": { "type": "integer" }, "description": "0-based indices of steps this step depends on" }
                            },
                            "required": ["artifact_id", "action"]
                        }
                    },
                    "destroy_steps": {
                        "type": "array",
                        "description": "Ordered list of steps for destroying. Separate from deploy \
                                        so the same artifact can have different actions (e.g. apply \
                                        in deploy, destroy in destroy).",
                        "items": {
                            "type": "object",
                            "properties": {
                                "artifact_id": { "type": "string" },
                                "action":      { "type": "string", "enum": ["run", "plan", "apply", "destroy"] },
                                "label":       { "type": "string" },
                                "depends_on":  { "type": "array", "items": { "type": "integer" } }
                            },
                            "required": ["artifact_id", "action"]
                        }
                    }
                },
                "required": ["deploy_steps", "destroy_steps"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let plan = validate_execution_plan_input(&params)?;
        validate_terraform_destroy_coverage(&plan)?;

        for step in plan.deploy_steps.iter().chain(plan.destroy_steps.iter()) {
            let artifact = get_artifact_in_project(&self.neo4j, &self.project_id, &step.artifact_id)
                .await?
                .ok_or_else(|| anyhow!("artifact {} not found in this project", step.artifact_id))?;
            let kind_str = artifact["kind"].as_str().unwrap_or("");
            let kind = ArtifactKind::parse(kind_str).unwrap_or(ArtifactKind::Markdown);
            if !action_valid_for_kind(kind, &step.action) {
                anyhow::bail!("action '{}' is not valid for artifact {} of kind '{}'", step.action, step.artifact_id, kind_str);
            }
        }

        let deploy_steps: Vec<crate::deployments::handlers::ParsedStepInput> = plan.deploy_steps.iter()
            .map(|s| crate::deployments::handlers::ParsedStepInput {
                artifact_id: s.artifact_id.clone(),
                action:      s.action.clone(),
                label:       s.label.clone(),
                depends_on:  s.depends_on.clone(),
            })
            .collect();
        let destroy_steps: Vec<crate::deployments::handlers::ParsedStepInput> = plan.destroy_steps.iter()
            .map(|s| crate::deployments::handlers::ParsedStepInput {
                artifact_id: s.artifact_id.clone(),
                action:      s.action.clone(),
                label:       s.label.clone(),
                depends_on: s.depends_on.clone(),
            })
            .collect();

        crate::deployments::handlers::set_execution_plan_core(
            &self.neo4j, &self.project_id, &self.deployment_id,
            &deploy_steps, &destroy_steps,
        ).await.map_err(|e| anyhow!(e.1.0["error"].as_str().unwrap_or("set_execution_plan failed").to_string()))?;

        Ok(serde_json::to_string(&json!({
            "deployed": true,
            "deploy_step_count":  deploy_steps.len(),
            "destroy_step_count": destroy_steps.len(),
        }))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_link_params_requires_artifact_id() {
        let e = validate_link_params(&json!({ "role": "design" })).unwrap_err();
        assert!(e.to_string().contains("artifact_id"));
    }

    #[test]
    fn validate_link_params_requires_role() {
        let e = validate_link_params(&json!({ "artifact_id": "a1" })).unwrap_err();
        assert!(e.to_string().contains("role"));
    }

    #[test]
    fn validate_link_params_rejects_unknown_role() {
        let e = validate_link_params(&json!({ "artifact_id": "a1", "role": "banana" })).unwrap_err();
        assert!(e.to_string().contains("role"));
    }

    #[test]
    fn validate_link_params_accepts_valid_input() {
        let (artifact_id, role) = validate_link_params(&json!({ "artifact_id": "a1", "role": "terraform" })).unwrap();
        assert_eq!(artifact_id, "a1");
        assert_eq!(role, "terraform");
    }

    #[test]
    fn role_accepts_kind_matches_design_and_guide_to_markdown_or_pdf() {
        assert!(role_accepts_kind("design", ArtifactKind::Markdown));
        assert!(role_accepts_kind("design", ArtifactKind::Pdf));
        assert!(!role_accepts_kind("design", ArtifactKind::Terraform));
        assert!(role_accepts_kind("guide", ArtifactKind::Markdown));
        assert!(!role_accepts_kind("guide", ArtifactKind::Terragrunt));
    }

    #[test]
    fn role_accepts_kind_matches_terraform_to_terraform_or_terragrunt() {
        assert!(role_accepts_kind("terraform", ArtifactKind::Terraform));
        assert!(role_accepts_kind("terraform", ArtifactKind::Terragrunt));
        assert!(!role_accepts_kind("terraform", ArtifactKind::Markdown));
    }

    #[test]
    fn validate_update_template_params_requires_non_empty_content() {
        let e = validate_update_template_params(&json!({ "content": "   " })).unwrap_err();
        assert!(e.to_string().contains("content"));
    }

    #[test]
    fn validate_update_template_params_accepts_valid_content() {
        let content = validate_update_template_params(&json!({ "content": "# Playbook" })).unwrap();
        assert_eq!(content, "# Playbook");
    }

    #[test]
    fn map_api_err_extracts_error_message_from_body() {
        use axum::{http::StatusCode, Json};
        let api_err: ApiError = (StatusCode::BAD_REQUEST, Json(json!({ "error": "nothing to destroy" })));
        let e = map_api_err(api_err);
        assert_eq!(e.to_string(), "nothing to destroy");
    }

    #[test]
    fn action_valid_for_kind_bash_only_allows_run() {
        assert!(action_valid_for_kind(ArtifactKind::Bash, "run"));
        assert!(!action_valid_for_kind(ArtifactKind::Bash, "plan"));
        assert!(!action_valid_for_kind(ArtifactKind::Bash, "apply"));
        assert!(!action_valid_for_kind(ArtifactKind::Bash, "destroy"));
    }

    #[test]
    fn action_valid_for_kind_terraform_allows_plan_apply_destroy() {
        assert!(action_valid_for_kind(ArtifactKind::Terraform, "plan"));
        assert!(action_valid_for_kind(ArtifactKind::Terraform, "apply"));
        assert!(action_valid_for_kind(ArtifactKind::Terraform, "destroy"));
        assert!(!action_valid_for_kind(ArtifactKind::Terraform, "run"));
    }

    #[test]
    fn action_valid_for_kind_terragrunt_allows_plan_apply_destroy() {
        assert!(action_valid_for_kind(ArtifactKind::Terragrunt, "plan"));
        assert!(action_valid_for_kind(ArtifactKind::Terragrunt, "apply"));
        assert!(action_valid_for_kind(ArtifactKind::Terragrunt, "destroy"));
        assert!(!action_valid_for_kind(ArtifactKind::Terragrunt, "run"));
    }

    #[test]
    fn action_valid_for_kind_markdown_allows_nothing() {
        assert!(!action_valid_for_kind(ArtifactKind::Markdown, "run"));
        assert!(!action_valid_for_kind(ArtifactKind::Markdown, "apply"));
    }

    #[test]
    fn validate_execution_plan_input_parses_linear_deploy_plan() {
        let input = json!({
            "deploy_steps": [
                {"artifact_id": "a1", "action": "run", "label": "Prep", "depends_on": []},
                {"artifact_id": "a2", "action": "apply", "label": "Apply", "depends_on": [0]}
            ],
            "destroy_steps": [
                {"artifact_id": "a2", "action": "destroy", "label": "Destroy", "depends_on": []}
            ]
        });
        let plan = validate_execution_plan_input(&input).unwrap();
        assert_eq!(plan.deploy_steps.len(), 2);
        assert_eq!(plan.deploy_steps[0].artifact_id, "a1");
        assert_eq!(plan.deploy_steps[0].action, "run");
        assert_eq!(plan.deploy_steps[0].label, "Prep");
        assert!(plan.deploy_steps[0].depends_on.is_empty());
        assert_eq!(plan.deploy_steps[1].depends_on, vec![0]);
        assert_eq!(plan.destroy_steps.len(), 1);
        assert_eq!(plan.destroy_steps[0].action, "destroy");
    }

    #[test]
    fn validate_execution_plan_input_rejects_unknown_action() {
        let input = json!({
            "deploy_steps": [
                {"artifact_id": "a1", "action": "delete", "label": "X", "depends_on": []}
            ],
            "destroy_steps": []
        });
        assert!(validate_execution_plan_input(&input).is_err());
    }

    #[test]
    fn validate_execution_plan_input_rejects_depends_on_out_of_bounds() {
        let input = json!({
            "deploy_steps": [
                {"artifact_id": "a1", "action": "run", "label": "X", "depends_on": [5]}
            ],
            "destroy_steps": []
        });
        assert!(validate_execution_plan_input(&input).is_err());
    }

    #[test]
    fn validate_execution_plan_input_rejects_cycle_in_deploy_steps() {
        let input = json!({
            "deploy_steps": [
                {"artifact_id": "a1", "action": "run", "label": "X", "depends_on": [1]},
                {"artifact_id": "a2", "action": "run", "label": "Y", "depends_on": [0]}
            ],
            "destroy_steps": []
        });
        assert!(validate_execution_plan_input(&input).is_err());
    }

    #[test]
    fn validate_execution_plan_input_rejects_cycle_in_destroy_steps() {
        let input = json!({
            "deploy_steps": [],
            "destroy_steps": [
                {"artifact_id": "a1", "action": "destroy", "label": "X", "depends_on": [1]},
                {"artifact_id": "a2", "action": "destroy", "label": "Y", "depends_on": [0]}
            ]
        });
        assert!(validate_execution_plan_input(&input).is_err());
    }

    #[test]
    fn validate_execution_plan_input_accepts_empty_plan() {
        let input = json!({"deploy_steps": [], "destroy_steps": []});
        let plan = validate_execution_plan_input(&input).unwrap();
        assert!(plan.deploy_steps.is_empty());
        assert!(plan.destroy_steps.is_empty());
    }

    #[test]
    fn validate_execution_plan_input_rejects_missing_artifact_id() {
        let input = json!({
            "deploy_steps": [{"action": "run", "label": "X", "depends_on": []}],
            "destroy_steps": []
        });
        assert!(validate_execution_plan_input(&input).is_err());
    }

    #[test]
    fn validate_execution_plan_input_uses_default_label_when_missing() {
        let input = json!({
            "deploy_steps": [{"artifact_id": "a1", "action": "run", "depends_on": []}],
            "destroy_steps": []
        });
        let plan = validate_execution_plan_input(&input).unwrap();
        assert_eq!(plan.deploy_steps[0].label, "run a1");
    }

    #[test]
    fn validate_execution_plan_input_rejects_depends_on_in_destroy_referencing_deploy() {
        let input = json!({
            "deploy_steps": [{"artifact_id": "a1", "action": "run", "label": "X", "depends_on": []}],
            "destroy_steps": [{"artifact_id": "a2", "action": "destroy", "label": "Y", "depends_on": [0]}]
        });
        assert!(validate_execution_plan_input(&input).is_err());
    }

    #[test]
    fn validate_terraform_destroy_coverage_passes_when_apply_has_destroy() {
        let plan = ParsedExecutionPlan {
            deploy_steps:  vec![
                ParsedStep { artifact_id: "a1".into(), action: "apply".into(),   label: "Apply".into(),  depends_on: vec![] },
            ],
            destroy_steps: vec![
                ParsedStep { artifact_id: "a1".into(), action: "destroy".into(), label: "Destroy".into(), depends_on: vec![] },
            ],
        };
        assert!(validate_terraform_destroy_coverage(&plan).is_ok());
    }

    #[test]
    fn validate_terraform_destroy_coverage_passes_when_no_apply_steps() {
        let plan = ParsedExecutionPlan {
            deploy_steps:  vec![
                ParsedStep { artifact_id: "a1".into(), action: "run".into(), label: "Prep".into(), depends_on: vec![] },
            ],
            destroy_steps: vec![],
        };
        assert!(validate_terraform_destroy_coverage(&plan).is_ok());
    }

    #[test]
    fn validate_terraform_destroy_coverage_passes_when_empty_plan() {
        let plan = ParsedExecutionPlan {
            deploy_steps:  vec![],
            destroy_steps: vec![],
        };
        assert!(validate_terraform_destroy_coverage(&plan).is_ok());
    }

    #[test]
    fn validate_terraform_destroy_coverage_fails_when_apply_missing_destroy() {
        let plan = ParsedExecutionPlan {
            deploy_steps:  vec![
                ParsedStep { artifact_id: "a1".into(), action: "apply".into(), label: "Apply".into(), depends_on: vec![] },
            ],
            destroy_steps: vec![],
        };
        let e = validate_terraform_destroy_coverage(&plan).unwrap_err();
        assert!(e.to_string().contains("a1"));
        assert!(e.to_string().contains("destroy"));
    }

    #[test]
    fn validate_terraform_destroy_coverage_fails_when_one_of_two_applies_missing_destroy() {
        let plan = ParsedExecutionPlan {
            deploy_steps:  vec![
                ParsedStep { artifact_id: "a1".into(), action: "apply".into(), label: "Apply 1".into(), depends_on: vec![] },
                ParsedStep { artifact_id: "a2".into(), action: "apply".into(), label: "Apply 2".into(), depends_on: vec![] },
            ],
            destroy_steps: vec![
                ParsedStep { artifact_id: "a1".into(), action: "destroy".into(), label: "Destroy 1".into(), depends_on: vec![] },
            ],
        };
        let e = validate_terraform_destroy_coverage(&plan).unwrap_err();
        assert!(e.to_string().contains("a2"));
        assert!(!e.to_string().contains("a1"));
    }

    #[test]
    fn validate_terraform_destroy_coverage_ignores_plan_and_run_actions() {
        let plan = ParsedExecutionPlan {
            deploy_steps:  vec![
                ParsedStep { artifact_id: "a1".into(), action: "plan".into(), label: "Plan".into(), depends_on: vec![] },
                ParsedStep { artifact_id: "a2".into(), action: "run".into(),  label: "Run".into(),  depends_on: vec![] },
            ],
            destroy_steps: vec![],
        };
        assert!(validate_terraform_destroy_coverage(&plan).is_ok());
    }

}
