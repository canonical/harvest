use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use harvest_db::Db;
use harvest_search::llm::LanguageModel;
use harvest_search::Deadline;
use serde_json::{json, Value};

use crate::jobs::fence::Fence;
use crate::jobs::queue::{WorkItem, WorkQueue};
use crate::jobs::Job;

pub const KIND: &str = "enrich";
const BATCH: usize = 20;
const SOURCE_CHARS: usize = 4_000;
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

const SYSTEM: &str = "You document code for a search index. For the symbol shown, reply with JSON only: {\"summary\": \"one or two sentences on what it does and why\", \"questions\": [three to five questions a developer might ask that this code answers], \"tags\": [three to six short concept tags]}.";

const VERSIONS_SQL: &str = "SELECT r.name AS repo, v.tag AS version, v.id FROM versions v JOIN repositories r ON r.id = v.repository_id WHERE v.ingested";

const REUSE_SQL: &str = r#"
INSERT INTO symbol_enrichments (symbol_id, source_hash, model, summary, questions, tags)
SELECT s.id, md5(s.source), e.model, e.summary, e.questions, e.tags
FROM symbols s
JOIN LATERAL (
    SELECT x.model, x.summary, x.questions, x.tags
    FROM symbol_enrichments x
    WHERE x.source_hash = md5(s.source)
    LIMIT 1
) e ON true
WHERE s.version_id = $vid AND s.source IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM symbol_enrichments y WHERE y.symbol_id = s.id AND y.source_hash = md5(s.source))
ON CONFLICT (symbol_id) DO UPDATE
   SET source_hash = EXCLUDED.source_hash, model = EXCLUDED.model, summary = EXCLUDED.summary,
       questions = EXCLUDED.questions, tags = EXCLUDED.tags, updated_at = now()
"#;

const REUSE_IDS_SQL: &str = r#"
INSERT INTO symbol_enrichments (symbol_id, source_hash, model, summary, questions, tags)
SELECT s.id, md5(s.source), e.model, e.summary, e.questions, e.tags
FROM symbols s
JOIN LATERAL (
    SELECT x.model, x.summary, x.questions, x.tags
    FROM symbol_enrichments x
    WHERE x.source_hash = md5(s.source)
    LIMIT 1
) e ON true
WHERE s.id = ANY($ids::bigint[])
ON CONFLICT (symbol_id) DO UPDATE
   SET source_hash = EXCLUDED.source_hash, model = EXCLUDED.model, summary = EXCLUDED.summary,
       questions = EXCLUDED.questions, tags = EXCLUDED.tags, updated_at = now()
"#;

const STALE_SQL: &str = r#"
SELECT s.id
FROM symbols s
LEFT JOIN symbol_enrichments e ON e.symbol_id = s.id
WHERE s.version_id = $vid AND s.source IS NOT NULL AND length(s.source) > 40
  AND (e.symbol_id IS NULL OR e.source_hash <> md5(s.source))
ORDER BY s.id
"#;

const SYMBOLS_SQL: &str = r#"
SELECT s.id, s.name, s.impl_type, s.label, coalesce(s.signature, '') AS signature,
       coalesce(s.docstring, '') AS docstring, left(s.source, $chars) AS source, f.path
FROM symbols s JOIN files f ON f.id = s.file_id
WHERE s.id = ANY($ids::bigint[])
  AND NOT EXISTS (SELECT 1 FROM symbol_enrichments e WHERE e.symbol_id = s.id AND e.source_hash = md5(s.source))
"#;

const UPSERT_SQL: &str = r#"
INSERT INTO symbol_enrichments (symbol_id, source_hash, model, summary, questions, tags, fencing_token)
SELECT s.id, md5(s.source), $model, $summary, $questions, $tags, $token
FROM symbols s WHERE s.id = $id
ON CONFLICT (symbol_id) DO UPDATE
   SET source_hash = EXCLUDED.source_hash, model = EXCLUDED.model, summary = EXCLUDED.summary,
       questions = EXCLUDED.questions, tags = EXCLUDED.tags, fencing_token = EXCLUDED.fencing_token,
       updated_at = now()
"#;

pub struct EnrichJob {
    db: Arc<Db>,
    model: Arc<dyn LanguageModel>,
    model_name: String,
}

impl EnrichJob {
    pub fn new(db: Arc<Db>, model: Arc<dyn LanguageModel>, model_name: impl Into<String>) -> Self {
        Self {
            db,
            model,
            model_name: model_name.into(),
        }
    }
}

fn strings(value: &Value, max: usize) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(max)
                .collect()
        })
        .unwrap_or_default()
}

pub fn prompt(row: &Value) -> String {
    let name = row["name"].as_str().unwrap_or_default();
    let qualified = match row["impl_type"].as_str().filter(|p| !p.is_empty()) {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_string(),
    };
    format!(
        "{} {} in {}\n{}\n{}\n\n{}",
        row["label"].as_str().unwrap_or("symbol").to_lowercase(),
        qualified,
        row["path"].as_str().unwrap_or_default(),
        row["signature"].as_str().unwrap_or_default(),
        row["docstring"].as_str().unwrap_or_default(),
        row["source"].as_str().unwrap_or_default()
    )
}

