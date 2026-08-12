pub mod handlers;
pub mod lxd_provision;
pub mod port_forwards;
pub mod proxy;

use axum::extract::ws::Message;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Instant};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ResultBody {
    pub request_id: String,
    pub stdout:     String,
    pub stderr:     String,
    pub exit_code:  i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerraformFlavor {
    Terraform,
    Terragrunt,
}

impl TerraformFlavor {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "terraform"  => Some(Self::Terraform),
            "terragrunt" => Some(Self::Terragrunt),
            _            => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Terraform  => "terraform",
            Self::Terragrunt => "terragrunt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerraformAction {
    Plan,
    Apply,
    Destroy,
}

impl TerraformAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "plan"    => Some(Self::Plan),
            "apply"   => Some(Self::Apply),
            "destroy" => Some(Self::Destroy),
            _         => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Plan    => "plan",
            Self::Apply   => "apply",
            Self::Destroy => "destroy",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToAgent {
    Registered   { agent_token: String },
    HelloAck,
    Execute      { request_id: String, command: String, timeout_secs: u64 },
    RunTerraform {
        request_id:   String,
        artifact_id:  String,
        flavor:       TerraformFlavor,
        action:       TerraformAction,
        files:        BTreeMap<String, String>,
        timeout_secs: u64,
    },
    OpenShell    { session_id: String, cols: u16, rows: u16 },
    OpenTunnel   { session_id: String, port: u16 },
    Uninstall,
    Error        { message: String },
}

#[derive(Debug)]
pub struct CommandResult {
    pub stdout:    String,
    pub stderr:    String,
    pub exit_code: i32,
}

pub struct ConnectedAgent {
    pub id:           String,
    pub project_id:   String,
    pub hostname:     String,
    pub connected_at: DateTime<Utc>,
    pub sender:       mpsc::Sender<ServerToAgent>,
}

pub struct PendingResult {
    pub tx:       oneshot::Sender<Result<CommandResult, String>>,
    pub deadline: Instant,
}

pub struct PendingConsoleSession {
    pub agent_id:      String,
    pub to_browser_tx: mpsc::Sender<Message>,
    pub to_agent_rx:   mpsc::Receiver<Message>,
}

pub struct PendingTunnelSession {
    pub agent_id:      String,
    pub to_caller_tx:  mpsc::Sender<Message>,
    pub to_agent_rx:   mpsc::Receiver<Message>,
}

#[derive(Default)]
pub struct MachineRegistry {
    pub agents:          DashMap<String, ConnectedAgent>,
    pub pending:         DashMap<String, PendingResult>,
    pub token_index:     DashMap<String, String>,
    pub console_pending: DashMap<String, PendingConsoleSession>,
    pub tunnel_pending:  DashMap<String, PendingTunnelSession>,
    /// Live stdout/stderr line subscribers for in-flight `execute_terraform` calls, keyed by request_id.
    pub output:          DashMap<String, mpsc::Sender<Value>>,
}

impl MachineRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn disconnect_if_current(&self, agent_id: &str, sender: &mpsc::Sender<ServerToAgent>) -> bool {
        let is_current = self.agents
            .get(agent_id)
            .is_some_and(|a| a.sender.same_channel(sender));

        if !is_current {
            return false;
        }

        self.agents.remove(agent_id);
        true
    }

    pub fn agents_for_project(&self, project_id: &str) -> Vec<serde_json::Value> {
        self.agents
            .iter()
            .filter(|e| e.value().project_id == project_id)
            .map(|e| {
                let agent = e.value();
                serde_json::json!({
                    "id":           agent.id,
                    "hostname":     agent.hostname,
                    "online":       true,
                    "connected_at": agent.connected_at.to_rfc3339(),
                })
            })
            .collect()
    }

    async fn send_and_await(
        &self,
        agent_id: &str,
        request_id: String,
        message: ServerToAgent,
        timeout_secs: u64,
    ) -> Result<CommandResult, String> {
        let sender = self
            .agents
            .get(agent_id)
            .ok_or_else(|| format!("agent {agent_id} not connected"))?
            .sender
            .clone();

        let (result_tx, result_rx) = oneshot::channel();

        self.pending.insert(request_id.clone(), PendingResult {
            tx:       result_tx,
            deadline: Instant::now() + std::time::Duration::from_secs(timeout_secs + 5),
        });

        sender
            .send(message)
            .await
            .map_err(|_| "agent disconnected before send".to_string())?;

        let wait = std::time::Duration::from_secs(timeout_secs + 10);
        tokio::time::timeout(wait, result_rx)
            .await
            .map_err(|_| "timed out waiting for command result".to_string())?
            .map_err(|_| "result channel closed".to_string())?
    }

    pub async fn execute(
        &self,
        agent_id: &str,
        command:  String,
        timeout_secs: u64,
    ) -> Result<CommandResult, String> {
        let request_id = Uuid::new_v4().to_string();
        self.send_and_await(
            agent_id,
            request_id.clone(),
            ServerToAgent::Execute { request_id, command, timeout_secs },
            timeout_secs,
        ).await
    }

    /// `output_tx`, if given, receives a `{"stream": "stdout"|"stderr", "line": "..."}` value
    /// for each line the agent produces while the command runs.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_terraform(
        &self,
        agent_id:     &str,
        artifact_id:  String,
        flavor:       TerraformFlavor,
        action:       TerraformAction,
        files:        BTreeMap<String, String>,
        timeout_secs: u64,
        output_tx:    Option<mpsc::Sender<Value>>,
    ) -> Result<CommandResult, String> {
        let request_id = Uuid::new_v4().to_string();
        if let Some(tx) = output_tx {
            self.output.insert(request_id.clone(), tx);
        }
        let result = self.send_and_await(
            agent_id,
            request_id.clone(),
            ServerToAgent::RunTerraform { request_id: request_id.clone(), artifact_id, flavor, action, files, timeout_secs },
            timeout_secs,
        ).await;
        self.output.remove(&request_id);
        result
    }

    pub async fn open_console_session(
        &self,
        agent_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(String, mpsc::Receiver<Message>, mpsc::Sender<Message>), String> {
        let sender = self
            .agents
            .get(agent_id)
            .ok_or_else(|| format!("agent {agent_id} not connected"))?
            .sender
            .clone();

        let session_id = Uuid::new_v4().to_string();
        let (to_agent_tx, to_agent_rx) = mpsc::channel::<Message>(64);
        let (to_browser_tx, to_browser_rx) = mpsc::channel::<Message>(64);

        self.console_pending.insert(session_id.clone(), PendingConsoleSession {
            agent_id: agent_id.to_string(),
            to_browser_tx,
            to_agent_rx,
        });

        if sender
            .send(ServerToAgent::OpenShell { session_id: session_id.clone(), cols, rows })
            .await
            .is_err()
        {
            self.console_pending.remove(&session_id);
            return Err("agent disconnected before send".to_string());
        }

        Ok((session_id, to_browser_rx, to_agent_tx))
    }

    pub fn claim_console_session(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<(mpsc::Sender<Message>, mpsc::Receiver<Message>)> {
        let (_, pending) = self.console_pending.remove(session_id)?;
        if pending.agent_id != agent_id {
            return None;
        }
        Some((pending.to_browser_tx, pending.to_agent_rx))
    }

    pub fn expire_console_session(&self, session_id: &str) -> bool {
        self.console_pending.remove(session_id).is_some()
    }

    pub async fn open_tunnel_session(
        &self,
        agent_id: &str,
        port: u16,
    ) -> Result<(String, mpsc::Receiver<Message>, mpsc::Sender<Message>), String> {
        let sender = self
            .agents
            .get(agent_id)
            .ok_or_else(|| format!("agent {agent_id} not connected"))?
            .sender
            .clone();

        let session_id = Uuid::new_v4().to_string();
        let (to_agent_tx, to_agent_rx) = mpsc::channel::<Message>(64);
        let (to_caller_tx, to_caller_rx) = mpsc::channel::<Message>(64);

        self.tunnel_pending.insert(session_id.clone(), PendingTunnelSession {
            agent_id: agent_id.to_string(),
            to_caller_tx,
            to_agent_rx,
        });

        if sender
            .send(ServerToAgent::OpenTunnel { session_id: session_id.clone(), port })
            .await
            .is_err()
        {
            self.tunnel_pending.remove(&session_id);
            return Err("agent disconnected before send".to_string());
        }

        Ok((session_id, to_caller_rx, to_agent_tx))
    }

    pub fn claim_tunnel_session(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Option<(mpsc::Sender<Message>, mpsc::Receiver<Message>)> {
        let (_, pending) = self.tunnel_pending.remove(session_id)?;
        if pending.agent_id != agent_id {
            return None;
        }
        Some((pending.to_caller_tx, pending.to_agent_rx))
    }

    pub fn expire_tunnel_session(&self, session_id: &str) -> bool {
        self.tunnel_pending.remove(session_id).is_some()
    }
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_token_is_deterministic() {
        assert_eq!(hash_token("abc"), hash_token("abc"));
    }

    #[test]
    fn hash_token_differs_for_different_inputs() {
        assert_ne!(hash_token("token-a"), hash_token("token-b"));
    }

    #[test]
    fn hash_token_is_64_hex_chars() {
        let h = hash_token("some-token");
        assert_eq!(h.len(), 64, "SHA-256 hex is 64 chars: {h}");
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()), "not hex: {h}");
    }

    #[test]
    fn terraform_flavor_parses_known_values_only() {
        assert_eq!(TerraformFlavor::parse("terraform"), Some(TerraformFlavor::Terraform));
        assert_eq!(TerraformFlavor::parse("terragrunt"), Some(TerraformFlavor::Terragrunt));
        assert_eq!(TerraformFlavor::parse("cloudformation"), None);
    }

    #[test]
    fn terraform_flavor_as_str_round_trips() {
        for flavor in [TerraformFlavor::Terraform, TerraformFlavor::Terragrunt] {
            assert_eq!(TerraformFlavor::parse(flavor.as_str()), Some(flavor));
        }
    }

    #[test]
    fn terraform_action_parses_known_values_only() {
        assert_eq!(TerraformAction::parse("plan"), Some(TerraformAction::Plan));
        assert_eq!(TerraformAction::parse("apply"), Some(TerraformAction::Apply));
        assert_eq!(TerraformAction::parse("destroy"), Some(TerraformAction::Destroy));
        assert_eq!(TerraformAction::parse("delete"), None);
    }

    #[test]
    fn terraform_action_as_str_round_trips() {
        for action in [TerraformAction::Plan, TerraformAction::Apply, TerraformAction::Destroy] {
            assert_eq!(TerraformAction::parse(action.as_str()), Some(action));
        }
    }

    #[tokio::test]
    async fn execute_terraform_unknown_agent_returns_error() {
        let registry = MachineRegistry::new();
        let e = registry.execute_terraform(
            "nonexistent",
            "artifact-1".into(),
            TerraformFlavor::Terraform,
            TerraformAction::Plan,
            BTreeMap::new(),
            30,
            None,
        ).await.unwrap_err();
        assert!(e.contains("not connected"), "got: {e}");
    }

    #[tokio::test]
    async fn execute_terraform_sends_run_terraform_message_and_resolves_result() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");

        let mut files = BTreeMap::new();
        files.insert("main.tf".to_string(), "resource \"local_file\" \"x\" {}".to_string());

        let registry_clone = Arc::clone(&registry);
        let files_clone = files.clone();
        let handle = tokio::spawn(async move {
            registry_clone.execute_terraform(
                "a1",
                "artifact-1".into(),
                TerraformFlavor::Terraform,
                TerraformAction::Plan,
                files_clone,
                30,
                None,
            ).await
        });

        let request_id = match rx.recv().await.unwrap() {
            ServerToAgent::RunTerraform { request_id, artifact_id, flavor, action, files: sent_files, timeout_secs } => {
                assert_eq!(artifact_id, "artifact-1");
                assert_eq!(flavor, TerraformFlavor::Terraform);
                assert_eq!(action, TerraformAction::Plan);
                assert_eq!(sent_files, files);
                assert_eq!(timeout_secs, 30);
                request_id
            }
            other => panic!("expected RunTerraform, got {other:?}"),
        };

        let (_, pending) = registry.pending.remove(&request_id).unwrap();
        pending.tx.send(Ok(CommandResult { stdout: "planned".into(), stderr: String::new(), exit_code: 0 })).unwrap();

        let result = handle.await.unwrap().unwrap();
        assert_eq!(result.stdout, "planned");
    }

    #[tokio::test]
    async fn execute_terraform_registers_and_cleans_up_output_channel() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");

        let (output_tx, mut output_rx) = mpsc::channel::<Value>(8);

        let registry_clone = Arc::clone(&registry);
        let handle = tokio::spawn(async move {
            registry_clone.execute_terraform(
                "a1",
                "artifact-1".into(),
                TerraformFlavor::Terraform,
                TerraformAction::Plan,
                BTreeMap::new(),
                30,
                Some(output_tx),
            ).await
        });

        let request_id = match rx.recv().await.unwrap() {
            ServerToAgent::RunTerraform { request_id, .. } => request_id,
            other => panic!("expected RunTerraform, got {other:?}"),
        };

        assert!(registry.output.contains_key(&request_id), "output channel should be registered while the run is in flight");

        let sender = registry.output.get(&request_id).unwrap().clone();
        sender.send(serde_json::json!({"stream": "stdout", "line": "Initializing..."})).await.unwrap();
        let line = output_rx.recv().await.unwrap();
        assert_eq!(line["line"], "Initializing...");

        let (_, pending) = registry.pending.remove(&request_id).unwrap();
        pending.tx.send(Ok(CommandResult { stdout: String::new(), stderr: String::new(), exit_code: 0 })).unwrap();
        handle.await.unwrap().unwrap();

        assert!(!registry.output.contains_key(&request_id), "output channel should be cleaned up once the run completes");
    }

    fn register_agent(registry: &MachineRegistry, agent_id: &str) -> mpsc::Receiver<ServerToAgent> {
        let (tx, rx) = mpsc::channel::<ServerToAgent>(8);
        registry.agents.insert(agent_id.to_string(), ConnectedAgent {
            id:           agent_id.to_string(),
            project_id:   "proj-1".into(),
            hostname:     "host-1".into(),
            connected_at: Utc::now(),
            sender:       tx,
        });
        rx
    }

    #[tokio::test]
    async fn open_console_session_unknown_agent_returns_error() {
        let registry = MachineRegistry::new();
        let e = registry.open_console_session("nonexistent", 80, 24).await.unwrap_err();
        assert!(e.contains("not connected"), "got: {e}");
    }

    #[tokio::test]
    async fn open_console_session_sends_open_shell_message() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");

        let (session_id, _to_browser_rx, _to_agent_tx) =
            registry.open_console_session("a1", 80, 24).await.unwrap();

        match rx.recv().await.unwrap() {
            ServerToAgent::OpenShell { session_id: sid, cols, rows } => {
                assert_eq!(sid, session_id);
                assert_eq!(cols, 80);
                assert_eq!(rows, 24);
            }
            other => panic!("expected OpenShell, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn claim_console_session_succeeds_once_for_matching_agent() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");
        let (session_id, ..) = registry.open_console_session("a1", 80, 24).await.unwrap();
        rx.recv().await.unwrap();

        assert!(registry.claim_console_session(&session_id, "a1").is_some());
        assert!(registry.claim_console_session(&session_id, "a1").is_none());
    }

    #[tokio::test]
    async fn claim_console_session_rejects_mismatched_agent() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");
        let (session_id, ..) = registry.open_console_session("a1", 80, 24).await.unwrap();
        rx.recv().await.unwrap();

        assert!(registry.claim_console_session(&session_id, "a2").is_none());
        assert!(registry.claim_console_session(&session_id, "a1").is_none());
    }

    #[tokio::test]
    async fn expire_console_session_removes_unclaimed_entry() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");
        let (session_id, ..) = registry.open_console_session("a1", 80, 24).await.unwrap();
        rx.recv().await.unwrap();

        assert!(registry.expire_console_session(&session_id));
        assert!(!registry.expire_console_session(&session_id));
        assert!(registry.claim_console_session(&session_id, "a1").is_none());
    }

    #[tokio::test]
    async fn open_tunnel_session_unknown_agent_returns_error() {
        let registry = MachineRegistry::new();
        let e = registry.open_tunnel_session("nonexistent", 8080).await.unwrap_err();
        assert!(e.contains("not connected"), "got: {e}");
    }

    #[tokio::test]
    async fn open_tunnel_session_sends_open_tunnel_message() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");

        let (session_id, _to_caller_rx, _to_agent_tx) =
            registry.open_tunnel_session("a1", 8080).await.unwrap();

        match rx.recv().await.unwrap() {
            ServerToAgent::OpenTunnel { session_id: sid, port } => {
                assert_eq!(sid, session_id);
                assert_eq!(port, 8080);
            }
            other => panic!("expected OpenTunnel, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn claim_tunnel_session_succeeds_once_for_matching_agent() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");
        let (session_id, ..) = registry.open_tunnel_session("a1", 8080).await.unwrap();
        rx.recv().await.unwrap();

        assert!(registry.claim_tunnel_session(&session_id, "a1").is_some());
        assert!(registry.claim_tunnel_session(&session_id, "a1").is_none());
    }

    #[tokio::test]
    async fn claim_tunnel_session_rejects_mismatched_agent() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");
        let (session_id, ..) = registry.open_tunnel_session("a1", 8080).await.unwrap();
        rx.recv().await.unwrap();

        assert!(registry.claim_tunnel_session(&session_id, "a2").is_none());
        assert!(registry.claim_tunnel_session(&session_id, "a1").is_none());
    }

    #[tokio::test]
    async fn expire_tunnel_session_removes_unclaimed_entry() {
        let registry = MachineRegistry::new();
        let mut rx = register_agent(&registry, "a1");
        let (session_id, ..) = registry.open_tunnel_session("a1", 8080).await.unwrap();
        rx.recv().await.unwrap();

        assert!(registry.expire_tunnel_session(&session_id));
        assert!(!registry.expire_tunnel_session(&session_id));
        assert!(registry.claim_tunnel_session(&session_id, "a1").is_none());
    }

    #[test]
    fn disconnect_if_current_removes_matching_connection() {
        let registry = MachineRegistry::new();
        let (tx, _rx) = mpsc::channel::<ServerToAgent>(8);
        registry.agents.insert("a1".into(), ConnectedAgent {
            id:           "a1".into(),
            project_id:   "proj-1".into(),
            hostname:     "host-1".into(),
            connected_at: Utc::now(),
            sender:       tx.clone(),
        });

        assert!(registry.disconnect_if_current("a1", &tx));
        assert!(registry.agents.get("a1").is_none());
    }

    #[test]
    fn disconnect_if_current_ignores_stale_connection_after_reconnect() {
        let registry = MachineRegistry::new();
        let (old_tx, _old_rx) = mpsc::channel::<ServerToAgent>(8);
        let (new_tx, _new_rx) = mpsc::channel::<ServerToAgent>(8);

        registry.agents.insert("a1".into(), ConnectedAgent {
            id:           "a1".into(),
            project_id:   "proj-1".into(),
            hostname:     "host-1".into(),
            connected_at: Utc::now(),
            sender:       old_tx.clone(),
        });

        registry.agents.insert("a1".into(), ConnectedAgent {
            id:           "a1".into(),
            project_id:   "proj-1".into(),
            hostname:     "host-1".into(),
            connected_at: Utc::now(),
            sender:       new_tx.clone(),
        });

        assert!(!registry.disconnect_if_current("a1", &old_tx));
        assert!(registry.agents.get("a1").is_some(), "new connection must survive the stale guard's drop");
        assert!(registry.agents.get("a1").unwrap().sender.same_channel(&new_tx));
    }

    #[test]
    fn disconnect_if_current_returns_false_for_unknown_agent() {
        let registry = MachineRegistry::new();
        let (tx, _rx) = mpsc::channel::<ServerToAgent>(8);
        assert!(!registry.disconnect_if_current("nonexistent", &tx));
    }
}
