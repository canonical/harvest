pub mod bus;
pub mod node;
pub mod peer;
pub mod singleton;

use std::future::Future;
use std::sync::Arc;

use tokio::sync::watch;

#[derive(Clone)]
pub struct Shutdown(Arc<watch::Sender<bool>>);

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    pub fn new() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }

    pub fn trigger(&self) {
        self.0.send_replace(true);
    }

    pub fn is_triggered(&self) -> bool {
        *self.0.borrow()
    }

    pub fn wait(&self) -> impl Future<Output = ()> + Send + 'static {
        let mut receiver = self.0.subscribe();
        async move {
            let _ = receiver.wait_for(|triggered| *triggered).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn waiters_resolve_once_shutdown_is_triggered() {
        let shutdown = Shutdown::new();
        let waiter = tokio::spawn(shutdown.wait());
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        shutdown.trigger();
        tokio::time::timeout(Duration::from_secs(1), waiter).await.unwrap().unwrap();
        assert!(shutdown.is_triggered());
    }

    #[tokio::test]
    async fn waiting_after_the_trigger_resolves_immediately() {
        let shutdown = Shutdown::new();
        shutdown.trigger();
        tokio::time::timeout(Duration::from_millis(100), shutdown.wait()).await.unwrap();
    }
}
