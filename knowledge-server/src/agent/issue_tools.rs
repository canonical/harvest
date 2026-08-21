use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::artifacts::bundle;
use crate::deployments::FailedRun;
use crate::issues;
use crate::llm::types::ToolDefinition;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

fn required_str(params: &Value, key: &str) -> Result<String> {
    params[key].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("{key} is required"))
}

pub struct ListDeploymentIssuesTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub project_id:    String,
    pub deployment_id: String,
}

#[async_trait]
impl Tool for ListDeploymentIssuesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_deployment_issues".into(),
            description: "List the issues already tracked for this deployment (id, title, \
                          description, status, fingerprint), so you can decide whether the current \
                          failure matches one of them."
                .into(),
            parameters: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let issues = issues::list_open_issues_for_deployment(&self.neo4j, &self.project_id, &self.deployment_id).await?;
        Ok(serde_json::to_string(&issues)?)
    }
}

#[derive(Debug)]
enum CreateOrLinkParams {
    Create { title: String, description: String, fingerprint: String },
    LinkExisting { issue_id: String },
}

fn validate_create_or_link_params(params: &Value) -> Result<CreateOrLinkParams> {
    match required_str(params, "action")?.as_str() {
        "create" => Ok(CreateOrLinkParams::Create {
            title:       required_str(params, "title")?,
            description: required_str(params, "description")?,
            fingerprint: required_str(params, "fingerprint")?,
        }),
        "link_existing" => Ok(CreateOrLinkParams::LinkExisting {
            issue_id: required_str(params, "issue_id")?,
        }),
        _ => Err(anyhow!("action must be 'create' or 'link_existing'")),
    }
}

fn create_or_link_issue_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type":        "string",
                "enum":        ["create", "link_existing"],
                "description": "'create' if this failure doesn't match any existing issue, \
                                'link_existing' if it's the same root cause as one already listed"
            },
            "title": {
                "type":        "string",
                "description": "Short issue title (required when action is 'create')"
            },
            "description": {
                "type":        "string",
                "description": "Markdown description of the problem, with relevant log/code \
                                snippets (required when action is 'create')"
            },
            "fingerprint": {
                "type":        "string",
                "description": "A short signature of the root cause, used to help match future \
                                failures against this issue (required when action is 'create')"
            },
            "issue_id": {
                "type":        "string",
                "description": "The id of the existing issue this run's failure matches \
                                (required when action is 'link_existing')"
            }
        },
        "required": ["action"]
    })
}

pub struct CreateOrLinkIssueTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub project_id:    String,
    pub deployment_id: String,
    pub run:           FailedRun,
    pub created_by:    String,
}

#[async_trait]
impl Tool for CreateOrLinkIssueTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_or_link_issue".into(),
            description: "Record this run's failure as an issue: either create a new issue, or, if \
                          it's the same root cause as an existing issue from list_deployment_issues, \
                          link this run to that issue instead. Call this once per distinct failure \
                          found in the run's output."
                .into(),
            parameters: create_or_link_issue_parameters(),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        match validate_create_or_link_params(&params)? {
            CreateOrLinkParams::Create { title, description, fingerprint } => {
                let issue_id = issues::create_issue(
                    &self.neo4j, &self.project_id, &self.deployment_id, &self.run,
                    &title, &description, &fingerprint, &self.created_by,
                ).await?;
                Ok(serde_json::to_string(&json!({ "created": true, "issue_id": issue_id }))?)
            }
            CreateOrLinkParams::LinkExisting { issue_id } => {
                let current = issues::issue_status(&self.neo4j, &self.project_id, &issue_id).await?
                    .ok_or_else(|| anyhow!("issue {issue_id} not found in this project"))?;
                issues::link_run_to_issue(&self.neo4j, &self.project_id, &issue_id, &self.run).await?;
                if issues::is_regression_reopen(current) {
                    issues::reopen_for_regression(&self.neo4j, &self.project_id, &issue_id, current, &self.run).await?;
                }
                Ok(serde_json::to_string(&json!({ "linked": true, "issue_id": issue_id }))?)
            }
        }
    }
}

pub struct ProposeIssueSolutionTool {
    pub neo4j:         Arc<Neo4jClient>,
    pub project_id:    String,
    pub deployment_id: String,
    pub issue_id:      String,
}

fn propose_issue_solution_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "explanation": {
                "type":        "string",
                "description": "A short explanation of what was wrong and what this change fixes"
            },
            "files": {
                "type":        "string",
                "description": "A JSON object, encoded as a string, mapping every file path in the \
                                corrected bundle to its full content, e.g. \
                                {\"main.tf\": \"...\", \"variables.tf\": \"...\"}. Include every \
                                file, whether changed or not."
            }
        },
        "required": ["explanation", "files"]
    })
}

