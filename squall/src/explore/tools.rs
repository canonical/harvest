use anyhow::{bail, Result};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::harvest::Client;
use crate::llm::types::ToolDefinition;

#[async_trait]
pub trait ExplorationTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, params: Value) -> Result<String>;
}

const HARVEST_TOOL_NAMES: &[&str] = &[
    "list_groups",
    "create_project",
    "get_project",
    "create_conversation",
    "send_chat_message",
    "create_artifact",
    "update_artifact",
    "get_artifact",
    "get_deployment",
    "generate_design",
    "add_context_artifact",
    "list_deployment_runs",
];

const INFRA_TOOL_NAMES: &[&str] = &["deploy", "redeploy", "destroy"];

pub struct HarvestTool {
    client: Arc<Client>,
    name: &'static str,
}

impl HarvestTool {
    pub fn build_toolset(client: Arc<Client>, allow_infra: bool) -> Vec<Arc<dyn ExplorationTool>> {
        let mut names: Vec<&'static str> = HARVEST_TOOL_NAMES.to_vec();
        if allow_infra {
            names.extend_from_slice(INFRA_TOOL_NAMES);
        }
        names
            .into_iter()
            .map(|name| Arc::new(HarvestTool { client: client.clone(), name }) as Arc<dyn ExplorationTool>)
            .collect()
    }
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required })
}

fn pretty(value: Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(&value)?)
}

fn field<T: Default>(params: &mut Value, key: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    match params.get_mut(key).map(Value::take) {
        Some(v) if !v.is_null() => Ok(serde_json::from_value(v)?),
        _ => Ok(T::default()),
    }
}

fn require<T>(params: &mut Value, key: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    match params.get_mut(key).map(Value::take) {
        Some(v) if !v.is_null() => Ok(serde_json::from_value(v)?),
        _ => bail!("missing required parameter: {key}"),
    }
}

#[async_trait]
impl ExplorationTool for HarvestTool {
    fn definition(&self) -> ToolDefinition {
        let (description, parameters) = match self.name {
            "list_groups" => (
                "List the groups this account belongs to, to find a group_id for create_project.",
                schema(json!({}), &[]),
            ),
            "create_project" => (
                "Create a new Harvest project. Every project automatically gets one Deployment.",
                schema(
                    json!({
                        "name": {"type": "string"},
                        "description": {"type": "string"},
                        "group_id": {"type": "string"},
                    }),
                    &["name", "group_id"],
                ),
            ),
            "get_project" => (
                "Fetch a project by id.",
                schema(json!({"project_id": {"type": "string"}}), &["project_id"]),
            ),
            "create_conversation" => (
                "Start a new chat conversation inside a project.",
                schema(
                    json!({"project_id": {"type": "string"}, "title": {"type": "string"}}),
                    &["project_id"],
                ),
            ),
            "send_chat_message" => (
                "Send a chat message to the project's agent and get its answer synchronously.",
                schema(
                    json!({
                        "project_id": {"type": "string"},
                        "query": {"type": "string"},
                        "conversation_id": {"type": "string"},
                    }),
                    &["project_id", "query"],
                ),
            ),
            "create_artifact" => (
                "Create a new artifact in a project. kind is one of markdown, pdf, terraform, terragrunt, bash. For terraform/terragrunt, content must be a JSON object mapping filename to file content.",
                schema(
                    json!({
                        "project_id": {"type": "string"},
                        "title": {"type": "string"},
                        "kind": {"type": "string"},
                        "content": {},
                    }),
                    &["project_id", "title", "kind", "content"],
                ),
            ),
            "update_artifact" => (
                "Overwrite an existing artifact's title and content. kind cannot change.",
                schema(
                    json!({
                        "artifact_id": {"type": "string"},
                        "title": {"type": "string"},
                        "kind": {"type": "string"},
                        "content": {},
                    }),
                    &["artifact_id", "title", "kind", "content"],
                ),
            ),
            "get_artifact" => (
                "Fetch an artifact by id.",
                schema(json!({"artifact_id": {"type": "string"}}), &["artifact_id"]),
            ),
            "get_deployment" => (
                "Fetch a project's single auto-created deployment, including its id and infra_state.",
                schema(json!({"project_id": {"type": "string"}}), &["project_id"]),
            ),
            "generate_design" => (
                "Ask Harvest's agent to generate or regenerate the design document for a deployment.",
                schema(
                    json!({
                        "project_id": {"type": "string"},
                        "deployment_id": {"type": "string"},
                        "artifact_ids": {"type": "array", "items": {"type": "string"}},
                    }),
                    &["project_id", "deployment_id"],
                ),
            ),
            "add_context_artifact" => (
                "Create a new artifact and attach it to a deployment as context.",
                schema(
                    json!({
                        "project_id": {"type": "string"},
                        "deployment_id": {"type": "string"},
                        "title": {"type": "string"},
                        "kind": {"type": "string"},
                        "content": {},
                    }),
                    &["project_id", "deployment_id", "title", "kind", "content"],
                ),
            ),
            "list_deployment_runs" => (
                "List past deploy/destroy runs for a deployment, to check the outcome of an infra action.",
                schema(
                    json!({"project_id": {"type": "string"}, "deployment_id": {"type": "string"}}),
                    &["project_id", "deployment_id"],
                ),
            ),
            "deploy" => (
                "Run terraform apply against a deployment's real infrastructure. This has real-world consequences: it can provision real cloud resources and incur real cost. Use deliberately.",
                schema(
                    json!({"project_id": {"type": "string"}, "deployment_id": {"type": "string"}}),
                    &["project_id", "deployment_id"],
                ),
            ),
            "redeploy" => (
                "Re-run terraform apply against a deployment's existing real infrastructure. This has real-world consequences. Use deliberately.",
                schema(
                    json!({"project_id": {"type": "string"}, "deployment_id": {"type": "string"}}),
                    &["project_id", "deployment_id"],
                ),
            ),
            "destroy" => (
                "Run terraform destroy against a deployment's real infrastructure, tearing it down. This has real-world consequences and cannot be undone. Use deliberately.",
                schema(
                    json!({"project_id": {"type": "string"}, "deployment_id": {"type": "string"}}),
                    &["project_id", "deployment_id"],
                ),
            ),
            other => unreachable!("unknown harvest tool: {other}"),
        };
        ToolDefinition { name: self.name.to_string(), description: description.to_string(), parameters }
    }

