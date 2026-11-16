use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use harvest_db::Db;

pub const NEW_CHAT_TITLE: &str = "New chat";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

fn from_row(row: &Value) -> Chat {
    let text = |key: &str| row[key].as_str().unwrap_or_default().to_string();
    Chat {
        id: text("id"),
        title: row["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(NEW_CHAT_TITLE).to_string(),
        updated_at: text("updated_at"),
    }
}

pub async fn list(db: &Db, session_id: &str) -> Result<Vec<Chat>> {
    let rows = db
        .query(
            "SELECT id, title, updated_at FROM conversations
             WHERE code_session_id = $sid ORDER BY updated_at DESC, created_at DESC",
            json!({ "sid": session_id }),
        )
        .await?;
    Ok(rows.iter().map(from_row).collect())
}

pub async fn create(db: &Db, session_id: &str, user_id: &str, title: &str) -> Result<Chat> {
    let id = Uuid::new_v4().to_string();
    let now = harvest_db::now_rfc3339();
    let rows = db
        .query(
            "INSERT INTO conversations (id, user_id, title, created_by, created_at, updated_at, code_session_id)
             VALUES ($id, $uid, $title, $uid, $now, $now, $sid)
             RETURNING id, title, updated_at",
            json!({ "id": id, "uid": user_id, "title": title, "now": now, "sid": session_id }),
        )
        .await?;
    rows.first().map(from_row).context("the chat was not created")
}

pub async fn belongs(db: &Db, session_id: &str, chat_id: &str) -> Result<bool> {
    let rows = db
        .query(
            "SELECT 1 AS ok FROM conversations WHERE id = $cid AND code_session_id = $sid",
            json!({ "cid": chat_id, "sid": session_id }),
        )
        .await?;
    Ok(!rows.is_empty())
}

pub async fn delete(db: &Db, session_id: &str, chat_id: &str) -> Result<bool> {
    let deleted = db
        .execute(
            "DELETE FROM conversations WHERE id = $cid AND code_session_id = $sid",
            json!({ "cid": chat_id, "sid": session_id }),
        )
        .await?;
    Ok(deleted > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untitled_chats_get_a_readable_title() {
        let chat = from_row(&json!({ "id": "c1", "title": null, "updated_at": "t" }));
        assert_eq!(chat.title, NEW_CHAT_TITLE);
        let named = from_row(&json!({ "id": "c2", "title": "Fix login", "updated_at": "t" }));
        assert_eq!(named.title, "Fix login");
    }
}
