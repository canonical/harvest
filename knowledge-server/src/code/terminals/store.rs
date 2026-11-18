use anyhow::Result;
use serde_json::{json, Value};

use harvest_db::Db;

use super::model::{Terminal, TerminalId};

pub const MAX_PER_SESSION: i64 = 8;
const CREATE_ATTEMPTS: usize = 3;
const COLUMNS: &str = "id, title, created_at, last_attached_at";

fn from_row(row: &Value) -> Terminal {
    let text = |key: &str| row[key].as_str().map(String::from);
    Terminal {
        id: text("id").unwrap_or_default(),
        title: text("title").unwrap_or_default(),
        created_at: text("created_at").unwrap_or_default(),
        last_attached_at: text("last_attached_at"),
    }
}

pub async fn create(db: &Db, session_id: &str) -> Result<Option<Terminal>> {
    for _ in 0..CREATE_ATTEMPTS {
        let id = TerminalId::generate();
        let rows = db
            .query(
                &format!(
                    "INSERT INTO code_terminals (id, session_id, slot, title)
                     SELECT $id, $sid, free.slot, 'Terminal ' || free.slot
                     FROM generate_series(1, $max::int) AS free(slot)
                     WHERE free.slot NOT IN (SELECT slot FROM code_terminals WHERE session_id = $sid)
                     ORDER BY free.slot
                     LIMIT 1
                     ON CONFLICT (session_id, slot) DO NOTHING
                     RETURNING {COLUMNS}"
                ),
                json!({ "id": id.as_str(), "sid": session_id, "max": MAX_PER_SESSION }),
            )
            .await?;
        if let Some(row) = rows.first() {
            return Ok(Some(from_row(row)));
        }
        if count(db, session_id).await? >= MAX_PER_SESSION {
            return Ok(None);
        }
    }
    Ok(None)
}

async fn count(db: &Db, session_id: &str) -> Result<i64> {
    let rows = db
        .query("SELECT count(*) AS n FROM code_terminals WHERE session_id = $sid", json!({ "sid": session_id }))
        .await?;
    Ok(rows.first().and_then(|row| row["n"].as_i64()).unwrap_or_default())
}

pub async fn list(db: &Db, session_id: &str) -> Result<Vec<Terminal>> {
    let rows = db
        .query(
            &format!("SELECT {COLUMNS} FROM code_terminals WHERE session_id = $sid ORDER BY slot"),
            json!({ "sid": session_id }),
        )
        .await?;
    Ok(rows.iter().map(from_row).collect())
}

pub async fn get(db: &Db, session_id: &str, id: &TerminalId) -> Result<Option<Terminal>> {
    let rows = db
        .query(
            &format!("SELECT {COLUMNS} FROM code_terminals WHERE session_id = $sid AND id = $id"),
            json!({ "sid": session_id, "id": id.as_str() }),
        )
        .await?;
    Ok(rows.first().map(from_row))
}

pub async fn rename(db: &Db, session_id: &str, id: &TerminalId, title: &str) -> Result<Option<Terminal>> {
    let rows = db
        .query(
            &format!("UPDATE code_terminals SET title = $title WHERE session_id = $sid AND id = $id RETURNING {COLUMNS}"),
            json!({ "sid": session_id, "id": id.as_str(), "title": title }),
        )
        .await?;
    Ok(rows.first().map(from_row))
}

pub async fn remove(db: &Db, session_id: &str, id: &TerminalId) -> Result<bool> {
    let rows = db
        .query(
            "DELETE FROM code_terminals WHERE session_id = $sid AND id = $id RETURNING id",
            json!({ "sid": session_id, "id": id.as_str() }),
        )
        .await?;
    Ok(!rows.is_empty())
}

pub async fn mark_attached(db: &Db, id: &TerminalId) -> Result<()> {
    db.execute(
        "UPDATE code_terminals SET last_attached_at = now() WHERE id = $id",
        json!({ "id": id.as_str() }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn session(db: &Db) -> String {
        db.execute("INSERT INTO repositories (name, url) VALUES ('t', 'https://x/t')", json!({})).await.unwrap();
        db.execute(
            "INSERT INTO code_projects (id, repository_id) SELECT 'p-t', id FROM repositories WHERE name = 't'",
            json!({}),
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO users (id, email, provider) VALUES ('u-t', 'u@t', 'local')",
            json!({}),
        )
        .await
        .unwrap();
        db.execute(
            "INSERT INTO code_sessions (id, project_id, user_id, branch, base_branch, status)
             VALUES ('s-t', 'p-t', 'u-t', 'b', 'main', 'active')",
            json!({}),
        )
        .await
        .unwrap();
        "s-t".into()
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn creates_never_exceed_the_limit_and_number_their_titles() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = &test_db.db;
        let session_id = session(db).await;

        let created = futures::future::join_all((0..12).map(|_| create(db, &session_id))).await;
        let concurrent = created.into_iter().filter_map(|terminal| terminal.unwrap()).count() as i64;
        assert!((1..=MAX_PER_SESSION).contains(&concurrent));

        while create(db, &session_id).await.unwrap().is_some() {}

        let titles: Vec<String> = list(db, &session_id).await.unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, (1..=MAX_PER_SESSION).map(|n| format!("Terminal {n}")).collect::<Vec<_>>());
        assert_eq!(create(db, &session_id).await.unwrap(), None);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn a_freed_slot_is_reused_and_terminals_belong_to_their_session() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = &test_db.db;
        let session_id = session(db).await;
        let first = create(db, &session_id).await.unwrap().unwrap();
        let second = create(db, &session_id).await.unwrap().unwrap();
        let first_id = TerminalId::parse(&first.id).unwrap();

        assert!(get(db, "another-session", &first_id).await.unwrap().is_none());
        assert_eq!(rename(db, &session_id, &first_id, "server").await.unwrap().unwrap().title, "server");
        assert!(remove(db, &session_id, &first_id).await.unwrap());
        assert!(!remove(db, &session_id, &first_id).await.unwrap());

        let third = create(db, &session_id).await.unwrap().unwrap();
        assert_eq!(third.title, "Terminal 1");
        assert_ne!(third.id, second.id);
    }
}
