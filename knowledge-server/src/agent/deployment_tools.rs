use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::artifacts::{bundle, handlers::{get_artifact_in_project, ArtifactKind}};
use crate::deployments::handlers::{load_runnable_bundle, ApiError};
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

}
