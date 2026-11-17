use anyhow::Result;
use serde_json::json;

use super::{sessions, CodeEnv};

const LISTED: i64 = 30;

pub async fn record(env: &CodeEnv, session_id: &str, command: &str) -> Result<()> {
    let Some(session) = sessions::get(env, session_id).await? else {
        return Ok(());
    };
    env.db
        .execute(
            "INSERT INTO code_learned_setup (project_id, command) VALUES ($pid, $command)
             ON CONFLICT (project_id, command) DO UPDATE SET learned_at = now()",
            json!({ "pid": session.project_id, "command": command.trim() }),
        )
        .await?;
    Ok(())
}

pub async fn list(env: &CodeEnv, project_id: &str) -> Result<Vec<String>> {
    let rows = env
        .db
        .query(
            "SELECT command FROM code_learned_setup WHERE project_id = $pid ORDER BY learned_at LIMIT $limit",
            json!({ "pid": project_id, "limit": LISTED }),
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| row["command"].as_str().map(String::from))
        .collect())
}

pub async fn forget(env: &CodeEnv, project_id: &str) -> Result<()> {
    env.db
        .execute("DELETE FROM code_learned_setup WHERE project_id = $pid", json!({ "pid": project_id }))
        .await?;
    Ok(())
}