#[async_trait]
impl Job for EnrichJob {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn batch(&self) -> usize {
        2
    }

    async fn produce(&self, queue: &WorkQueue) -> Result<usize> {
        let mut queued = 0;
        for version in self.db.query(VERSIONS_SQL, json!({})).await? {
            let vid = version["id"].as_i64().unwrap_or(0);
            self.db.execute(REUSE_SQL, json!({ "vid": vid })).await?;
            let stale: Vec<i64> = self
                .db
                .query(STALE_SQL, json!({ "vid": vid }))
                .await?
                .iter()
                .filter_map(|r| r["id"].as_i64())
                .collect();
            for (n, batch) in stale.chunks(BATCH).enumerate() {
                let key = format!(
                    "{}@{}#{}-{}",
                    version["repo"].as_str().unwrap_or_default(),
                    version["version"].as_str().unwrap_or_default(),
                    n,
                    batch[0]
                );
                if queue.enqueue(KIND, &key, json!({ "ids": batch })).await? {
                    queued += 1;
                }
            }
        }
        Ok(queued)
    }

    async fn run(&self, item: &WorkItem, fence: &Fence) -> Result<()> {
        let ids: Vec<i64> = item.payload["ids"]
            .as_array()
            .context("an enrichment item needs ids")?
            .iter()
            .filter_map(Value::as_i64)
            .collect();
        self.db
            .execute(REUSE_IDS_SQL, json!({ "ids": ids }))
            .await?;
        let rows = self
            .db
            .query(
                SYMBOLS_SQL,
                json!({ "ids": ids, "chars": SOURCE_CHARS as i64 }),
            )
            .await?;
        let mut results = Vec::new();
        for row in &rows {
            fence.check()?;
            let reply = self
                .model
                .complete_json(SYSTEM, &prompt(row), Deadline::after(CALL_TIMEOUT))
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let summary = reply["summary"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            if summary.is_empty() {
                continue;
            }
            results.push((
                row["id"].as_i64().unwrap_or(0),
                summary,
                strings(&reply["questions"], 5),
                strings(&reply["tags"], 6),
            ));
        }
        let tx = self.db.begin().await?;
        for (id, summary, questions, tags) in results {
            tx.execute(
                UPSERT_SQL,
                json!({
                    "id": id, "model": self.model_name, "summary": summary,
                    "questions": questions, "tags": tags, "token": fence.token() as i64,
                }),
            )
            .await?;
        }
        fence.commit(tx).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::lease::Lease;
    use harvest_search::llm::ScriptedModel;

    async fn seed(db: &Db) {
        db.batch_execute(
            r#"
            INSERT INTO repositories (id, name) VALUES (1, 'r');
            INSERT INTO versions (id, repository_id, tag, ingested) VALUES (1, 1, 'v1', true), (2, 1, 'v2', true);
            INSERT INTO files (id, version_id, path) VALUES (1, 1, 'a.py'), (2, 2, 'a.py');
            INSERT INTO symbols (id, file_id, version_id, label, name, source) VALUES
                (10, 1, 1, 'Function', 'retry', 'def retry(n):\n    for i in range(n):\n        attempt(i)\n'),
                (20, 2, 2, 'Function', 'retry', 'def retry(n):\n    for i in range(n):\n        attempt(i)\n'),
                (21, 2, 2, 'Function', 'other', 'def other():\n    return compute_something_long()\n');
            "#,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn enrichment_runs_once_per_content_hash_and_reuses_it_across_versions() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = Arc::new(test_db.db.clone());
        seed(&db).await;
        let queue = WorkQueue::new(Arc::clone(&db));
        let model = Arc::new(ScriptedModel::new(vec![json!({
            "summary": "Retries an attempt n times.",
            "questions": ["How are attempts retried?"],
            "tags": ["retry"]
        })]));
        let job = EnrichJob::new(Arc::clone(&db), model.clone(), "scripted");
        assert!(job.produce(&queue).await.unwrap() >= 1);
        for item in queue.claim(KIND, "node", 10).await.unwrap() {
            let fence = Fence::new(Arc::new(Lease::fixed(
                &format!("job:{KIND}:{}", item.key),
                5,
            )));
            job.run(&item, &fence).await.unwrap();
            queue.complete(item.id).await.unwrap();
        }
        let enriched = db
            .query(
                "SELECT symbol_id, summary, questions FROM symbol_enrichments ORDER BY symbol_id",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(enriched.len(), 3);
        let calls_after_first_pass = model.prompts().len();
        assert_eq!(
            calls_after_first_pass, 2,
            "identical sources share one model call"
        );
        assert_eq!(
            job.produce(&queue).await.unwrap(),
            0,
            "nothing is stale any more"
        );

        let stale = Fence::new(Arc::new(Lease::fixed("job:enrich:x", 1)));
        let newer = Fence::new(Arc::new(Lease::fixed("job:enrich:x", 2)));
        newer.commit(db.begin().await.unwrap()).await.unwrap();
        assert!(matches!(
            stale.commit(db.begin().await.unwrap()).await,
            Err(crate::jobs::fence::Fenced::Superseded(_))
        ));
    }
}
