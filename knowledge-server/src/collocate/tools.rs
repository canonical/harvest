use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use collocate_remote::control::ExecOptions;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::agent::tool::Tool;
use crate::llm::types::ToolDefinition;
use super::client::{CollocateHandle, CreateContainerOpts};
use super::sessions::{SessionContainer, SessionContainerRegistry};

const RUN_PREVIEW_CHARS: usize = 3000;
const EXEC_PREVIEW_CHARS: usize = 3000;
const LIST_PREVIEW_CHARS: usize = 2000;
const MUTATE_PREVIEW_CHARS: usize = 500;
const MAX_EXEC_TIMEOUT_SECS: u64 = 600;
const MAX_CONTAINER_NAME_LEN: usize = 64;

fn required_str(params: &Value, key: &str) -> Result<String> {
    params[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| anyhow!("{key} is required"))
}

fn optional_str(params: &Value, key: &str) -> Option<String> {
    params[key].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn sanitize_name(name: &str, project_id: &str, conversation_id: &str) -> String {
    let prefix = format!("hv-{}-{}-", &project_id[..project_id.len().min(8)], &conversation_id[..conversation_id.len().min(8)]);
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let full = format!("{prefix}{cleaned}");
    if full.len() > MAX_CONTAINER_NAME_LEN {
        full[..MAX_CONTAINER_NAME_LEN].to_string()
    } else {
        full
    }
}

pub struct CollocateRunTool {
    pub handle: CollocateHandle,
    pub project_id: String,
}

#[async_trait]
impl Tool for CollocateRunTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "collocate_run".into(),
            description: "Run a command in an ephemeral collocate container that is created for this \
                          single call and removed immediately after. This runs containers directly \
                          on the Collocate daemon — do NOT use list_agents or run_command for container \
                          tasks, and do NOT generate Docker scripts. Use this for one-off tasks like \
                          building, linting, running tests, or verifying a service with curl. The \
                          container uses the default image unless 'image' is specified. Returns stdout, \
                          stderr, and exit code."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "image": {
                        "type": "string",
                        "description": "Container image (e.g. 'ubuntu:24.04', 'python:3.12-slim'). Defaults to the server's configured default image."
                    },
                    "command": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The command to execute as argv array"
                    },
                    "env": {
                        "type": "object",
                        "description": "Environment variables as key-value pairs",
                        "additionalProperties": { "type": "string" }
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory inside the container"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Execution timeout in seconds (default 120, max 600)",
                        "default": 120
                    }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let command = params["command"]
            .as_array()
            .ok_or_else(|| anyhow!("command must be an array"))?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<_>>();
        if command.is_empty() {
            anyhow::bail!("command must not be empty");
        }
        let image = optional_str(&params, "image");
        let workdir = optional_str(&params, "workdir");
        let env: Vec<(String, String)> = params["env"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let timeout_secs = params["timeout_secs"]
            .as_u64()
            .unwrap_or(120)
            .min(MAX_EXEC_TIMEOUT_SECS);

        let image_for_opts = image.clone().or_else(|| Some(self.handle.config().default_image.clone()));
        let name = format!("hv-ephemeral-{}", Utc::now().timestamp_millis());
        let opts = CreateContainerOpts {
            image: image_for_opts,
            command,
            env,
            workdir,
            ..Default::default()
        };

        let id = self.handle.create_container(&name, opts).await?;
        let argv: Vec<String> = params["command"]
            .as_array()
            .ok_or_else(|| anyhow!("command must be an array"))?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        let exec_opts = ExecOptions {
            timeout_secs: Some(timeout_secs),
            ..ExecOptions::new()
        };
        let result = self.handle.exec(&id.to_string(), &argv_refs, &exec_opts, Vec::new()).await;
        let _ = self.handle.rm_force(&id.to_string()).await;
        let result = result?;
        Ok(serde_json::to_string_pretty(&json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
        }))?)
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(RUN_PREVIEW_CHARS).collect()
    }
}

