use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use crate::machines::port_forwards;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

const LIST_PREVIEW_CHARS:   usize = 1000;
const MUTATE_PREVIEW_CHARS: usize = 500;

fn required_str(params: &Value, key: &str) -> Result<String> {
    params[key].as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("{key} is required"))
}

fn validate_list_params(params: &Value) -> Result<String> {
    required_str(params, "agent_id")
}

fn validate_create_params(params: &Value) -> Result<(String, u16, String)> {
    let agent_id = required_str(params, "agent_id")?;
    let port = params["port"].as_u64().ok_or_else(|| anyhow!("port is required"))?;
    let port = port_forwards::validate_port(port).map_err(|e| anyhow!(e))?;
    let route_name = required_str(params, "route_name")?;
    Ok((agent_id, port, route_name))
}

fn validate_update_params(params: &Value) -> Result<(String, String, Option<u16>, Option<String>)> {
    let agent_id = required_str(params, "agent_id")?;
    let forward_id = required_str(params, "forward_id")?;

    let port = match params.get("port") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let raw = v.as_u64().ok_or_else(|| anyhow!("port must be a number"))?;
            Some(port_forwards::validate_port(raw).map_err(|e| anyhow!(e))?)
        }
    };
    let route_name = params["route_name"].as_str().map(str::to_string);

    Ok((agent_id, forward_id, port, route_name))
}

fn validate_delete_params(params: &Value) -> Result<(String, String)> {
    let agent_id = required_str(params, "agent_id")?;
    let forward_id = required_str(params, "forward_id")?;
    Ok((agent_id, forward_id))
}

pub struct ListPortForwardsTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for ListPortForwardsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_port_forwards".into(),
            description: "List HTTP port forwards exposed for an agent machine in this project. \
                          Use list_agents first to find the agent_id."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type":        "string",
                        "description": "The agent machine ID (from list_agents)"
                    }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let agent_id = validate_list_params(&params)?;
        let forwards = port_forwards::list_for_agent(&self.neo4j, &self.project_id, &agent_id)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(serde_json::to_string_pretty(&forwards)?)
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(LIST_PREVIEW_CHARS).collect()
    }
}

pub struct CreatePortForwardTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for CreatePortForwardTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_port_forward".into(),
            description: "Expose an HTTP port on a connected agent machine as a named route, \
                          reachable at /agents/{agent_id}/{route_name} once created. This makes \
                          real network traffic on that port reachable through the server and must \
                          only be called after the user has explicitly confirmed."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type":        "string",
                        "description": "The agent machine ID (from list_agents)"
                    },
                    "port": {
                        "type":        "integer",
                        "description": "The port on the agent to forward (1-65535)"
                    },
                    "route_name": {
                        "type":        "string",
                        "description": "A short name for the route, e.g. 'app'"
                    }
                },
                "required": ["agent_id", "port", "route_name"]
            }),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (agent_id, port, route_name) = validate_create_params(&params)?;
        let forward = port_forwards::create(&self.neo4j, &self.project_id, &agent_id, port, &route_name)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(format!(
            "Port forward '{}' created for agent '{agent_id}' (port {}).",
            forward.route_name, forward.port
        ))
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(MUTATE_PREVIEW_CHARS).collect()
    }
}

pub struct UpdatePortForwardTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for UpdatePortForwardTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "update_port_forward".into(),
            description: "Change the port and/or route name of an existing port forward. \
                          Use list_port_forwards first to find the forward_id. This changes what \
                          traffic reaches through /agents/{agent_id}/{route_name} and must only be \
                          called after the user has explicitly confirmed."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type":        "string",
                        "description": "The agent machine ID"
                    },
                    "forward_id": {
                        "type":        "string",
                        "description": "The port forward ID (from list_port_forwards)"
                    },
                    "port": {
                        "type":        "integer",
                        "description": "New port (1-65535); omit to keep unchanged"
                    },
                    "route_name": {
                        "type":        "string",
                        "description": "New route name; omit to keep unchanged"
                    }
                },
                "required": ["agent_id", "forward_id"]
            }),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (agent_id, forward_id, port, route_name) = validate_update_params(&params)?;
        let forward = port_forwards::update(&self.neo4j, &self.project_id, &agent_id, &forward_id, port, route_name)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(format!(
            "Port forward '{}' updated (port {}).",
            forward.route_name, forward.port
        ))
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(MUTATE_PREVIEW_CHARS).collect()
    }
}

