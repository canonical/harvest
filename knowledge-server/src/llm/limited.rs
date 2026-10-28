use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

use crate::llm::types::{LlmResponse, Message, ModelInfo, StreamEvent, ToolDefinition};
use crate::llm::LlmProvider;

const QUEUE_PATIENCE: Duration = Duration::from_secs(2);

pub struct ConcurrencyLimited {
    inner: Arc<dyn LlmProvider>,
    permits: Arc<Semaphore>,
    patience: Duration,
}

impl ConcurrencyLimited {
    pub fn wrap(inner: Arc<dyn LlmProvider>, max_concurrency: usize) -> Arc<dyn LlmProvider> {
        if max_concurrency == 0 {
            return inner;
        }
        Arc::new(Self {
            inner,
            permits: Arc::new(Semaphore::new(max_concurrency)),
            patience: QUEUE_PATIENCE,
        })
    }

    async fn permit(&self) -> Result<OwnedSemaphorePermit> {
        match tokio::time::timeout(self.patience, Arc::clone(&self.permits).acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(anyhow!(
                "provider {} is unavailable: every concurrent request slot is busy",
                self.inner.id()
            )),
        }
    }
}

#[async_trait]
impl LlmProvider for ConcurrencyLimited {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn kind(&self) -> &str {
        self.inner.kind()
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    fn expose_to_ui(&self) -> bool {
        self.inner.expose_to_ui()
    }

    fn user_provided_key(&self) -> bool {
        self.inner.user_provided_key()
    }

    fn name(&self) -> Option<&str> {
        self.inner.name()
    }

    fn configured_models(&self) -> Option<&[String]> {
        self.inner.configured_models()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.inner.list_models().await
    }

    async fn chat_with(
        &self,
        model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let _permit = self.permit().await?;
        self.inner.chat_with(model, messages, tools).await
    }

    async fn chat_stream_with(
        &self,
        model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let _permit = self.permit().await?;
        self.inner
            .chat_stream_with(model, messages, tools, tx)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MockBehaviour;
    use crate::llm::breaker::Failure;
    use crate::llm::mock::MockProvider;
    use crate::llm::types::ProviderMeta;

    fn slow_mock() -> Arc<dyn LlmProvider> {
        Arc::new(MockProvider::new(
            "mock".into(),
            MockBehaviour {
                chunks: 1,
                chunk_delay_ms: 0,
                first_token_ms: 200,
                reply: Some("ok".into()),
            },
            ProviderMeta {
                id: "slow".into(),
                expose_to_ui: true,
                name: None,
                models: None,
                user_provided_key: false,
            },
        ))
    }

    #[tokio::test]
    async fn zero_means_no_limit() {
        let inner = slow_mock();
        let wrapped = ConcurrencyLimited::wrap(Arc::clone(&inner), 0);
        assert!(Arc::ptr_eq(&inner, &wrapped));
    }

    #[tokio::test]
    async fn a_saturated_provider_fails_fast_with_a_transient_error() {
        let limited = Arc::new(ConcurrencyLimited {
            inner: slow_mock(),
            permits: Arc::new(Semaphore::new(1)),
            patience: Duration::from_millis(20),
        });
        let busy = {
            let limited = Arc::clone(&limited);
            tokio::spawn(async move { limited.chat_with(None, &[], &[]).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let rejected = limited.chat_with(None, &[], &[]).await.unwrap_err();
        assert_eq!(Failure::of(&rejected), Failure::Transient);
        assert!(busy.await.unwrap().is_ok());
        assert!(limited.chat_with(None, &[], &[]).await.is_ok());
    }
}
