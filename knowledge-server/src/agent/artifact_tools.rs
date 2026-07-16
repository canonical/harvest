use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::artifacts::handlers::{create_artifact, ArtifactKind};
use crate::llm::types::ToolDefinition;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

fn validate_artifact_params(params: &Value) -> Result<(String, ArtifactKind, String)> {
    let title = params["title"].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("title is required"))?
        .to_string();
    let kind_str = params["kind"].as_str()
        .ok_or_else(|| anyhow!("kind is required"))?;
    let kind = ArtifactKind::parse(kind_str)
        .ok_or_else(|| anyhow!("kind must be 'markdown' or 'pdf'"))?;
    let content = params["content"].as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("content is required"))?
        .to_string();
    Ok((title, kind, content))
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
            description: "Create a shareable document from Markdown content and save it \
                          as a project artifact the user can view and download. Use this \
                          when the user asks for a report, document, or file to keep or \
                          share, rather than just replying in chat."
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
                        "enum":        ["markdown", "pdf"],
                        "description": "The format to offer for download"
                    },
                    "content": {
                        "type":        "string",
                        "description": "The full document body, written in Markdown"
                    }
                },
                "required": ["title", "kind", "content"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (title, kind, content) = validate_artifact_params(&params)?;
        let created = create_artifact(
            &self.neo4j, &self.project_id, kind, &title, &content, "assistant",
        ).await?;
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
        let (title, kind, content) = validate_artifact_params(&json!({
            "title": "Report", "kind": "pdf", "content": "# Report"
        })).unwrap();
        assert_eq!(title, "Report");
        assert_eq!(kind, ArtifactKind::Pdf);
        assert_eq!(content, "# Report");
    }
}
