use anyhow::{Context, Result};
use harvest_db::Db;
use serde_json::{json, Value};

use crate::agent::HistoryMessage;
use crate::conversations::summary::StoredSummary;

#[derive(Clone, Copy, Debug)]
pub enum Owner<'a> {
    User(&'a str),
    Project(&'a str),
    Unscoped,
}

impl<'a> Owner<'a> {
    fn clause(self) -> &'static str {
        match self {
            Owner::User(_) => "c.user_id = $owner",
            Owner::Project(_) => "c.project_id = $owner",
            Owner::Unscoped => "TRUE",
        }
    }

    fn id(self) -> &'a str {
        match self {
            Owner::User(id) | Owner::Project(id) => id,
            Owner::Unscoped => "",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Guard<'a> {
    Unguarded,
    HoldingTurn(&'a str),
}

impl<'a> Guard<'a> {
    fn clause(self) -> &'static str {
        match self {
            Guard::Unguarded => "TRUE",
            Guard::HoldingTurn(_) => {
                "EXISTS (SELECT 1 FROM active_turns t WHERE t.conv_id = c.id AND t.turn_id = $turn)"
            }
        }
    }

    fn turn(self) -> &'a str {
        match self {
            Guard::Unguarded => "",
            Guard::HoldingTurn(turn_id) => turn_id,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct ConversationContext {
    pub summary: Option<StoredSummary>,
    pub recent: Vec<Value>,
    pub message_count: usize,
}

impl ConversationContext {
    pub fn history(&self) -> Vec<HistoryMessage> {
        let recent = history_messages_from_raw(&self.recent);
        let Some(summary) = &self.summary else {
            return recent;
        };
        let mut out = Vec::with_capacity(recent.len() + 1);
        out.push(HistoryMessage {
            role: "summary".into(),
            text: summary.text.clone(),
            ..Default::default()
        });
        out.extend(recent);
        out
    }

    pub fn recent_history(&self) -> Vec<HistoryMessage> {
        history_messages_from_raw(&self.recent)
    }
}

pub fn history_messages_from_raw(raw: &[Value]) -> Vec<HistoryMessage> {
    raw.iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect()
}

pub async fn load_context(db: &Db, owner: Owner<'_>, conv_id: &str) -> Result<ConversationContext> {
    let sql = format!(
        "WITH conv AS (
             SELECT c.id, c.summary, c.message_count,
                    CASE WHEN c.summary_upto <= c.message_count THEN c.summary_upto ELSE 0 END AS summary_upto
             FROM conversations c
             WHERE c.id = $cid AND {}
         )
         SELECT conv.summary, conv.summary_upto, conv.message_count,
                COALESCE((SELECT jsonb_agg(m.payload ORDER BY m.seq)
                          FROM conversation_messages m
                          WHERE m.conv_id = conv.id AND m.seq >= conv.summary_upto), '[]'::jsonb) AS recent
         FROM conv",
        owner.clause()
    );
    let rows = db
        .query(&sql, json!({ "cid": conv_id, "owner": owner.id() }))
        .await
        .context("loading conversation context")?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(ConversationContext::default());
    };
    Ok(ConversationContext {
        summary: StoredSummary::from_row(&row),
        recent: row["recent"].as_array().cloned().unwrap_or_default(),
        message_count: row["message_count"].as_u64().unwrap_or(0) as usize,
    })
}

pub async fn load_page(db: &Db, owner: Owner<'_>, conv_id: &str, page: Page) -> Result<Vec<Value>> {
    let sql = format!(
        "SELECT m.payload FROM (
             SELECT m.seq, m.payload
             FROM conversation_messages m
             JOIN conversations c ON c.id = m.conv_id
             WHERE m.conv_id = $cid AND {} AND m.seq < $before
             ORDER BY m.seq DESC
             LIMIT $limit
         ) m ORDER BY m.seq",
        owner.clause()
    );
    let rows = db
        .query(
            &sql,
            json!({
                "cid": conv_id,
                "owner": owner.id(),
                "before": page.before.map_or(i32::MAX as i64, |b| b as i64),
                "limit": page.limit.map_or(i64::MAX, |l| l as i64),
            }),
        )
        .await
        .context("loading conversation messages")?;
    Ok(rows.into_iter().map(|mut r| r["payload"].take()).collect())
}