pub struct DeletePortForwardTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for DeletePortForwardTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_port_forward".into(),
            description: "Permanently remove a port forward, closing the \
                          /agents/{agent_id}/{route_name} route. Use list_port_forwards first to \
                          find the forward_id. This cannot be undone and must only be called after \
                          the user has explicitly confirmed."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type":        "string",
                        "description": "The agent machine ID"
                    },
                    "forward_id": {
                        "type":        "string",
                        "description": "The port forward ID (from list_port_forwards)"
                    }
                },
                "required": ["agent_id", "forward_id"]
            }),
        }
    }

    fn requires_confirmation(&self) -> bool {
        true
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let (agent_id, forward_id) = validate_delete_params(&params)?;
        port_forwards::delete(&self.neo4j, &self.project_id, &agent_id, &forward_id)
            .await
            .map_err(|e| anyhow!(e.to_string()))?;
        Ok(format!("Port forward '{forward_id}' deleted."))
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(MUTATE_PREVIEW_CHARS).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_params_requires_agent_id() {
        assert!(validate_list_params(&json!({})).is_err());
    }

    #[test]
    fn list_params_accepts_valid_input() {
        assert_eq!(validate_list_params(&json!({ "agent_id": "a1" })).unwrap(), "a1");
    }

    #[test]
    fn create_params_requires_agent_id() {
        let err = validate_create_params(&json!({ "port": 8080, "route_name": "app" })).unwrap_err();
        assert!(err.to_string().contains("agent_id"));
    }

    #[test]
    fn create_params_requires_port() {
        let err = validate_create_params(&json!({ "agent_id": "a1", "route_name": "app" })).unwrap_err();
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn create_params_rejects_invalid_port() {
        let err = validate_create_params(&json!({ "agent_id": "a1", "port": 99999, "route_name": "app" })).unwrap_err();
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn create_params_requires_route_name() {
        let err = validate_create_params(&json!({ "agent_id": "a1", "port": 8080 })).unwrap_err();
        assert!(err.to_string().contains("route_name"));
    }

    #[test]
    fn create_params_accepts_valid_input() {
        let (agent_id, port, route_name) = validate_create_params(&json!({
            "agent_id": "a1", "port": 8080, "route_name": "app"
        })).unwrap();
        assert_eq!(agent_id, "a1");
        assert_eq!(port, 8080);
        assert_eq!(route_name, "app");
    }

    #[test]
    fn update_params_requires_agent_id_and_forward_id() {
        assert!(validate_update_params(&json!({ "forward_id": "f1" })).is_err());
        assert!(validate_update_params(&json!({ "agent_id": "a1" })).is_err());
    }

    #[test]
    fn update_params_allows_omitting_port_and_route_name() {
        let (agent_id, forward_id, port, route_name) = validate_update_params(&json!({
            "agent_id": "a1", "forward_id": "f1"
        })).unwrap();
        assert_eq!(agent_id, "a1");
        assert_eq!(forward_id, "f1");
        assert_eq!(port, None);
        assert_eq!(route_name, None);
    }

    #[test]
    fn update_params_rejects_invalid_port() {
        let err = validate_update_params(&json!({
            "agent_id": "a1", "forward_id": "f1", "port": 0
        })).unwrap_err();
        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn update_params_accepts_new_port_and_route_name() {
        let (_, _, port, route_name) = validate_update_params(&json!({
            "agent_id": "a1", "forward_id": "f1", "port": 9090, "route_name": "app2"
        })).unwrap();
        assert_eq!(port, Some(9090));
        assert_eq!(route_name, Some("app2".to_string()));
    }

    #[test]
    fn delete_params_requires_agent_id_and_forward_id() {
        assert!(validate_delete_params(&json!({ "forward_id": "f1" })).is_err());
        assert!(validate_delete_params(&json!({ "agent_id": "a1" })).is_err());
    }

    #[test]
    fn delete_params_accepts_valid_input() {
        let (agent_id, forward_id) = validate_delete_params(&json!({
            "agent_id": "a1", "forward_id": "f1"
        })).unwrap();
        assert_eq!(agent_id, "a1");
        assert_eq!(forward_id, "f1");
    }
}
