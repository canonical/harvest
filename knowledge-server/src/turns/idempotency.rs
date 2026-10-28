use std::time::Duration;

use anyhow::{Context, Result};
use axum::http::HeaderMap;
use harvest_db::Db;
use serde_json::json;

pub const HEADER: &str = "idempotency-key";
const MAX_KEY_LEN: usize = 200;

#[derive(Debug, PartialEq, Eq)]
pub enum Reservation {
    Fresh,
    Replay { turn_id: String },
}

pub fn key_from(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|k| !k.is_empty() && k.len() <= MAX_KEY_LEN)
        .map(str::to_string)
}

pub async fn reserve(
    db: &Db,
    user_id: &str,
    key: &str,
    conv_id: &str,
    turn_id: &str,
) -> Result<Reservation> {
    let rows = db
        .query(
            "WITH inserted AS (
                 INSERT INTO turn_requests (user_id, key, conv_id, turn_id)
                 VALUES ($uid, $key, $cid, $tid)
                 ON CONFLICT (user_id, key) DO NOTHING
                 RETURNING turn_id, true AS fresh
             )
             SELECT turn_id, fresh FROM inserted
             UNION ALL
             SELECT turn_id, false AS fresh FROM turn_requests
             WHERE user_id = $uid AND key = $key AND NOT EXISTS (SELECT 1 FROM inserted)",
            json!({ "uid": user_id, "key": key, "cid": conv_id, "tid": turn_id }),
        )
        .await
        .context("reserving the turn request")?;
    let row = rows
        .into_iter()
        .next()
        .context("the turn request vanished while being reserved")?;
    if row["fresh"].as_bool() == Some(true) {
        return Ok(Reservation::Fresh);
    }
    Ok(Reservation::Replay {
        turn_id: row["turn_id"].as_str().unwrap_or_default().to_string(),
    })
}

pub async fn release(db: &Db, user_id: &str, key: &str) {
    let released = db
        .execute(
            "DELETE FROM turn_requests WHERE user_id = $uid AND key = $key",
            json!({ "uid": user_id, "key": key }),
        )
        .await;
    if let Err(e) = released {
        tracing::warn!(error = %e, "could not release a turn request");
    }
}

pub async fn purge(db: &Db, older_than: Duration) -> Result<u64> {
    db.execute(
        "DELETE FROM turn_requests WHERE created_at < now() - make_interval(secs => $secs::float8)",
        json!({ "secs": older_than.as_secs_f64() }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use harvest_db::test_support::TestDb;

    #[test]
    fn keys_are_read_from_the_header_and_bounded() {
        let mut headers = HeaderMap::new();
        assert_eq!(key_from(&headers), None);
        headers.insert(HEADER, HeaderValue::from_static("  abc  "));
        assert_eq!(key_from(&headers).as_deref(), Some("abc"));
        headers.insert(HEADER, HeaderValue::from_str(&"x".repeat(201)).unwrap());
        assert_eq!(key_from(&headers), None);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn a_repeated_key_replays_the_first_turn() {
        let test_db = TestDb::new().await;
        let db = &test_db.db;
        assert_eq!(
            reserve(db, "u", "k", "c", "t1").await.unwrap(),
            Reservation::Fresh
        );
        assert_eq!(
            reserve(db, "u", "k", "c", "t2").await.unwrap(),
            Reservation::Replay {
                turn_id: "t1".into()
            }
        );
        assert_eq!(
            reserve(db, "other", "k", "c", "t3").await.unwrap(),
            Reservation::Fresh
        );
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn a_released_key_can_be_used_again() {
        let test_db = TestDb::new().await;
        let db = &test_db.db;
        reserve(db, "u", "k", "c", "t1").await.unwrap();
        release(db, "u", "k").await;
        assert_eq!(
            reserve(db, "u", "k", "c", "t2").await.unwrap(),
            Reservation::Fresh
        );
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn old_requests_are_purged() {
        let test_db = TestDb::new().await;
        let db = &test_db.db;
        reserve(db, "u", "k", "c", "t1").await.unwrap();
        assert_eq!(purge(db, Duration::from_secs(3600)).await.unwrap(), 0);
        assert_eq!(purge(db, Duration::ZERO).await.unwrap(), 1);
    }
}