#[derive(Debug, Default, Clone, Copy, serde::Deserialize)]
pub struct Page {
    pub before: Option<usize>,
    pub limit: Option<usize>,
}

impl Page {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn last(limit: usize) -> Self {
        Self {
            before: None,
            limit: Some(limit),
        }
    }
}

pub async fn append(
    db: &Db,
    owner: Owner<'_>,
    guard: Guard<'_>,
    conv_id: &str,
    messages: &[Value],
    now: &str,
) -> Result<bool> {
    let sql = format!(
        "WITH bumped AS (
             UPDATE conversations c
             SET message_count = c.message_count + $n, updated_at = $now
             WHERE c.id = $cid AND {} AND {}
             RETURNING c.message_count - $n AS start
         )
         INSERT INTO conversation_messages (conv_id, seq, payload)
         SELECT $cid, (bumped.start + m.ord - 1)::integer, m.value
         FROM bumped, jsonb_array_elements($messages::jsonb) WITH ORDINALITY AS m(value, ord)
         RETURNING seq",
        owner.clause(),
        guard.clause()
    );
    let inserted = db
        .query(
            &sql,
            json!({
                "cid": conv_id,
                "owner": owner.id(),
                "turn": guard.turn(),
                "n": messages.len() as i64,
                "now": now,
                "messages": messages,
            }),
        )
        .await
        .context("appending conversation messages")?;
    Ok(inserted.len() == messages.len() && !messages.is_empty())
}

