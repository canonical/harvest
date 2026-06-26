pub mod anthropic;
pub mod gemini;
pub mod openai_compat;
pub mod types;
mod retry;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::LlmProviderConfig;
use types::{LlmResponse, Message, StreamEvent, ToolDefinition};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse>;

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let response = self.chat(messages, tools).await?;
        match response {
            LlmResponse::Message { text } => {
                let _ = tx.send(StreamEvent::TextDelta { text }).await;
                let _ = tx.send(StreamEvent::Done { stop_reason: "end_turn".into() }).await;
            }
            LlmResponse::ToolCalls { calls, preamble } => {
                if !preamble.is_empty() {
                    let _ = tx.send(StreamEvent::TextDelta { text: preamble }).await;
                }
                for call in calls {
                    let _ = tx.send(StreamEvent::ToolCallReady(call)).await;
                }
                let _ = tx.send(StreamEvent::Done { stop_reason: "tool_use".into() }).await;
            }
        }
        Ok(())
    }
}

struct FallbackProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
}

impl FallbackProvider {
    fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        assert!(!providers.is_empty(), "FallbackProvider requires at least one provider");
        Self { providers }
    }

    fn is_rate_limited(err: &anyhow::Error) -> bool {
        let msg = err.to_string().to_lowercase();
        msg.contains("429") || msg.contains("resource_exhausted")
    }
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    async fn chat(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        let mut last_err = anyhow::anyhow!("no LLM providers configured");
        for provider in &self.providers {
            match provider.chat(messages, tools).await {
                Ok(response) => return Ok(response),
                Err(e) if Self::is_rate_limited(&e) => {
                    tracing::warn!(error = %e, "LLM provider rate limited — trying next provider");
                    last_err = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let mut last_err = anyhow::anyhow!("no LLM providers configured");
        for provider in &self.providers {
            match provider.chat_stream(messages, tools, tx.clone()).await {
                Ok(()) => return Ok(()),
                Err(e) if Self::is_rate_limited(&e) => {
                    tracing::warn!(error = %e, "LLM provider stream rate limited — trying next provider");
                    last_err = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}

fn build_provider(config: &LlmProviderConfig) -> Arc<dyn LlmProvider> {
    match config {
        LlmProviderConfig::Anthropic { model, api_key, timeout_secs, max_retries, .. } =>
            Arc::new(anthropic::AnthropicProvider::new(
                model.clone(), api_key.clone(), *timeout_secs, *max_retries,
            )),
        LlmProviderConfig::Gemini { model, api_key, timeout_secs, max_retries, .. } =>
            Arc::new(gemini::GeminiProvider::new(
                model.clone(), api_key.clone(), *timeout_secs, *max_retries,
            )),
        LlmProviderConfig::OpenAiCompat { base_url, api_key, model, timeout_secs, max_retries, .. } =>
            Arc::new(openai_compat::OpenAiCompatProvider::new(
                base_url.clone(), api_key.clone(), model.clone(), *timeout_secs, *max_retries,
            )),
    }
}

pub fn from_config(configs: &[LlmProviderConfig]) -> Arc<dyn LlmProvider> {
    let mut ordered: Vec<&LlmProviderConfig> = configs.iter().collect();
    ordered.sort_by_key(|c| c.priority());
    let providers = ordered.iter().map(|c| build_provider(c)).collect();
    Arc::new(FallbackProvider::new(providers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use std::collections::VecDeque;

    struct MockProvider {
        responses: Mutex<VecDeque<Result<LlmResponse>>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Result<LlmResponse>>) -> Arc<Self> {
            Arc::new(Self { responses: Mutex::new(responses.into()) })
        }
        fn ok(text: &str) -> Result<LlmResponse> {
            Ok(LlmResponse::Message { text: text.into() })
        }
        fn rate_limited() -> Result<LlmResponse> {
            Err(anyhow!("provider error 429 Too Many Requests"))
        }
        fn auth_error() -> Result<LlmResponse> {
            Err(anyhow!("provider error 401 Unauthorized"))
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn chat(&self, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
            self.responses.lock().unwrap().pop_front()
                .unwrap_or_else(|| Err(anyhow!("MockProvider: exhausted")))
        }
    }

    fn fallback(providers: Vec<Arc<dyn LlmProvider>>) -> FallbackProvider {
        FallbackProvider::new(providers)
    }

    #[test]
    fn detects_429_in_error_message() {
        assert!(FallbackProvider::is_rate_limited(&anyhow!("error 429 Too Many Requests")));
    }

    #[test]
    fn detects_resource_exhausted_case_insensitive() {
        assert!(FallbackProvider::is_rate_limited(&anyhow!("Gemini RESOURCE_EXHAUSTED quota")));
    }

    #[test]
    fn non_rate_limit_error_not_detected() {
        assert!(!FallbackProvider::is_rate_limited(&anyhow!("401 Unauthorized")));
        assert!(!FallbackProvider::is_rate_limited(&anyhow!("network timeout")));
        assert!(!FallbackProvider::is_rate_limited(&anyhow!("500 Internal Server Error")));
    }

    #[tokio::test]
    async fn single_provider_returns_its_response() {
        let p = fallback(vec![MockProvider::new(vec![MockProvider::ok("hello")])]);
        let r = p.chat(&[], &[]).await.unwrap();
        assert!(matches!(r, LlmResponse::Message { text } if text == "hello"));
    }

    #[tokio::test]
    async fn first_rate_limited_falls_back_to_second() {
        let p1 = MockProvider::new(vec![MockProvider::rate_limited()]);
        let p2 = MockProvider::new(vec![MockProvider::ok("from second")]);
        let fb = fallback(vec![p1, p2]);
        let r = fb.chat(&[], &[]).await.unwrap();
        assert!(matches!(r, LlmResponse::Message { text } if text == "from second"));
    }

    #[tokio::test]
    async fn non_rate_limit_error_propagates_without_trying_second() {
        let p1 = MockProvider::new(vec![MockProvider::auth_error()]);
        let p2 = MockProvider::new(vec![MockProvider::ok("should not reach")]);
        let fb = fallback(vec![p1, p2]);
        let err = fb.chat(&[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("401"), "expected 401, got: {err}");
    }

    #[tokio::test]
    async fn all_providers_rate_limited_returns_last_error() {
        let p1 = MockProvider::new(vec![MockProvider::rate_limited()]);
        let p2 = MockProvider::new(vec![MockProvider::rate_limited()]);
        let fb = fallback(vec![p1, p2]);
        let err = fb.chat(&[], &[]).await.unwrap_err();
        assert!(err.to_string().contains("429"), "expected 429, got: {err}");
    }

    #[tokio::test]
    async fn first_succeeds_second_not_called() {
        let p1 = MockProvider::new(vec![MockProvider::ok("first wins")]);
        let p2 = MockProvider::new(vec![]);
        let fb = fallback(vec![p1, p2]);
        let r = fb.chat(&[], &[]).await.unwrap();
        assert!(matches!(r, LlmResponse::Message { text } if text == "first wins"));
    }

    #[tokio::test]
    async fn stream_first_rate_limited_falls_back_to_second() {
        let p1 = MockProvider::new(vec![MockProvider::rate_limited()]);
        let p2 = MockProvider::new(vec![MockProvider::ok("streamed from second")]);
        let fb = fallback(vec![p1, p2]);

        let (tx, mut rx) = mpsc::channel(16);
        fb.chat_stream(&[], &[], tx).await.unwrap();

        let mut texts = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let StreamEvent::TextDelta { text } = ev { texts.push(text); }
        }
        assert_eq!(texts, vec!["streamed from second"]);
    }

    #[tokio::test]
    async fn stream_non_rate_limit_error_propagates() {
        let p1 = MockProvider::new(vec![MockProvider::auth_error()]);
        let p2 = MockProvider::new(vec![MockProvider::ok("should not reach")]);
        let fb = fallback(vec![p1, p2]);

        let (tx, _rx) = mpsc::channel(16);
        let err = fb.chat_stream(&[], &[], tx).await.unwrap_err();
        assert!(err.to_string().contains("401"));
    }

    #[test]
    fn from_config_handles_single_provider() {
        let cfg = vec![crate::config::LlmProviderConfig::Gemini {
            model:        "m".into(),
            api_key:      "k".into(),
            priority:     0,
            timeout_secs: 30,
            max_retries:  0,
        }];
        let _ = from_config(&cfg);
    }
}
