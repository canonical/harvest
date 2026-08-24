use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::artifacts::handlers::{
    create_artifact, get_artifact_in_project, update_artifact, validate_content_for_kind, ArtifactKind,
};
use crate::llm::types::ToolDefinition;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

fn validate_artifact_params(params: &Value) -> Result<(String, ArtifactKind, String, Option<String>)> {
    let title = params["title"].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("title is required"))?
        .to_string();
    let kind_str = params["kind"].as_str()
        .ok_or_else(|| anyhow!("kind is required"))?;
    let kind = ArtifactKind::parse(kind_str)
        .ok_or_else(|| anyhow!("kind must be 'markdown', 'pdf', 'terraform', 'terragrunt', or 'bash'"))?;
    let content = params["content"].as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("content is required"))?
        .to_string();
    validate_content_for_kind(kind, &content).map_err(|e| anyhow!(e))?;
    let artifact_id = params["artifact_id"].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    Ok((title, kind, content, artifact_id))
}

pub struct GenerateArtifactTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
    pub server_url: String,
}

#[async_trait]
impl Tool for GenerateArtifactTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "generate_artifact".into(),
            description: "Create a shareable document, bash script, or Terraform/Terragrunt module \
                          and save it as a project artifact the user can view, download, and (for \
                          terraform/terragrunt/bash) run on a connected agent. Use this when the \
                          user asks for a report, document, script, or infrastructure module to \
                          keep, share, or deploy, rather than just replying in chat. Pass \
                          artifact_id to revise an existing artifact in place instead of \
                          creating a new one."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type":        "string",
                        "description": "A short, descriptive title for the document"
                    },
                    "kind": {
                        "type":        "string",
                        "enum":        ["markdown", "pdf", "terraform", "terragrunt", "bash"],
                        "description": "The format to offer for download. Use 'terraform' or \
                                        'terragrunt' for infrastructure-as-code modules, 'bash' \
                                        for shell scripts, that can be run on an agent."
                    },
                    "content": {
                        "type":        "string",
                        "description": "For 'markdown'/'pdf', the full document body written in \
                                        Markdown. For 'terraform'/'terragrunt', a JSON object \
                                        mapping relative file path to file text, e.g. \
                                        {\"main.tf\": \"...\", \"variables.tf\": \"...\"}. For \
                                        'bash', the full shell script text."
                    },
                    "artifact_id": {
                        "type":        "string",
                        "description": "If set, revise this existing artifact in place instead of \
                                        creating a new one, keeping the same id and download link. \
                                        Required to redeploy a Terraform/Terragrunt artifact after \
                                        editing it, so the agent's on-disk state persists across \
                                        runs. The kind must match the existing artifact's kind."
                    }
                },
                "required": ["title", "kind", "content"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (title, kind, content, artifact_id) = validate_artifact_params(&params)?;
        let created = if let Some(existing_id) = artifact_id {
            let existing = get_artifact_in_project(&self.neo4j, &self.project_id, &existing_id)
                .await?
                .ok_or_else(|| anyhow!("artifact {existing_id} not found in this project"))?;
            let existing_kind = ArtifactKind::parse(existing["kind"].as_str().unwrap_or(""))
                .ok_or_else(|| anyhow!("artifact {existing_id} has an unknown kind"))?;
            update_artifact(&self.neo4j, &existing_id, existing_kind, kind, &title, &content).await?
        } else {
            create_artifact(
                &self.neo4j, &self.project_id, kind, &title, &content, "assistant",
            ).await?
        };
        let id = created["id"].as_str().unwrap_or_default();
        Ok(serde_json::to_string(&json!({
            "id":    id,
            "title": title,
            "kind":  kind.as_str(),
            "url":   format!("{}/#/artifacts/{}", self.server_url, id),
        }))?)
    }

    fn preview(&self, result: &str) -> String {
        let parsed: Value = serde_json::from_str(result).unwrap_or(Value::Null);
        let id    = parsed["id"].as_str().unwrap_or_default();
        let title = parsed["title"].as_str().unwrap_or("artifact");
        serde_json::to_string(&json!({
            "__type": "link",
            "href":   format!("#/artifacts/{id}"),
            "label":  format!("Open {title}"),
        })).unwrap_or_else(|_| result.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_params_requires_title() {
        let err = validate_artifact_params(&json!({ "kind": "markdown", "content": "hi" })).unwrap_err();
        assert!(err.to_string().contains("title"));
    }

    #[test]
    fn validate_params_requires_known_kind() {
        let err = validate_artifact_params(&json!({
            "title": "Report", "kind": "docx", "content": "hi"
        })).unwrap_err();
        assert!(err.to_string().contains("kind"));
    }

    #[test]
    fn validate_params_requires_content() {
        let err = validate_artifact_params(&json!({
            "title": "Report", "kind": "markdown", "content": ""
        })).unwrap_err();
        assert!(err.to_string().contains("content"));
    }

    #[test]
    fn validate_params_accepts_valid_input() {
        let (title, kind, content, artifact_id) = validate_artifact_params(&json!({
            "title": "Report", "kind": "pdf", "content": "# Report"
        })).unwrap();
        assert_eq!(title, "Report");
        assert_eq!(kind, ArtifactKind::Pdf);
        assert_eq!(content, "# Report");
        assert_eq!(artifact_id, None);
    }

    #[test]
    fn validate_params_accepts_terraform_kind_with_valid_bundle() {
        let (_, kind, _, _) = validate_artifact_params(&json!({
            "title": "Infra", "kind": "terraform", "content": r#"{"main.tf": "..."}"#
        })).unwrap();
        assert_eq!(kind, ArtifactKind::Terraform);
    }

    #[test]
    fn validate_params_rejects_terraform_kind_with_non_bundle_content() {
        let err = validate_artifact_params(&json!({
            "title": "Infra", "kind": "terraform", "content": "# just markdown"
        })).unwrap_err();
        assert!(err.to_string().contains("JSON"));
    }

    #[test]
    fn validate_params_passes_through_artifact_id() {
        let (_, _, _, artifact_id) = validate_artifact_params(&json!({
            "title": "Report", "kind": "markdown", "content": "hi", "artifact_id": "art-1"
        })).unwrap();
        assert_eq!(artifact_id, Some("art-1".to_string()));
    }
}
