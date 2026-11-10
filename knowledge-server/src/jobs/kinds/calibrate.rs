use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use harvest_db::Db;
use harvest_search::calibration::Calibrator;
use harvest_search::Scope;
use serde_json::json;

use crate::jobs::fence::Fence;
use crate::jobs::queue::{WorkItem, WorkQueue};
use crate::jobs::Job;
use crate::search::SearchService;

pub const KIND: &str = "calibrate";

const VERSIONS_SQL: &str = r#"
SELECT r.name AS repo, v.tag AS version, v.id, v.parser_version,
       (SELECT count(*) FROM symbols s WHERE s.version_id = v.id) AS symbols,
       (SELECT coalesce(max(s.id), 0) FROM symbols s WHERE s.version_id = v.id) AS max_symbol
FROM versions v
JOIN repositories r ON r.id = v.repository_id
WHERE v.ingested
"#;

const UPSERT_SQL: &str = r#"
INSERT INTO search_calibrations (version_id, reranker, gate, queries, metrics, created_at)
SELECT v.id, $reranker, $gate, $queries, $metrics, now()
FROM versions v
JOIN repositories r ON r.id = v.repository_id
WHERE r.name = $repo AND v.tag = $version
ON CONFLICT (version_id) DO UPDATE SET
    reranker = EXCLUDED.reranker, gate = EXCLUDED.gate, queries = EXCLUDED.queries,
    metrics = EXCLUDED.metrics, created_at = EXCLUDED.created_at
"#;

pub struct CalibrateJob {
    db: Arc<Db>,
    service: Arc<SearchService>,
}

impl CalibrateJob {
    pub fn new(db: Arc<Db>, service: Arc<SearchService>) -> Self {
        Self { db, service }
    }
}

#[async_trait]
impl Job for CalibrateJob {
    fn kind(&self) -> &'static str {
        KIND
    }

    fn batch(&self) -> usize {
        1
    }

    async fn produce(&self, queue: &WorkQueue) -> Result<usize> {
        let mut queued = 0;
        for version in self.db.query(VERSIONS_SQL, json!({})).await? {
            if version["symbols"].as_i64().unwrap_or(0) == 0 {
                continue;
            }
            let repo = version["repo"].as_str().unwrap_or_default();
            let tag = version["version"].as_str().unwrap_or_default();
            let payload = json!({
                "repo": repo,
                "version": tag,
                "content": format!(
                    "p{}-n{}-m{}",
                    version["parser_version"].as_i64().unwrap_or(0),
                    version["symbols"].as_i64().unwrap_or(0),
                    version["max_symbol"].as_i64().unwrap_or(0)
                ),
            });
            if queue
                .enqueue(KIND, &format!("{repo}@{tag}"), payload)
                .await?
            {
                queued += 1;
            }
        }
        Ok(queued)
    }

    async fn run(&self, item: &WorkItem, fence: &Fence) -> Result<()> {
        let repo = item.payload["repo"]
            .as_str()
            .context("a calibration item needs a repo")?;
        let version = item.payload["version"]
            .as_str()
            .context("a calibration item needs a version")?;
        let scope = Scope::new(repo, version);
        let Some(index) = self.service.registry.ensure(&scope).await? else {
            anyhow::bail!("the index for {repo}@{version} is still loading");
        };
        fence.check()?;
        let embedder = Arc::clone(self.service.registry.embedder());
        let watched = fence.clone();
        let calibrator = Calibrator::new(move |reranker| {
            crate::search::wiring::rounds_with(reranker, Arc::clone(&embedder))
        })
        .with_stop(Arc::new(move || watched.check().is_err()));
        let base = self.service.calibration_base.reranker.clone();
        let runtime = tokio::runtime::Handle::current();
        let outcome = tokio::task::spawn_blocking(move || {
            runtime.block_on(async move { calibrator.calibrate(index, base).await })
        })
        .await?;
        fence.check()?;
        let Some((calibration, report)) = outcome else {
            tracing::info!(
                repo,
                version,
                "no calibration produced (too few questions or out of time); keeping defaults"
            );
            return Ok(());
        };
        tracing::info!(
            repo,
            version,
            examples = report.examples,
            base_recall = report.base_recall,
            trained_recall = report.trained_recall,
            kept = report.kept_trained_reranker,
            "search calibrated"
        );
        let tx = self.db.begin().await?;
        tx.execute(
            UPSERT_SQL,
            json!({
                "repo": repo,
                "version": version,
                "reranker": serde_json::to_value(&calibration.reranker)?,
                "gate": serde_json::to_value(&calibration.gate)?,
                "queries": report.examples as i64,
                "metrics": serde_json::to_value(&report)?,
            }),
        )
        .await?;
        fence.commit(tx).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::lease::Lease;
    use crate::config::{SearchConfig, SearchMode};
    use crate::search::record::Recorder;

    async fn seed(db: &Db) {
        let mut sql = String::from(
            "INSERT INTO repositories (id, name) VALUES (1, 'r');
             INSERT INTO versions (id, repository_id, tag, ingested, parser_version) VALUES (1, 1, 'v1', true, 4);",
        );
        for i in 0..30 {
            sql.push_str(&format!(
                "INSERT INTO files (id, version_id, path, origin) VALUES ({id}, 1, 'pkg/mod{i}.py', 'source');
                 INSERT INTO symbols (id, file_id, version_id, label, name, signature, start_line, end_line, source, docstring)
                 VALUES ({id}, {id}, 1, 'Function', 'handle_step_{i}', 'def handle_step_{i}():', 1, 3,
                         'def handle_step_{i}():\n    return step_{i}()\n', 'Process the pending work item number {i} for the queue.');",
                id = i + 1
            ));
        }
        for i in 1..30 {
            sql.push_str(&format!(
                "INSERT INTO symbol_edges (src_id, dst_id, relation) VALUES ({}, {}, 'CALLS');",
                i,
                i + 1
            ));
        }
        db.batch_execute(&sql).await.unwrap();
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn calibration_is_stored_per_version_and_changes_the_index_generation() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = Arc::new(test_db.db.clone());
        seed(&db).await;
        let config = SearchConfig {
            engine: SearchMode::Tiered,
            ..SearchConfig::default()
        };
        let service =
            crate::search::wiring::build(&config, Arc::clone(&db), None, Recorder::metrics_only())
                .unwrap();
        let scope = Scope::new("r", "v1");
        let before = service.registry.ensure(&scope).await.unwrap().unwrap();
        assert!(before.calibration.is_none());

        let queue = WorkQueue::new(Arc::clone(&db));
        let job = CalibrateJob::new(Arc::clone(&db), Arc::clone(&service));
        assert_eq!(job.produce(&queue).await.unwrap(), 1);
        let items = queue.claim(KIND, "node", 10).await.unwrap();
        let fence = Fence::new(Arc::new(Lease::fixed(
            &format!("job:{KIND}:{}", items[0].key),
            3,
        )));
        job.run(&items[0], &fence).await.unwrap();
        queue.complete(items[0].id).await.unwrap();

        let stored = db
            .query(
                "SELECT queries, metrics FROM search_calibrations WHERE version_id = 1",
                json!({}),
            )
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert!(stored[0]["queries"].as_i64().unwrap() >= 20);
        assert_eq!(
            job.produce(&queue).await.unwrap(),
            0,
            "a calibration does not requeue itself"
        );

        assert!(service.registry.refresh(&scope).await.unwrap());
        let after = service.registry.get(&scope).unwrap();
        assert!(after.calibration.is_some());
        assert_ne!(before.generation, after.generation);
    }
}