#[async_trait]
impl Tool for ProposeIssueSolutionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "propose_issue_solution".into(),
            description: "Propose a corrected version of this deployment's Terraform/Terragrunt \
                          bundle as the fix for this issue. This does not apply anything — it stages \
                          a diff for the user to review and apply themselves. Always include every \
                          file (changed or not), not just the ones you edited."
                .into(),
            parameters: propose_issue_solution_parameters(),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let explanation = required_str(&params, "explanation")?;
        let files_str = required_str(&params, "files")?;
        let files: BTreeMap<String, String> = serde_json::from_str(&files_str)
            .map_err(|e| anyhow!("files must be a JSON object mapping path to content: {e}"))?;
        bundle::validate_bundle(&files).map_err(|e| anyhow!(e))?;

        crate::deployments::handlers::load_runnable_bundle(&self.neo4j, &self.project_id, &self.deployment_id)
            .await
            .map_err(|(_, body)| anyhow!(body.0["error"].as_str().unwrap_or("deployment action failed").to_string()))?;

        issues::set_proposed_solution(
            &self.neo4j, &self.project_id, &self.issue_id, &explanation, &files_str, files.len(),
        ).await?;

        Ok(serde_json::to_string(&json!({
            "proposed":    true,
            "explanation": explanation,
            "file_count":  files.len(),
        }))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_create_or_link_params_requires_action() {
        let e = validate_create_or_link_params(&json!({})).unwrap_err();
        assert!(e.to_string().contains("action"));
    }

    #[test]
    fn validate_create_or_link_params_rejects_unknown_action() {
        let e = validate_create_or_link_params(&json!({ "action": "banana" })).unwrap_err();
        assert!(e.to_string().contains("action"));
    }

    #[test]
    fn validate_create_or_link_params_create_requires_title() {
        let e = validate_create_or_link_params(&json!({
            "action": "create", "description": "d", "fingerprint": "f"
        })).unwrap_err();
        assert!(e.to_string().contains("title"));
    }

    #[test]
    fn validate_create_or_link_params_create_requires_description() {
        let e = validate_create_or_link_params(&json!({
            "action": "create", "title": "t", "fingerprint": "f"
        })).unwrap_err();
        assert!(e.to_string().contains("description"));
    }

    #[test]
    fn validate_create_or_link_params_create_requires_fingerprint() {
        let e = validate_create_or_link_params(&json!({
            "action": "create", "title": "t", "description": "d"
        })).unwrap_err();
        assert!(e.to_string().contains("fingerprint"));
    }

    #[test]
    fn validate_create_or_link_params_create_accepts_valid_input() {
        let parsed = validate_create_or_link_params(&json!({
            "action": "create", "title": "t", "description": "d", "fingerprint": "f"
        })).unwrap();
        assert!(matches!(parsed, CreateOrLinkParams::Create { .. }));
    }

    #[test]
    fn validate_create_or_link_params_link_existing_requires_issue_id() {
        let e = validate_create_or_link_params(&json!({ "action": "link_existing" })).unwrap_err();
        assert!(e.to_string().contains("issue_id"));
    }

    #[test]
    fn validate_create_or_link_params_link_existing_accepts_valid_input() {
        let parsed = validate_create_or_link_params(&json!({
            "action": "link_existing", "issue_id": "i1"
        })).unwrap();
        assert!(matches!(parsed, CreateOrLinkParams::LinkExisting { issue_id } if issue_id == "i1"));
    }

    #[test]
    fn create_or_link_issue_parameters_requires_only_action() {
        let params = create_or_link_issue_parameters();
        assert_eq!(params["required"], json!(["action"]));
    }

    #[test]
    fn propose_issue_solution_parameters_types_files_as_a_string() {
        let params = propose_issue_solution_parameters();
        assert_eq!(params["properties"]["files"]["type"], "string");
    }

    #[test]
    fn propose_issue_solution_rejects_missing_explanation() {
        let e = required_str(&json!({ "files": "{}" }), "explanation").unwrap_err();
        assert!(e.to_string().contains("explanation"));
    }

    #[test]
    fn propose_issue_solution_rejects_invalid_files() {
        let mut files = BTreeMap::new();
        files.insert("../etc/passwd".to_string(), "x".to_string());
        assert!(bundle::validate_bundle(&files).is_err());
    }

    #[test]
    fn propose_issue_solution_files_param_parses_from_json_string() {
        let files_str = json!({ "main.tf": "x" }).to_string();
        let files: BTreeMap<String, String> = serde_json::from_str(&files_str).unwrap();
        assert_eq!(files.get("main.tf"), Some(&"x".to_string()));
    }
}
