use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use harvest_db::Db;
use knowledge_harvester::config::RepoConfig;
use knowledge_harvester::pipeline::Pipeline;

use crate::cluster::node::ClusterNode;

const PROGRESS_FLUSH_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestionStatus {
    Pending,
    Running,
    Completed { versions: Vec<String> },
    Failed { error: String },
}

impl IngestionStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed { .. } => "completed",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }

    fn from_row(row: &Value) -> Self {
        match row["status"].as_str() {
            Some("pending") => Self::Pending,
            Some("running") => Self::Running,
            Some("completed") => Self::Completed {
                versions: row["versions"]
                    .as_array()
                    .map(|v| v.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                    .unwrap_or_default(),
            },
            _ => Self::Failed { error: row["error"].as_str().unwrap_or("unknown error").to_string() },
        }
    }
}

#[derive(Clone, Debug)]
pub struct IngestionJob {
    pub id:     String,
    pub repo:   String,
    pub status: IngestionStatus,
}

#[derive(Clone)]
pub struct IngestionJobs {
    db:   Arc<Db>,
    node: Arc<ClusterNode>,
}

pub type IngestionRegistry = IngestionJobs;

impl IngestionJobs {
    pub fn new(db: Arc<Db>, node: Arc<ClusterNode>) -> Self {
        Self { db, node }
    }

    pub fn standalone(db: Arc<Db>) -> Self {
        let node = ClusterNode::new(Arc::clone(&db), &crate::config::ClusterConfig::default());
        Self { db, node }
    }