pub struct CollocateCreateSessionTool {
    pub handle: CollocateHandle,
    pub registry: Arc<SessionContainerRegistry>,
    pub project_id: String,
    pub conversation_id: String,
}

#[async_trait]
impl Tool for CollocateCreateSessionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "collocate_create_session".into(),
            description: "Create a session collocate container that stays alive across multiple \
                          collocate_exec calls within this conversation. This runs containers directly \
                          on the Collocate daemon — do NOT use list_agents or generate Docker scripts. \
                          Use this for persistent services (web servers, databases) or multi-step \
                          builds. Set persistent=true for a long-lived service that survives past \
                          this response. Non-persistent containers are auto-deleted at the end of the \
                          response unless collocate_delete_session is called first. Returns the \
                          container ID, IP address, and published ports."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "A short name for the container"
                    },
                    "image": {
                        "type": "string",
                        "description": "Container image (e.g. 'ubuntu:24.04', 'python:3.12-slim')"
                    },
                    "command": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Command to run as the container's main process. Omit for a sleep/idle container."
                    },
                    "env": {
                        "type": "object",
                        "description": "Environment variables",
                        "additionalProperties": { "type": "string" }
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory inside the container"
                    },
                    "user": {
                        "type": "string",
                        "description": "User to run as (e.g. 'root')"
                    },
                    "publish": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Port mappings as 'host_port:container_port' strings (e.g. ['8080:80'])"
                    },
                    "persistent": {
                        "type": "boolean",
                        "description": "If true, the container is long-lived and survives past the response. Default false.",
                        "default": false
                    },
                    "idle_timeout_secs": {
                        "type": "integer",
                        "description": "Auto-stop timeout in seconds"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let name = required_str(&params, "name")?;
        let image = optional_str(&params, "image");
        let command: Vec<String> = params["command"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let env: Vec<(String, String)> = params["env"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();
        let workdir = optional_str(&params, "workdir");
        let user = optional_str(&params, "user");
        let persistent = params["persistent"].as_bool().unwrap_or(false);
        let idle_timeout_secs = params["idle_timeout_secs"].as_u64();

        let publish: Vec<(u16, u16)> = params["publish"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        v.as_str().and_then(|s| {
                            let parts: Vec<&str> = s.split(':').collect();
                            if parts.len() == 2 {
                                let host: u16 = parts[0].parse().ok()?;
                                let container: u16 = parts[1].parse().ok()?;
                                Some((host, container))
                            } else {
                                None
                            }
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let count = self.registry.count_for_project(&self.project_id);
        if count >= self.handle.config().max_containers {
            anyhow::bail!(
                "maximum {} collocate containers reached for this project",
                self.handle.config().max_containers
            );
        }

        let full_name = sanitize_name(&name, &self.project_id, &self.conversation_id);
        let image_for_opts = image.clone().or_else(|| Some(self.handle.config().default_image.clone()));
        let opts = CreateContainerOpts {
            image: image_for_opts,
            command,
            env,
            workdir,
            user,
            publish: publish.clone(),
            persistent,
            idle_timeout_secs,
            ..Default::default()
        };

        let id = self.handle.create_container(&full_name, opts).await?;
        let id_str = id.to_string();

        let containers = self.handle.list().await?;
        let info = containers
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| anyhow!("container {id_str} not found after creation"))?;
        let address = info.address.map(|a| a.to_string());
        let published: Vec<String> = info.published.clone();

        let session = SessionContainer {
            id: id_str.clone(),
            name: full_name.clone(),
            project_id: self.project_id.clone(),
            conversation_id: self.conversation_id.clone(),
            created_at: Utc::now(),
            persistent,
            address: address.clone(),
            published: published.clone(),
        };
        self.registry.register_with_handle(session, Arc::new(self.handle.clone()));

        Ok(serde_json::to_string_pretty(&json!({
            "container_id": id_str,
            "name": full_name,
            "address": address,
            "published": published,
            "persistent": persistent,
        }))?)
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(MUTATE_PREVIEW_CHARS).collect()
    }
}

pub struct CollocateExecTool {
    pub handle: CollocateHandle,
    pub registry: Arc<SessionContainerRegistry>,
    pub project_id: String,
    pub conversation_id: String,
}

#[async_trait]
impl Tool for CollocateExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "collocate_exec".into(),
            description: "Execute a command inside an existing session collocate container. \
                          Use collocate_create_session first to get a container_id, then call this \
                          multiple times for each command. Returns stdout, stderr, and exit code."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "container_id": {
                        "type": "string",
                        "description": "The container ID from collocate_create_session"
                    },
                    "command": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "The command to execute as argv array"
                    },
                    "env": {
                        "type": "object",
                        "description": "Environment variables",
                        "additionalProperties": { "type": "string" }
                    },
                    "workdir": {
                        "type": "string",
                        "description": "Working directory for this command"
                    },
                    "user": {
                        "type": "string",
                        "description": "User to run as (e.g. 'root')"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (default 60, max 600)",
                        "default": 60
                    }
                },
                "required": ["container_id", "command"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let container_id = required_str(&params, "container_id")?;
        let command: Vec<String> = params["command"]
            .as_array()
            .ok_or_else(|| anyhow!("command must be an array"))?
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        if command.is_empty() {
            anyhow::bail!("command must not be empty");
        }
        let timeout_secs = params["timeout_secs"]
            .as_u64()
            .unwrap_or(60)
            .min(MAX_EXEC_TIMEOUT_SECS);

        let _ = self
            .registry
            .verify_ownership(&container_id, &self.project_id, &self.conversation_id)
            .ok_or_else(|| anyhow!("container {container_id} not found in this conversation"))?;

        let env: Vec<(String, String)> = params["env"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
            .unwrap_or_default();
        let workdir = optional_str(&params, "workdir");
        let user = optional_str(&params, "user");

        let mut opts = ExecOptions {
            timeout_secs: Some(timeout_secs),
            ..ExecOptions::new()
        };
        for (k, v) in &env {
            opts = opts.with_env(k.clone(), v.clone());
        }
        if let Some(wd) = &workdir {
            opts.workdir = Some(wd.clone());
        }
        if let Some(u) = &user {
            opts.user = Some(u.clone());
        }

        let argv: Vec<&str> = command.iter().map(|s| s.as_str()).collect();
        let result = self.handle.exec(&container_id, &argv, &opts, Vec::new()).await?;
        Ok(serde_json::to_string_pretty(&json!({
            "stdout": result.stdout,
            "stderr": result.stderr,
            "exit_code": result.exit_code,
        }))?)
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(EXEC_PREVIEW_CHARS).collect()
    }
}

pub struct CollocateDeleteSessionTool {
    pub handle: CollocateHandle,
    pub registry: Arc<SessionContainerRegistry>,
    pub project_id: String,
    pub conversation_id: String,
}

#[async_trait]
impl Tool for CollocateDeleteSessionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "collocate_delete_session".into(),
            description: "Delete a session collocate container. Use this when you are done with \
                          a container to free resources immediately. Non-persistent containers are \
                          also auto-deleted at the end of the response if not explicitly deleted."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "container_id": {
                        "type": "string",
                        "description": "The container ID to delete"
                    }
                },
                "required": ["container_id"]
            }),
        }
    }

    fn requires_confirmation(&self) -> bool {
        false
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let container_id = required_str(&params, "container_id")?;
        let _ = self
            .registry
            .verify_ownership(&container_id, &self.project_id, &self.conversation_id)
            .ok_or_else(|| anyhow!("container {container_id} not found in this conversation"))?;
        self.handle.rm_force(&container_id).await?;
        self.registry.remove_entry(&container_id);
        Ok(format!("Container '{container_id}' deleted."))
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(MUTATE_PREVIEW_CHARS).collect()
    }
}

