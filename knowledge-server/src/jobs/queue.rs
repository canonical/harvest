use std::sync::Arc;

use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

const MAX_ATTEMPTS: i64 = 3;

#[derive(Clone, Debug, PartialEq)]
pub struct WorkItem {
    pub id: i64,
    pub kind: String,
    pub key: String,
    pub payload: Value,
    pub attempts: i64,
}

#[derive(Clone)]
pub struct WorkQueue {
    db: Arc<Db>,
}

const CLAIM_SQL: &str = r#"
UPDATE work_queue
   SET status = 'running', claimed_by = $node, claimed_at = now(), attempts = attempts + 1
 WHERE id IN (
       SELECT id FROM work_queue
        WHERE kind = $kind AND status = 'pending'
        ORDER BY id
        FOR UPDATE SKIP LOCKED
        LIMIT $limit)
RETURNING id, kind, key, payload, attempts
"#;

const FAIL_SQL: &str = r#"
UPDATE work_queue
   SET status = CASE WHEN attempts >= $max THEN 'failed' ELSE 'pending' END,
       claimed_by = NULL, claimed_at = NULL, error = $error
 WHERE id = $id
"#;

const REAP_SQL: &str = r#"
UPDATE work_queue
   SET status = 'pending', claimed_by = NULL, claimed_at = NULL
 WHERE status = 'running' AND NOT (claimed_by = ANY($alive::text[]))
"#;

fn item(row: &Value) -> WorkItem {
    WorkItem {
        id: row["id"].as_i64().unwrap_or(0),
        kind: row["kind"].as_str().unwrap_or_default().to_string(),
        key: row["key"].as_str().unwrap_or_default().to_string(),
        payload: row["payload"].clone(),
        attempts: row["attempts"].as_i64().unwrap_or(0),
    }
}

impl WorkQueue {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Arc<Db> {
        &self.db
    }

    pub async fn enqueue(&self, kind: &str, key: &str, payload: Value) -> Result<bool> {
        let inserted = self
            .db
            .execute(
                "INSERT INTO work_queue (kind, key, payload) VALUES ($kind, $key, $payload)
                 ON CONFLICT (kind, key) DO UPDATE
                    SET payload = EXCLUDED.payload, status = 'pending', attempts = 0, error = NULL
                  WHERE work_queue.status IN ('done', 'failed')
                    AND work_queue.payload IS DISTINCT FROM EXCLUDED.payload",
                json!({ "kind": kind, "key": key, "payload": payload }),
            )
            .await?;
        Ok(inserted > 0)
    }

    pub async fn claim(&self, kind: &str, node: &str, limit: usize) -> Result<Vec<WorkItem>> {
        Ok(self
            .db
            .query(
                CLAIM_SQL,
                json!({ "kind": kind, "node": node, "limit": limit as i64 }),
            )
            .await?
            .iter()
            .map(item)
            .collect())
    }

    pub async fn complete(&self, id: i64) -> Result<()> {
        self.db
            .execute(
                "UPDATE work_queue SET status = 'done', done_at = now(), error = NULL WHERE id = $id",
                json!({ "id": id }),
            )
            .await?;
        Ok(())
    }

    pub async fn fail(&self, id: i64, error: &str) -> Result<()> {
        self.db
            .execute(
                FAIL_SQL,
                json!({ "id": id, "error": error, "max": MAX_ATTEMPTS }),
            )
            .await?;
        Ok(())
    }

    pub async fn reap(&self, alive: &[String]) -> Result<u64> {
        self.db.execute(REAP_SQL, json!({ "alive": alive })).await
    }

    pub async fn depth(&self, kind: &str) -> Result<i64> {
        Ok(self
            .db
            .query(
                "SELECT count(*) AS n FROM work_queue WHERE kind = $kind AND status IN ('pending', 'running')",
                json!({ "kind": kind }),
            )
            .await?
            .first()
            .and_then(|r| r["n"].as_i64())
            .unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn items_are_claimed_once_retried_and_reaped_from_dead_nodes() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let queue = WorkQueue::new(Arc::new(test_db.db.clone()));
        assert!(queue.enqueue("enrich", "a", json!({"n": 1})).await.unwrap());
        assert!(!queue.enqueue("enrich", "a", json!({"n": 1})).await.unwrap());
        queue.enqueue("enrich", "b", json!({"n": 2})).await.unwrap();

        let first = queue.claim("enrich", "node-1", 1).await.unwrap();
        let second = queue.claim("enrich", "node-2", 5).await.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].key, second[0].key);
        assert!(queue.claim("enrich", "node-3", 5).await.unwrap().is_empty());

        queue.fail(first[0].id, "boom").await.unwrap();
        let retried = queue.claim("enrich", "node-3", 5).await.unwrap();
        assert_eq!(retried[0].attempts, 2);

        assert_eq!(queue.reap(&["node-3".to_string()]).await.unwrap(), 1);
        assert_eq!(queue.depth("enrich").await.unwrap(), 2);
        let again = queue.claim("enrich", "node-3", 5).await.unwrap();
        assert_eq!(again.len(), 1);
        for item in retried.iter().chain(&again) {
            queue.complete(item.id).await.unwrap();
        }
        assert_eq!(queue.depth("enrich").await.unwrap(), 0);
        assert!(
            queue.enqueue("enrich", "a", json!({"n": 3})).await.unwrap(),
            "changed payloads re-run"
        );
    }
}
