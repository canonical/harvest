use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

// ── ListSecretsTool ───────────────────────────────────────────────────────────

pub struct ListSecretsTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for ListSecretsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_secrets".into(),
            description: "List the names of all secrets stored for this project. \
                          Returns names only, not values. Use get_secret to retrieve a specific value."
                .into(),
            parameters: json!({
                "type":       "object",
                "properties": {},
                "required":   []
            }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let rows = self.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_SECRET]->(s:ProjectSecret)
             RETURN s.name AS name, s.created_at AS created_at
             ORDER BY s.name",
            json!({ "pid": self.project_id }),
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

// ── GetSecretTool ─────────────────────────────────────────────────────────────

pub struct GetSecretTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for GetSecretTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_secret".into(),
            description: "Retrieve the value of a named secret from this project's secret store."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type":        "string",
                        "description": "The secret name (case-insensitive)"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let name = params["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("name is required"))?
            .trim()
            .to_uppercase();

        let rows = self.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_SECRET]->(s:ProjectSecret {name: $name})
             RETURN s.value AS value",
            json!({ "pid": self.project_id, "name": name }),
        ).await?;

        match rows.first() {
            Some(row) => Ok(row["value"].as_str().unwrap_or("").to_string()),
            None      => Ok(format!("No secret named '{name}' found.")),
        }
    }

    fn preview(&self, _result: &str) -> String {
        "[secret value retrieved]".into()
    }
}

// ── SaveSecretTool ────────────────────────────────────────────────────────────

pub struct SaveSecretTool {
    pub neo4j:      Arc<Neo4jClient>,
    pub project_id: String,
}

#[async_trait]
impl Tool for SaveSecretTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "save_secret".into(),
            description: "Save a secret to this project's secret store. \
                          If the secret already exists with the same value it is left unchanged. \
                          If it exists with a different value it is overwritten. \
                          Secret names are normalized to UPPER_CASE. \
                          Use this whenever you discover a credential, token, or sensitive value \
                          that should be stored for future use."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type":        "string",
                        "description": "The secret name (e.g. GITHUB_TOKEN, API_KEY)"
                    },
                    "value": {
                        "type":        "string",
                        "description": "The secret value"
                    }
                },
                "required": ["name", "value"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let name = params["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("name is required"))?
            .trim()
            .to_uppercase();
        let value = params["value"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("value is required"))?;

        if name.is_empty() {
            anyhow::bail!("name cannot be empty");
        }

        // Check for existing secret first to implement the "same value → no-op" rule.
        let existing = self.neo4j.query_read(
            "MATCH (:Project {id: $pid})-[:HAS_SECRET]->(s:ProjectSecret {name: $name})
             RETURN s.value AS value",
            json!({ "pid": self.project_id, "name": name }),
        ).await?;

        if let Some(row) = existing.first() {
            let current = row["value"].as_str().unwrap_or("");
            if current == value {
                return Ok(format!("Secret '{name}' already exists with the same value — no change made."));
            }
            // Different value: overwrite.
            self.neo4j.query_read(
                "MATCH (:Project {id: $pid})-[:HAS_SECRET]->(s:ProjectSecret {name: $name})
                 SET s.value = $value
                 RETURN s.name",
                json!({ "pid": self.project_id, "name": name, "value": value }),
            ).await?;
            return Ok(format!("Secret '{name}' updated with new value."));
        }

        // Does not exist yet: create.
        let now = Utc::now().to_rfc3339();
        self.neo4j.query_read(
            "MATCH (p:Project {id: $pid})
             CREATE (p)-[:HAS_SECRET]->(s:ProjectSecret {name: $name, value: $value, created_at: $now})",
            json!({ "pid": self.project_id, "name": name, "value": value, "now": now }),
        ).await?;
        Ok(format!("Secret '{name}' saved."))
    }

    fn preview(&self, result: &str) -> String {
        result.to_string()
    }
}
