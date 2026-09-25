use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::{mpsc, watch, RwLock};

use harvest_db::Db;
use knowledge_harvester::config::RepoConfig;
use knowledge_harvester::pipeline::Pipeline;

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IngestionStatus {
    Pending,
    Running,
    Completed { versions: Vec<String> },
    Failed { error: String },
}

#[derive(Clone)]
pub struct ProgressTracker {
    pub lines: Arc<RwLock<Vec<String>>>,
    pub watch_tx: watch::Sender<usize>,
}

impl ProgressTracker {
    pub fn new() -> (Self, mpsc::UnboundedSender<String>) {
        let lines = Arc::new(RwLock::new(Vec::new()));
        let (watch_tx, _) = watch::channel(0usize);
        let (mpsc_tx, mut mpsc_rx) = mpsc::unbounded_channel::<String>();

        let tracker = Self {
            lines: Arc::clone(&lines),
            watch_tx,
        };

        let watch_tx_clone = tracker.watch_tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = mpsc_rx.recv().await {
                let mut guard = lines.write().await;
                guard.push(msg);
                let count = guard.len();
                drop(guard);
                let _ = watch_tx_clone.send(count);
            }
        });

        (tracker, mpsc_tx)
    }

    pub fn subscribe(&self) -> watch::Receiver<usize> {
        self.watch_tx.subscribe()
    }

    pub async fn lines(&self) -> Vec<String> {
        self.lines.read().await.clone()
    }
}

pub struct IngestionJob {
    pub name: String,
    pub status: Arc<RwLock<IngestionStatus>>,
    pub started_at: DateTime<Utc>,
    pub progress: ProgressTracker,
}

pub type IngestionRegistry = Arc<RwLock<HashMap<String, Arc<IngestionJob>>>>;

pub fn new_registry() -> IngestionRegistry {
    Arc::new(RwLock::new(HashMap::new()))
}

pub async fn run_ingestion(
    registry: IngestionRegistry,
    db: Db,
    name: String,
    url: String,
    browse_url: Option<String>,
    refs: Vec<String>,
) {
    let (tracker, progress_tx) = ProgressTracker::new();

    {
        let mut jobs = registry.write().await;
        jobs.insert(
            name.clone(),
            Arc::new(IngestionJob {
                name: name.clone(),
                status: Arc::new(RwLock::new(IngestionStatus::Running)),
                started_at: Utc::now(),
                progress: tracker,
            }),
        );
    }

    let result = perform_ingestion(
        db,
        &name,
        &url,
        browse_url.as_deref(),
        &refs,
        progress_tx,
    )
    .await;

    let jobs = registry.read().await;
    if let Some(job) = jobs.get(&name) {
        let mut status = job.status.write().await;
        match result {
            Ok(versions) => {
                *status = IngestionStatus::Completed { versions };
            }
            Err(e) => {
                tracing::error!(repo = %name, error = %e, "ingestion failed");
                *status = IngestionStatus::Failed {
                    error: e.to_string(),
                };
            }
        }
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
    registry: IngestionRegistry,
    db: Db,
    name: String,
    url: String,
    version: String,
) {
    let (tracker, progress_tx) = ProgressTracker::new();

    {
        let mut jobs = registry.write().await;
        jobs.insert(
            name.clone(),
            Arc::new(IngestionJob {
                name: name.clone(),
                status: Arc::new(RwLock::new(IngestionStatus::Running)),
                started_at: Utc::now(),
                progress: tracker,
            }),
        );
    }

    let repo = RepoConfig {
        name: name.to_string(),
        url: url.to_string(),
        browse_url: None,
        refs: Some(vec![version.clone()]),
    };

    let mut pipeline = match Pipeline::with_db(db) {
        Ok(p) => p,
        Err(e) => {
            let jobs = registry.read().await;
            if let Some(job) = jobs.get(&name) {
                let mut status = job.status.write().await;
                *status = IngestionStatus::Failed { error: e.to_string() };
            }
            return;
        }
    };
    pipeline.set_progress_tx(progress_tx);

    let result = pipeline.process_single(&repo, true).await;

    let jobs = registry.read().await;
    if let Some(job) = jobs.get(&name) {
        let mut status = job.status.write().await;
        match result {
            Ok(_) => {
                *status = IngestionStatus::Completed { versions: vec![version] };
            }
            Err(e) => {
                tracing::error!(repo = %name, error = %e, "resync failed");
                *status = IngestionStatus::Failed { error: e.to_string() };
            }
        }
    }
}
