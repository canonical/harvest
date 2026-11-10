pub mod fence;
pub mod kinds;
pub mod queue;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::cluster::lease::Leases;
use crate::cluster::node::ClusterNode;
use crate::cluster::Shutdown;
use crate::metrics::{LEASES, WORK_ITEMS, WORK_QUEUE_DEPTH, WORK_QUEUE_REAPED};
use fence::Fence;
use queue::{WorkItem, WorkQueue};

#[async_trait]
pub trait Job: Send + Sync {
    fn kind(&self) -> &'static str;

    fn batch(&self) -> usize {
        4
    }

    async fn produce(&self, _queue: &WorkQueue) -> Result<usize> {
        Ok(0)
    }

    async fn run(&self, item: &WorkItem, fence: &Fence) -> Result<()>;
}

pub type Busy = Arc<dyn Fn() -> bool + Send + Sync>;

pub struct JobRunner {
    queue: WorkQueue,
    leases: Arc<dyn Leases>,
    node: Arc<ClusterNode>,
    jobs: Vec<Arc<dyn Job>>,
    interval: Duration,
    produce_every: u32,
    busy: Busy,
}

impl JobRunner {
    pub fn new(
        queue: WorkQueue,
        leases: Arc<dyn Leases>,
        node: Arc<ClusterNode>,
        jobs: Vec<Arc<dyn Job>>,
        busy: Busy,
    ) -> Self {
        Self {
            queue,
            leases,
            node,
            jobs,
            interval: Duration::from_secs(5),
            produce_every: 60,
            busy,
        }
    }

    pub fn with_interval(mut self, interval: Duration, produce_every: u32) -> Self {
        self.interval = interval;
        self.produce_every = produce_every.max(1);
        self
    }

    pub fn spawn(self, shutdown: Shutdown) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(self.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let stop = shutdown.wait();
            tokio::pin!(stop);
            let mut tick: u32 = 0;
            loop {
                tokio::select! {
                    _ = &mut stop => return,
                    _ = ticker.tick() => {
                        if let Err(e) = self.tick(tick % self.produce_every == 0).await {
                            tracing::warn!(error = %e, "background job tick failed");
                        }
                        tick = tick.wrapping_add(1);
                    }
                }
            }
        })
    }

    async fn exclusively<F, Fut>(&self, key: &str, work: F) -> Result<bool>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let Some(lease) = self.leases.try_acquire(key).await? else {
            LEASES.inc(&[("kind", key), ("result", "busy")]);
            return Ok(false);
        };
        LEASES.inc(&[("kind", key), ("result", "acquired")]);
        let outcome = tokio::select! {
            outcome = work() => outcome,
            _ = lease.lost() => Ok(()),
        };
        drop(lease);
        outcome.map(|_| true)
    }

    pub async fn tick(&self, produce: bool) -> Result<()> {
        self.exclusively("workqueue-reaper", || async {
            let alive = self.node.live_node_ids().await?;
            let reaped = self.queue.reap(&alive).await?;
            if reaped > 0 {
                WORK_QUEUE_REAPED.add(&[], reaped);
            }
            Ok(())
        })
        .await?;
        for job in &self.jobs {
            if produce {
                let producer = format!("produce:{}", job.kind());
                self.exclusively(&producer, || async {
                    let produced = job.produce(&self.queue).await?;
                    if produced > 0 {
                        tracing::info!(kind = job.kind(), produced, "queued background work");
                    }
                    Ok(())
                })
                .await?;
            }
            let depth = self.queue.depth(job.kind()).await?;
            WORK_QUEUE_DEPTH.set(&[("kind", job.kind())], depth.max(0) as u64);
            if (self.busy)() || self.node.is_fenced() || self.node.is_draining() {
                continue;
            }
            for item in self
                .queue
                .claim(job.kind(), self.node.node_id(), job.batch())
                .await?
            {
                self.run_item(job.as_ref(), &item).await?;
            }
        }
        Ok(())
    }

    async fn run_item(&self, job: &dyn Job, item: &WorkItem) -> Result<()> {
        let key = format!("job:{}:{}", item.kind, item.key);
        let Some(lease) = self.leases.try_acquire(&key).await? else {
            self.queue
                .fail(item.id, "another node holds this item")
                .await?;
            return Ok(());
        };
        let fence = Fence::new(Arc::new(lease));
        match job.run(item, &fence).await {
            Ok(()) => {
                WORK_ITEMS.inc(&[("kind", job.kind()), ("outcome", "done")]);
                self.queue.complete(item.id).await
            }
            Err(e) => {
                WORK_ITEMS.inc(&[("kind", job.kind()), ("outcome", "failed")]);
                tracing::warn!(error = %e, kind = job.kind(), key = item.key, "background job failed");
                self.queue.fail(item.id, &e.to_string()).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::lease;
    use crate::config::ClusterConfig;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting(Arc<AtomicUsize>);

    #[async_trait]
    impl Job for Counting {
        fn kind(&self) -> &'static str {
            "count"
        }

        fn batch(&self) -> usize {
            10
        }

        async fn run(&self, _item: &WorkItem, fence: &Fence) -> Result<()> {
            fence.check()?;
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn config(name: &str) -> ClusterConfig {
        ClusterConfig {
            node_name: Some(name.into()),
            heartbeat_interval_ms: 200,
            node_timeout_ms: 1_500,
            ..ClusterConfig::default()
        }
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn two_runners_process_each_item_once_and_reap_dead_claims() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = Arc::new(test_db.db.clone());
        let queue = WorkQueue::new(Arc::clone(&db));
        for key in ["a", "b", "c", "d"] {
            queue.enqueue("count", key, json!({})).await.unwrap();
        }
        queue
            .db()
            .execute(
                "UPDATE work_queue SET status = 'running', claimed_by = 'dead-node' WHERE key = 'd'",
                json!({}),
            )
            .await
            .unwrap();
        let runs = Arc::new(AtomicUsize::new(0));
        let mut runners = Vec::new();
        for name in ["a", "b"] {
            let node = ClusterNode::new(Arc::clone(&db), &config(name));
            node.heartbeat().await.unwrap();
            runners.push(JobRunner::new(
                queue.clone(),
                lease::for_node(&node, None),
                node,
                vec![Arc::new(Counting(Arc::clone(&runs)))],
                Arc::new(|| false),
            ));
        }
        let (first, second) = tokio::join!(runners[0].tick(false), runners[1].tick(false));
        first.unwrap();
        second.unwrap();
        for _ in 0..20 {
            if runs.load(Ordering::SeqCst) == 4 {
                break;
            }
            runners[0].tick(false).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(runs.load(Ordering::SeqCst), 4);
        assert_eq!(queue.depth("count").await.unwrap(), 0);
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn a_busy_node_claims_nothing() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = Arc::new(test_db.db.clone());
        let queue = WorkQueue::new(Arc::clone(&db));
        queue.enqueue("count", "a", json!({})).await.unwrap();
        let node = ClusterNode::new(Arc::clone(&db), &config("busy"));
        node.heartbeat().await.unwrap();
        let runs = Arc::new(AtomicUsize::new(0));
        let runner = JobRunner::new(
            queue.clone(),
            lease::for_node(&node, None),
            node,
            vec![Arc::new(Counting(Arc::clone(&runs)))],
            Arc::new(|| true),
        );
        runner.tick(false).await.unwrap();
        assert_eq!(runs.load(Ordering::SeqCst), 0);
        assert_eq!(queue.depth("count").await.unwrap(), 1);
    }
}
