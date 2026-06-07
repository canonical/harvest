use serde_json::json;
use uuid::Uuid;

use crate::llm::{LlmProvider, types::Message};
use crate::neo4j::Neo4jClient;

struct MemorySummary {
    title:   String,
    excerpt: String,
}

pub async fn maybe_generate_memory(
    neo4j: &Neo4jClient,
    llm:   &dyn LlmProvider,
    project_id:       &str,
    user_query:       &str,
    assistant_answer: &str,
) {
    let existing = fetch_existing_memories(neo4j, project_id).await;

    if let Some((title, content)) = ask_llm(llm, user_query, assistant_answer, &existing).await {
        persist_memory(neo4j, project_id, &title, &content).await;
    }
}

async fn fetch_existing_memories(neo4j: &Neo4jClient, project_id: &str) -> Vec<MemorySummary> {
    let rows = neo4j.query_read(
        "MATCH (:Project {id: $pid})-[:HAS_MEMORY]->(m:Memory)
         RETURN m.title AS title, m.content AS content
         ORDER BY m.created_at DESC
         LIMIT 50",
        json!({ "pid": project_id }),
    ).await.unwrap_or_default();

    rows.into_iter().filter_map(|row| {
        let title   = row.get("title")?.as_str()?.to_string();
        let content = row.get("content")?.as_str().unwrap_or("");
        let excerpt: String = content.chars().take(200).collect();
        Some(MemorySummary { title, excerpt })
    }).collect()
}

async fn ask_llm(
    llm:              &dyn LlmProvider,
    user_query:       &str,
    assistant_answer: &str,
    existing:         &[MemorySummary],
) -> Option<(String, String)> {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();

    let existing_text = if existing.is_empty() {
        "(none)".to_string()
    } else {
        existing.iter().enumerate()
            .map(|(i, m)| format!("{}. {}\n   {}", i + 1, m.title, m.excerpt))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let system = format!(
        "You are a memory assistant for a software project. Decide if a conversation \
         contains information worth saving as a persistent project memory.\n\
         Current date/time: {now}\n\n\
         Save a memory when the conversation contains: decisions made, problems solved \
         and their resolution, important configuration or architecture choices, known \
         issues discovered, or facts that will be useful context in future conversations.\n\n\
         Do NOT save a memory for: exploratory or hypothetical questions, general \
         explanations with no project-specific outcome, trivial exchanges, or information \
         already covered by an existing memory listed below.\n\n\
         EXISTING MEMORIES:\n{existing_text}\n\n\
         Respond ONLY with a JSON object — no prose, no markdown fences. Either:\n\
         {{\"create\":true,\"title\":\"<concise title>\",\"content\":\"<markdown, begin with date>\"}}\n\
         or:\n\
         {{\"create\":false}}"
    );

    let messages = vec![
        Message::system(system),
        Message::user(format!("User: {user_query}\n\nAssistant: {assistant_answer}")),
    ];

    let text = match llm.chat(&messages, &[]).await {
        Ok(crate::llm::types::LlmResponse::Message { text }) => text,
        _ => return None,
    };

    let json = parse_json(&text)?;

    if json.get("create").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }

    let title   = json.get("title")?.as_str()?.trim().to_string();
    let content = json.get("content")?.as_str()?.trim().to_string();

    if title.is_empty() || content.is_empty() {
        return None;
    }

    Some((title, content))
}

fn parse_json(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str(trimmed) {
        return Some(v);
    }
    let start = trimmed.find('{')?;
    let end   = trimmed.rfind('}')?;
    serde_json::from_str(&trimmed[start..=end]).ok()
}

async fn persist_memory(neo4j: &Neo4jClient, project_id: &str, title: &str, content: &str) {
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let result = neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         CREATE (m:Memory {
             id: $id, title: $title, content: $content,
             created_by: 'system', created_at: $now, updated_at: $now
         })
         CREATE (p)-[:HAS_MEMORY]->(m)
         RETURN m.id AS id",
        json!({ "pid": project_id, "id": id, "title": title, "content": content, "now": now }),
    ).await;
    if let Err(e) = result {
        tracing::warn!(error = %e, "failed to persist auto-generated memory");
    }
}