pub async fn update_last<F>(
    db: &Db,
    owner: Owner<'_>,
    guard: Guard<'_>,
    conv_id: &str,
    now: &str,
    edit: F,
) -> Result<bool>
where
    F: FnOnce(&mut Value),
{
    let tx = db.begin().await?;
    let sql = format!(
        "SELECT m.seq, m.payload
         FROM conversation_messages m
         JOIN conversations c ON c.id = m.conv_id
         WHERE c.id = $cid AND {} AND {} AND m.seq = c.message_count - 1
         FOR UPDATE OF c, m",
        owner.clause(),
        guard.clause()
    );
    let rows = tx
        .query(
            &sql,
            json!({ "cid": conv_id, "owner": owner.id(), "turn": guard.turn() }),
        )
        .await
        .context("locking the last conversation message")?;
    let Some(mut row) = rows.into_iter().next() else {
        tx.rollback().await?;
        return Ok(false);
    };
    let mut payload = row["payload"].take();
    edit(&mut payload);
    tx.execute(
        "UPDATE conversation_messages SET payload = $payload WHERE conv_id = $cid AND seq = $seq",
        json!({ "cid": conv_id, "seq": row["seq"], "payload": payload }),
    )
    .await?;
    tx.execute(
        "UPDATE conversations SET updated_at = $now WHERE id = $cid",
        json!({ "cid": conv_id, "now": now }),
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn replace_all(
    db: &Db,
    owner: Owner<'_>,
    conv_id: &str,
    title: &str,
    messages: &[Value],
    now: &str,
) -> Result<bool> {
    let tx = db.begin().await?;
    let sql = format!(
        "UPDATE conversations c
         SET title = $title, message_count = $n, updated_at = $now, summary = NULL, summary_upto = 0
         WHERE c.id = $cid AND {}
         RETURNING c.id",
        owner.clause()
    );
    let updated = tx
        .query(
            &sql,
            json!({
                "cid": conv_id,
                "owner": owner.id(),
                "title": title,
                "n": messages.len() as i64,
                "now": now,
            }),
        )
        .await?;
    if updated.is_empty() {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM conversation_messages WHERE conv_id = $cid",
        json!({ "cid": conv_id }),
    )
    .await?;
    tx.execute(
        "INSERT INTO conversation_messages (conv_id, seq, payload)
         SELECT $cid, (m.ord - 1)::integer, m.value
         FROM jsonb_array_elements($messages::jsonb) WITH ORDINALITY AS m(value, ord)",
        json!({ "cid": conv_id, "messages": messages }),
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

pub async fn ensure_user_conversation(
    db: &Db,
    user_id: &str,
    conv_id: &str,
    title: &str,
    now: &str,
) -> Result<()> {
    db.execute(
        "INSERT INTO conversations (id, user_id, title, message_count, created_at, updated_at)
         VALUES ($cid, $uid, $title, 0, $now, $now)
         ON CONFLICT (id) DO NOTHING",
        json!({ "cid": conv_id, "uid": user_id, "title": title, "now": now }),
    )
    .await
    .context("creating the conversation")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harvest_db::test_support::TestDb;

    async fn user_conversation(db: &Db) -> (String, String) {
        let uid = uuid::Uuid::new_v4().to_string();
        let cid = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO users (id, provider) VALUES ($uid, 'local')",
            json!({ "uid": uid }),
        )
        .await
        .unwrap();
        ensure_user_conversation(db, &uid, &cid, "t", &harvest_db::now_rfc3339())
            .await
            .unwrap();
        (uid, cid)
    }

    fn message(role: &str, text: &str) -> Value {
        json!({ "role": role, "text": text })
    }

    fn texts(messages: &[Value]) -> Vec<&str> {
        messages
            .iter()
            .map(|m| m["text"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn history_prepends_the_summary_to_the_recent_messages() {
        let context = ConversationContext {
            summary: Some(StoredSummary {
                text: "earlier".into(),
                upto: 4,
            }),
            recent: vec![message("user", "q"), message("assistant", "a")],
            message_count: 6,
        };
        let history = context.history();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "summary");
        assert_eq!(history[0].text, "earlier");
        assert_eq!(history[2].text, "a");
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn appends_keep_order_and_count() {
        let test_db = TestDb::new().await;
        let (uid, cid) = user_conversation(&test_db.db).await;
        let now = harvest_db::now_rfc3339();
        assert!(append(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &[message("user", "1"), message("assistant", "2")],
            &now
        )
        .await
        .unwrap());
        assert!(append(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &[message("user", "3")],
            &now
        )
        .await
        .unwrap());

        let context = load_context(&test_db.db, Owner::User(&uid), &cid)
            .await
            .unwrap();
        assert_eq!(context.message_count, 3);
        assert_eq!(texts(&context.recent), vec!["1", "2", "3"]);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn appends_to_a_conversation_of_another_owner_are_refused() {
        let test_db = TestDb::new().await;
        let (_, cid) = user_conversation(&test_db.db).await;
        let now = harvest_db::now_rfc3339();
        assert!(!append(
            &test_db.db,
            Owner::User("someone-else"),
            Guard::Unguarded,
            &cid,
            &[message("user", "x")],
            &now
        )
        .await
        .unwrap());
        let context = load_context(&test_db.db, Owner::Unscoped, &cid)
            .await
            .unwrap();
        assert_eq!(context.message_count, 0);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn context_starts_at_the_summary() {
        let test_db = TestDb::new().await;
        let (uid, cid) = user_conversation(&test_db.db).await;
        let now = harvest_db::now_rfc3339();
        let messages: Vec<Value> = (0..6).map(|i| message("user", &i.to_string())).collect();
        append(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &messages,
            &now,
        )
        .await
        .unwrap();
        crate::conversations::summary::store(
            &test_db.db,
            &cid,
            &StoredSummary {
                text: "s".into(),
                upto: 4,
            },
        )
        .await;

        let context = load_context(&test_db.db, Owner::User(&uid), &cid)
            .await
            .unwrap();
        assert_eq!(texts(&context.recent), vec!["4", "5"]);
        assert_eq!(context.summary.unwrap().upto, 4);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn pages_walk_backwards_from_the_end() {
        let test_db = TestDb::new().await;
        let (uid, cid) = user_conversation(&test_db.db).await;
        let now = harvest_db::now_rfc3339();
        let messages: Vec<Value> = (0..5).map(|i| message("user", &i.to_string())).collect();
        append(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &messages,
            &now,
        )
        .await
        .unwrap();

        let last = load_page(&test_db.db, Owner::User(&uid), &cid, Page::last(2))
            .await
            .unwrap();
        assert_eq!(texts(&last), vec!["3", "4"]);
        let earlier = load_page(
            &test_db.db,
            Owner::User(&uid),
            &cid,
            Page {
                before: Some(3),
                limit: Some(2),
            },
        )
        .await
        .unwrap();
        assert_eq!(texts(&earlier), vec!["1", "2"]);
        let all = load_page(&test_db.db, Owner::User(&uid), &cid, Page::all())
            .await
            .unwrap();
        assert_eq!(all.len(), 5);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn update_last_edits_only_the_final_message() {
        let test_db = TestDb::new().await;
        let (uid, cid) = user_conversation(&test_db.db).await;
        let now = harvest_db::now_rfc3339();
        append(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &[message("user", "q"), message("assistant", "a")],
            &now,
        )
        .await
        .unwrap();

        assert!(update_last(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &now,
            |m| m["text"] = json!("edited")
        )
        .await
        .unwrap());

        let all = load_page(&test_db.db, Owner::User(&uid), &cid, Page::all())
            .await
            .unwrap();
        assert_eq!(texts(&all), vec!["q", "edited"]);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn a_turn_that_lost_its_lock_cannot_write() {
        let test_db = TestDb::new().await;
        let db = &test_db.db;
        db.execute("INSERT INTO groups (id, name) VALUES ('g', 'g')", json!({}))
            .await
            .unwrap();
        db.execute(
            "INSERT INTO projects (id, name, group_id, created_at) VALUES ('p', 'p', 'g', now())",
            json!({}),
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO conversations (id, project_id, title, message_count, created_at, updated_at)
             VALUES ('c', 'p', 't', 0, now(), now())",
            json!({}),
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO active_turns (conv_id, project_id, turn_id, node_id, locked_by)
             VALUES ('c', 'p', 'new-turn', 'n', 'u')",
            json!({}),
        )
        .await
        .unwrap();
        let now = harvest_db::now_rfc3339();
        let owner = Owner::Project("p");
        assert!(!append(
            db,
            owner,
            Guard::HoldingTurn("old-turn"),
            "c",
            &[message("user", "stale")],
            &now
        )
        .await
        .unwrap());
        assert!(append(
            db,
            owner,
            Guard::HoldingTurn("new-turn"),
            "c",
            &[message("user", "fresh")],
            &now
        )
        .await
        .unwrap());
        assert!(
            !update_last(db, owner, Guard::HoldingTurn("old-turn"), "c", &now, |m| {
                m["text"] = json!("x")
            })
            .await
            .unwrap()
        );
        let all = load_page(db, owner, "c", Page::all()).await.unwrap();
        assert_eq!(texts(&all), vec!["fresh"]);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn replace_all_resets_messages_and_summary() {
        let test_db = TestDb::new().await;
        let (uid, cid) = user_conversation(&test_db.db).await;
        let now = harvest_db::now_rfc3339();
        let messages: Vec<Value> = (0..4).map(|i| message("user", &i.to_string())).collect();
        append(
            &test_db.db,
            Owner::User(&uid),
            Guard::Unguarded,
            &cid,
            &messages,
            &now,
        )
        .await
        .unwrap();
        crate::conversations::summary::store(
            &test_db.db,
            &cid,
            &StoredSummary {
                text: "s".into(),
                upto: 2,
            },
        )
        .await;

        assert!(replace_all(
            &test_db.db,
            Owner::User(&uid),
            &cid,
            "new",
            &[message("user", "only")],
            &now
        )
        .await
        .unwrap());

        let context = load_context(&test_db.db, Owner::User(&uid), &cid)
            .await
            .unwrap();
        assert_eq!(context.message_count, 1);
        assert!(context.summary.is_none());
        assert_eq!(texts(&context.recent), vec!["only"]);
    }
}
