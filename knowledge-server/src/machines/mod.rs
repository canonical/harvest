pub mod handlers;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Instant};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;


// ── Wire protocol ──────────────────────────────────────────────────────────────

/// Body of POST /agent/results
#[derive(Debug, Deserialize)]
pub struct ResultBody {
    pub request_id: String,
    pub stdout:     String,
    pub stderr:     String,
    pub exit_code:  i32,
}

/// SSE events pushed from server → agent.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerToAgent {
    Registered { agent_token: String },
    HelloAck,
    Execute    { request_id: String, command: String, timeout_secs: u64 },
    Error      { message: String },
}

// ── Registry types ─────────────────────────────────────────────────────────────

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

// ── MachineRegistry ────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct MachineRegistry {
    pub agents:      DashMap<String, ConnectedAgent>,
    pub pending:     DashMap<String, PendingResult>,
    /// Maps SHA-256(permanent_agent_token) → agent_id.
    /// Populated when an agent opens its SSE connection; cleared on disconnect.
    /// Lets POST /agent/ping and /agent/results identify the caller in O(1)
    /// without a DB round-trip on every request.
    pub token_index: DashMap<String, String>,
}

impl MachineRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn agents_for_project(&self, project_id: &str) -> Vec<serde_json::Value> {
        self.agents
            .iter()
            .filter(|e| e.value().project_id == project_id)
            .map(|e| {
                let a = e.value();
                serde_json::json!({
                    "id":           a.id,
                    "hostname":     a.hostname,
                    "online":       true,
                    "connected_at": a.connected_at.to_rfc3339(),
                })
            })
            .collect()
    }

    /// Send an execute command to an agent and wait for the result.
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
        let (tx, rx)   = oneshot::channel();

        self.pending.insert(request_id.clone(), PendingResult {
            tx,
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
        tokio::time::timeout(wait, rx)
            .await
            .map_err(|_| "timed out waiting for command result".to_string())?
            .map_err(|_| "result channel closed".to_string())?
    }
}

// ── Token hashing ──────────────────────────────────────────────────────────────

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize()
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
}