pub struct CollocateListContainersTool {
    pub registry: Arc<SessionContainerRegistry>,
    pub project_id: String,
    pub conversation_id: String,
}

#[async_trait]
impl Tool for CollocateListContainersTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "collocate_list_containers".into(),
            description: "List all collocate session containers in this conversation. Returns \
                          container IDs, names, IP addresses, states, and published ports."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let containers = self.registry.list_for_session(&self.project_id, &self.conversation_id);
        let result: Vec<Value> = containers
            .iter()
            .map(|c| {
                json!({
                    "container_id": c.id,
                    "name": c.name,
                    "address": c.address,
                    "published": c.published,
                    "persistent": c.persistent,
                    "created_at": c.created_at.to_rfc3339(),
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&result)?)
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(LIST_PREVIEW_CHARS).collect()
    }
}

pub struct CollocateTransferFileTool {
    pub handle: CollocateHandle,
    pub registry: Arc<SessionContainerRegistry>,
    pub project_id: String,
    pub conversation_id: String,
}

#[async_trait]
impl Tool for CollocateTransferFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "collocate_transfer_file".into(),
            description: "Transfer a file to or from a collocate container. For 'to' direction, \
                          provide 'content' (the file content as a string) and 'remote_path'. \
                          For 'from' direction, provide 'remote_path' and the file content is \
                          returned. Use this to write source files into a container before running \
                          tests, or to read output files back."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "container_id": {
                        "type": "string",
                        "description": "The container ID"
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["to", "from"],
                        "description": "Transfer direction: 'to' writes content into the container, 'from' reads content out"
                    },
                    "remote_path": {
                        "type": "string",
                        "description": "Path inside the container"
                    },
                    "content": {
                        "type": "string",
                        "description": "File content (required for 'to' direction)"
                    }
                },
                "required": ["container_id", "direction", "remote_path"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let container_id = required_str(&params, "container_id")?;
        let direction = required_str(&params, "direction")?;
        let remote_path = required_str(&params, "remote_path")?;

        let _ = self
            .registry
            .verify_ownership(&container_id, &self.project_id, &self.conversation_id)
            .ok_or_else(|| anyhow!("container {container_id} not found in this conversation"))?;

        match direction.as_str() {
            "to" => {
                let content = params["content"]
                    .as_str()
                    .ok_or_else(|| anyhow!("content is required for 'to' direction"))?;
                self.handle
                    .transfer_to(&container_id, &remote_path, content.as_bytes().to_vec())
                    .await?;
                Ok(format!("Wrote {} bytes to {remote_path}", content.len()))
            }
            "from" => {
                let data = self.handle.transfer_from(&container_id, &remote_path).await?;
                let content = String::from_utf8_lossy(&data).into_owned();
                Ok(content)
            }
            other => anyhow::bail!("direction must be 'to' or 'from', got '{other}'"),
        }
    }

    fn preview(&self, result: &str) -> String {
        result.chars().take(RUN_PREVIEW_CHARS).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_name_alphanumeric_only() {
        let name = sanitize_name("my container!!", "proj1234", "conv5678");
        assert!(name.starts_with("hv-proj1234-conv5678-"));
        assert!(!name.contains('!'));
        assert!(name.contains("mycontainer"));
    }

    #[test]
    fn sanitize_name_truncates_long_names() {
        let long_name = "a".repeat(200);
        let result = sanitize_name(&long_name, "p", "c");
        assert!(result.len() <= MAX_CONTAINER_NAME_LEN);
    }

    #[test]
    fn required_str_returns_trimmed_value() {
        let params = json!({"key": "  value  "});
        assert_eq!(required_str(&params, "key").unwrap(), "value");
    }

    #[test]
    fn required_str_errors_on_missing() {
        let params = json!({});
        let err = required_str(&params, "key").unwrap_err();
        assert!(err.to_string().contains("key"));
    }

    #[test]
    fn required_str_errors_on_empty() {
        let params = json!({"key": "   "});
        let err = required_str(&params, "key").unwrap_err();
        assert!(err.to_string().contains("key"));
    }

    #[test]
    fn optional_str_returns_none_for_empty() {
        let params = json!({"key": ""});
        assert!(optional_str(&params, "key").is_none());
    }

    #[test]
    fn optional_str_returns_trimmed() {
        let params = json!({"key": "  val  "});
        assert_eq!(optional_str(&params, "key").unwrap(), "val");
    }
}