    async fn execute(&self, mut params: Value) -> Result<String> {
        match self.name {
            "list_groups" => pretty(json!(self.client.list_groups().await?)),
            "create_project" => {
                let name: String = require(&mut params, "name")?;
                let description: Option<String> = field(&mut params, "description")?;
                let group_id: String = require(&mut params, "group_id")?;
                pretty(self.client.create_project(&name, description.as_deref(), &group_id).await?)
            }
            "get_project" => {
                let project_id: String = require(&mut params, "project_id")?;
                pretty(self.client.get_project(&project_id).await?)
            }
            "create_conversation" => {
                let project_id: String = require(&mut params, "project_id")?;
                let title: Option<String> = field(&mut params, "title")?;
                pretty(self.client.create_conversation(&project_id, title.as_deref()).await?)
            }
            "send_chat_message" => {
                let project_id: String = require(&mut params, "project_id")?;
                let query: String = require(&mut params, "query")?;
                let conversation_id: Option<String> = field(&mut params, "conversation_id")?;
                let resp = self.client.send_chat_message(&project_id, &query, conversation_id.as_deref()).await?;
                pretty(serde_json::to_value(resp)?)
            }
            "create_artifact" => {
                let project_id: String = require(&mut params, "project_id")?;
                let title: String = require(&mut params, "title")?;
                let kind: String = require(&mut params, "kind")?;
                let content: Value = require(&mut params, "content")?;
                pretty(self.client.create_artifact(&project_id, &title, &kind, &content).await?)
            }
            "update_artifact" => {
                let artifact_id: String = require(&mut params, "artifact_id")?;
                let title: String = require(&mut params, "title")?;
                let kind: String = require(&mut params, "kind")?;
                let content: Value = require(&mut params, "content")?;
                pretty(self.client.update_artifact(&artifact_id, &title, &kind, &content).await?)
            }
            "get_artifact" => {
                let artifact_id: String = require(&mut params, "artifact_id")?;
                pretty(self.client.get_artifact(&artifact_id).await?)
            }
            "get_deployment" => {
                let project_id: String = require(&mut params, "project_id")?;
                pretty(self.client.get_deployment(&project_id).await?)
            }
            "generate_design" => {
                let project_id: String = require(&mut params, "project_id")?;
                let deployment_id: String = require(&mut params, "deployment_id")?;
                let artifact_ids: Vec<String> = field(&mut params, "artifact_ids")?;
                let ids = if artifact_ids.is_empty() { None } else { Some(artifact_ids.as_slice()) };
                pretty(self.client.generate_design(&project_id, &deployment_id, ids).await?)
            }
            "add_context_artifact" => {
                let project_id: String = require(&mut params, "project_id")?;
                let deployment_id: String = require(&mut params, "deployment_id")?;
                let title: String = require(&mut params, "title")?;
                let kind: String = require(&mut params, "kind")?;
                let content: Value = require(&mut params, "content")?;
                pretty(self.client.add_context_artifact(&project_id, &deployment_id, &title, &kind, &content).await?)
            }
            "list_deployment_runs" => {
                let project_id: String = require(&mut params, "project_id")?;
                let deployment_id: String = require(&mut params, "deployment_id")?;
                pretty(json!(self.client.list_deployment_runs(&project_id, &deployment_id).await?))
            }
            "deploy" => {
                let project_id: String = require(&mut params, "project_id")?;
                let deployment_id: String = require(&mut params, "deployment_id")?;
                pretty(self.client.deploy(&project_id, &deployment_id).await?)
            }
            "redeploy" => {
                let project_id: String = require(&mut params, "project_id")?;
                let deployment_id: String = require(&mut params, "deployment_id")?;
                pretty(self.client.redeploy(&project_id, &deployment_id).await?)
            }
            "destroy" => {
                let project_id: String = require(&mut params, "project_id")?;
                let deployment_id: String = require(&mut params, "deployment_id")?;
                pretty(self.client.destroy(&project_id, &deployment_id).await?)
            }
            other => bail!("unknown harvest tool: {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    fn tool(server: &MockServer, name: &'static str) -> HarvestTool {
        HarvestTool { client: Arc::new(Client::new(server.base_url(), "tok").unwrap()), name }
    }

    #[test]
    fn build_toolset_excludes_infra_tools_when_disallowed() {
        let client = Arc::new(Client::new("http://localhost:8080", "tok").unwrap());
        let names: Vec<String> = HarvestTool::build_toolset(client, false)
            .iter()
            .map(|t| t.definition().name)
            .collect();
        assert!(!names.contains(&"deploy".to_string()));
        assert!(!names.contains(&"destroy".to_string()));
        assert!(names.contains(&"create_project".to_string()));
    }

    #[test]
    fn build_toolset_includes_infra_tools_when_allowed() {
        let client = Arc::new(Client::new("http://localhost:8080", "tok").unwrap());
        let names: Vec<String> = HarvestTool::build_toolset(client, true)
            .iter()
            .map(|t| t.definition().name)
            .collect();
        assert!(names.contains(&"deploy".to_string()));
        assert!(names.contains(&"destroy".to_string()));
        assert!(names.contains(&"redeploy".to_string()));
    }

    #[test]
    fn create_project_definition_requires_name_and_group_id() {
        let server = MockServer::start();
        let def = tool(&server, "create_project").definition();
        let required: Vec<String> = serde_json::from_value(def.parameters["required"].clone()).unwrap();
        assert_eq!(required, vec!["name", "group_id"]);
    }

    #[tokio::test]
    async fn create_project_execute_calls_client_and_returns_pretty_json() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/projects").json_body(json!({"name": "n", "group_id": "g1"}));
            then.status(201).json_body(json!({"id": "p1", "name": "n"}));
        });
        let result = tool(&server, "create_project")
            .execute(json!({"name": "n", "group_id": "g1"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["id"], "p1");
    }

    #[tokio::test]
    async fn create_project_execute_fails_without_required_params() {
        let server = MockServer::start();
        let err = tool(&server, "create_project").execute(json!({"name": "n"})).await.unwrap_err();
        assert!(err.to_string().contains("group_id"));
    }

    #[tokio::test]
    async fn send_chat_message_execute_returns_query_response_json() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST).path("/projects/p1/query").json_body(json!({"query": "hi"}));
            then.status(200).json_body(json!({"answer": "hello there"}));
        });
        let result = tool(&server, "send_chat_message")
            .execute(json!({"project_id": "p1", "query": "hi"}))
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["answer"], "hello there");
    }

    #[tokio::test]
    async fn generate_design_execute_omits_artifact_ids_when_empty() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST)
                .path("/projects/p1/deployments/d1/design/generate")
                .json_body(json!({}));
            then.status(200).json_body(json!({"design_doc": {"id": "a1"}}));
        });
        tool(&server, "generate_design")
            .execute(json!({"project_id": "p1", "deployment_id": "d1"}))
            .await
            .unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn deploy_execute_hits_deploy_endpoint() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/projects/p1/deployments/d1/deploy");
            then.status(200).json_body(json!({"ok": true}));
        });
        tool(&server, "deploy").execute(json!({"project_id": "p1", "deployment_id": "d1"})).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn get_project_execute_bubbles_up_http_errors() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/projects/missing");
            then.status(404).json_body(json!({"error": "not found"}));
        });
        let err = tool(&server, "get_project").execute(json!({"project_id": "missing"})).await.unwrap_err();
        assert!(err.to_string().contains("404"));
    }
}
