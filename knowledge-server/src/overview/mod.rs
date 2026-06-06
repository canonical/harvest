pub mod handlers;
pub mod pipeline;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::neo4j::Neo4jClient;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectOverview {
    pub env_doc:                   Option<String>,
    pub env_doc_updated_at:        Option<String>,
    pub current_status:            Option<String>,
    pub current_status_updated_at: Option<String>,
}

pub async fn get(neo4j: &Neo4jClient, project_id: &str) -> Result<ProjectOverview> {
    let rows = neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         RETURN p.env_doc                   AS env_doc,
                p.env_doc_updated_at        AS env_doc_updated_at,
                p.overview_status           AS current_status,
                p.overview_status_updated_at AS current_status_updated_at",
        json!({ "pid": project_id }),
    ).await?;

    let row = rows.into_iter().next().unwrap_or(Value::Null);
    Ok(ProjectOverview {
        env_doc:                   row.get("env_doc").and_then(|v| v.as_str()).map(String::from),
        env_doc_updated_at:        row.get("env_doc_updated_at").and_then(|v| v.as_str()).map(String::from),
        current_status:            row.get("current_status").and_then(|v| v.as_str()).map(String::from),
        current_status_updated_at: row.get("current_status_updated_at").and_then(|v| v.as_str()).map(String::from),
    })
}

pub async fn save(
    neo4j:          &Neo4jClient,
    project_id:     &str,
    env_doc:        &str,
    current_status: &str,
) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         SET p.env_doc                    = $env_doc,
             p.env_doc_updated_at         = $now,
             p.overview_status            = $status,
             p.overview_status_updated_at = $now
         RETURN p.id",
        json!({
            "pid":     project_id,
            "env_doc": env_doc,
            "status":  current_status,
            "now":     now,
        }),
    ).await?;
    Ok(())
}

pub async fn conversation_count(neo4j: &Neo4jClient, project_id: &str) -> Result<i64> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation)
         RETURN count(c) AS n",
        json!({ "pid": project_id }),
    ).await?;
    let n = rows.into_iter().next()
        .and_then(|r| r.get("n").cloned())
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Ok(n)
}

pub async fn all_conversations_text(neo4j: &Arc<Neo4jClient>, project_id: &str) -> Result<String> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_CONVERSATION]->(c:Conversation)
         RETURN c.title AS title, c.messages AS messages
         ORDER BY c.updated_at DESC
         LIMIT 20",
        json!({ "pid": project_id }),
    ).await?;

    let mut parts = Vec::new();
    for row in rows {
        let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("Untitled");
        let messages_str = row.get("messages").and_then(|v| v.as_str()).unwrap_or("[]");
        let messages: Vec<Value> = serde_json::from_str(messages_str).unwrap_or_default();
        let mut convo = format!("## {title}\n");
        for msg in messages {
            let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
            let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("");
            convo.push_str(&format!("**{role}**: {text}\n\n"));
        }
        parts.push(convo);
    }
    Ok(parts.join("\n---\n\n"))
}