    pub async fn try_start(&self, repo: &str, kind: &str) -> Result<Option<String>> {
        reap_dead_nodes(&self.db, &self.node).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let result = self.db.execute(
            "INSERT INTO ingestion_jobs (id, repo, kind, status, node_id) VALUES ($id, $repo, $kind, 'running', $node)",
            json!({ "id": id, "repo": repo, "kind": kind, "node": self.node.node_id() }),
        ).await;
        match result {
            Ok(_) => Ok(Some(id)),
            Err(e) if harvest_db::is_unique_violation(&e) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn latest(&self, repo: &str) -> Result<Option<IngestionJob>> {
        let rows = self.db.query(
            "SELECT id, repo, status, error, versions FROM ingestion_jobs WHERE repo = $repo ORDER BY started_at DESC LIMIT 1",
            json!({ "repo": repo }),
        ).await?;
        Ok(rows.first().map(|row| IngestionJob {
            id: row["id"].as_str().unwrap_or_default().to_string(),
            repo: repo.to_string(),
            status: IngestionStatus::from_row(row),
        }))
    }

    pub async fn status(&self, job_id: &str) -> Result<Option<IngestionStatus>> {
        let rows = self.db.query(
            "SELECT status, error, versions FROM ingestion_jobs WHERE id = $id",
            json!({ "id": job_id }),
        ).await?;
        Ok(rows.first().map(IngestionStatus::from_row))
    }

    pub async fn latest_statuses(&self) -> Result<Vec<(String, IngestionStatus)>> {
        let rows = self.db.query(
            "SELECT DISTINCT ON (repo) repo, status, error, versions FROM ingestion_jobs ORDER BY repo, started_at DESC",
            json!({}),
        ).await?;
        Ok(rows.iter().map(|r| (r["repo"].as_str().unwrap_or_default().to_string(), IngestionStatus::from_row(r))).collect())
    }

    pub async fn is_active(&self, repo: &str) -> Result<bool> {
        reap_dead_nodes(&self.db, &self.node).await?;
        Ok(self.latest(repo).await?.is_some_and(|j| j.status.is_active()))
    }

    pub async fn append(&self, job_id: &str, lines: &[String]) -> Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        self.db.execute(
            "INSERT INTO ingestion_progress (job_id, line) SELECT $id, unnest($lines::text[])",
            json!({ "id": job_id, "lines": lines }),
        ).await?;
        Ok(())
    }

    pub async fn progress_since(&self, job_id: &str, after: i64) -> Result<Vec<(i64, String)>> {
        let rows = self.db.query(
            "SELECT seq, line FROM ingestion_progress WHERE job_id = $id AND seq > $after ORDER BY seq",
            json!({ "id": job_id, "after": after }),
        ).await?;
        Ok(rows.iter().map(|r| (r["seq"].as_i64().unwrap_or(0), r["line"].as_str().unwrap_or_default().to_string())).collect())
    }

    pub async fn finish(&self, job_id: &str, outcome: Result<Vec<String>, String>) -> Result<()> {
        match outcome {
            Ok(versions) => self.db.execute(
                "UPDATE ingestion_jobs SET status = 'completed', versions = $versions, finished_at = now() WHERE id = $id",
                json!({ "id": job_id, "versions": versions }),
            ).await?,
            Err(error) => self.db.execute(
                "UPDATE ingestion_jobs SET status = 'failed', error = $error, finished_at = now() WHERE id = $id",
                json!({ "id": job_id, "error": error }),
            ).await?,
        };
        Ok(())
    }

    pub async fn forget(&self, repo: &str) -> Result<()> {
        self.db.execute(
            "DELETE FROM ingestion_jobs WHERE repo = $repo AND status NOT IN ('pending', 'running')",
            json!({ "repo": repo }),
        ).await?;
        Ok(())
    }

    fn progress_writer(&self, job_id: String) -> (mpsc::UnboundedSender<String>, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let jobs = self.clone();
        let handle = tokio::spawn(async move {
            let mut buffer = Vec::new();
            loop {
                let next = tokio::time::timeout(PROGRESS_FLUSH_INTERVAL, rx.recv()).await;
                match next {
                    Ok(Some(line)) => {
                        buffer.push(line);
                        continue;
                    }
                    Ok(None) => {
                        let _ = jobs.append(&job_id, &buffer).await;
                        return;
                    }
                    Err(_) => {
                        if let Err(e) = jobs.append(&job_id, &buffer).await {
                            tracing::warn!(error = %e, "failed to record ingestion progress");
                        }
                        buffer.clear();
                    }
                }
            }
        });
        (tx, handle)
    }
}

pub async fn reap_dead_nodes(db: &Db, node: &ClusterNode) -> Result<u64> {
    db.execute(
        "UPDATE ingestion_jobs j
         SET status = 'failed', error = 'interrupted: the server node running this ingestion was lost', finished_at = now()
         WHERE j.status IN ('pending', 'running') AND j.node_id <> $node AND NOT EXISTS (
             SELECT 1 FROM cluster_nodes n
             WHERE n.node_id = j.node_id
               AND n.heartbeat_at > now() - make_interval(secs => $timeout::float8))",
        json!({ "node": node.node_id(), "timeout": node.timeout_secs() }),
    ).await
}

pub async fn run_ingestion(
    jobs: IngestionJobs,
    job_id: String,
    db: Db,
    name: String,
    url: String,
    browse_url: Option<String>,
    refs: Vec<String>,
) {
    let (progress_tx, writer) = jobs.progress_writer(job_id.clone());
    let result = perform_ingestion(db, &name, &url, browse_url.as_deref(), &refs, progress_tx).await;
    let _ = writer.await;
    let outcome = result.map_err(|e| {
        tracing::error!(repo = %name, error = %e, "ingestion failed");
        e.to_string()
    });
    if let Err(e) = jobs.finish(&job_id, outcome).await {
        tracing::error!(repo = %name, error = %e, "failed to record ingestion outcome");
    }
}

async fn perform_ingestion(
    db: Db,
    name: &str,
    url: &str,
    browse_url: Option<&str>,
    refs: &[String],
    progress_tx: mpsc::UnboundedSender<String>,
) -> anyhow::Result<Vec<String>> {
    let repo = RepoConfig {
        name: name.to_string(),
        url: url.to_string(),
        browse_url: browse_url.map(String::from),
        refs: Some(refs.to_vec()),
    };

    let mut pipeline = Pipeline::with_db(db)?;
    pipeline.set_progress_tx(progress_tx);
    pipeline.process_single(&repo, false).await?;
    let versions = pipeline.ingested_versions(name).await?;
    Ok(versions)
}

pub async fn run_resync(
    jobs: IngestionJobs,
    job_id: String,
    db: Db,
    name: String,
    url: String,
    version: String,
) {
    let (progress_tx, writer) = jobs.progress_writer(job_id.clone());
    let repo = RepoConfig {
        name: name.to_string(),
        url: url.to_string(),
        browse_url: None,
        refs: Some(vec![version.clone()]),
    };
    let result = match Pipeline::with_db(db) {
        Ok(mut pipeline) => {
            pipeline.set_progress_tx(progress_tx);
            pipeline.process_single(&repo, true).await.map(|_| vec![version])
        }
        Err(e) => {
            drop(progress_tx);
            Err(e)
        }
    };
    let _ = writer.await;
    let outcome = result.map_err(|e| {
        tracing::error!(repo = %name, error = %e, "resync failed");
        e.to_string()
    });
    if let Err(e) = jobs.finish(&job_id, outcome).await {
        tracing::error!(repo = %name, error = %e, "failed to record resync outcome");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_from_rows() {
        assert_eq!(IngestionStatus::from_row(&json!({ "status": "running" })), IngestionStatus::Running);
        assert_eq!(
            IngestionStatus::from_row(&json!({ "status": "completed", "versions": ["v1"] })),
            IngestionStatus::Completed { versions: vec!["v1".into()] }
        );
        assert_eq!(
            IngestionStatus::from_row(&json!({ "status": "failed", "error": "boom" })),
            IngestionStatus::Failed { error: "boom".into() }
        );
        assert!(IngestionStatus::Pending.is_active());
        assert!(!IngestionStatus::Failed { error: String::new() }.is_active());
    }
}
