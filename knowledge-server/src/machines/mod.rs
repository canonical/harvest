pub mod handlers;
pub mod lxd_provision;

use axum::extract::ws::Message;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Instant};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ResultBody {
    pub request_id: String,
    pub stdout:     String,
    pub stderr:     String,
    pub exit_code:  i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToAgent {
    Registered { agent_token: String },
    HelloAck,
    Execute    { request_id: String, command: String, timeout_secs: u64 },
    OpenShell  { session_id: String, cols: u16, rows: u16 },
    Uninstall,
    Error      { message: String },
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

#[derive(Default)]
pub struct MachineRegistry {
    pub agents:          DashMap<String, ConnectedAgent>,
    pub pending:         DashMap<String, PendingResult>,
    pub token_index:     DashMap<String, String>,
    pub console_pending: DashMap<String, PendingConsoleSession>,
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

    pub async fn execute(
        &self,
        agent_id: &str,
        command:  String,
        timeout_secs: u64,
    ) -> Result<CommandResult, String> {
        let sender = self
            .agents
            .get(agent_id)
            .ok_or_else(|| format!("agent {agent_id} not connected"))?
            .sender
            .clone();

        let request_id = Uuid::new_v4().to_string();
        let (result_tx, result_rx) = oneshot::channel();

        self.pending.insert(request_id.clone(), PendingResult {
            tx:       result_tx,
            deadline: Instant::now() + std::time::Duration::from_secs(timeout_secs + 5),
        });

        sender
            .send(ServerToAgent::Execute {
                request_id: request_id.clone(),
                command,
                timeout_secs,
            })
            .await
            .map_err(|_| "agent disconnected before send".to_string())?;

        let wait = std::time::Duration::from_secs(timeout_secs + 10);
        tokio::time::timeout(wait, result_rx)
            .await
            .map_err(|_| "timed out waiting for command result".to_string())?
            .map_err(|_| "result channel closed".to_string())?
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
