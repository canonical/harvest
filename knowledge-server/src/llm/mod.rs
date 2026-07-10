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
use types::{LlmResponse, Message, ModelInfo, ProviderMeta, ProviderSelection, StreamEvent, ToolDefinition, UsedProvider};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> &str;
    fn default_model(&self) -> &str;

    fn expose_to_ui(&self) -> bool {
        true
    }

    fn name(&self) -> Option<&str> {
        None
    }

    fn configured_models(&self) -> Option<&[String]> {
        None
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    async fn available_models(&self) -> Result<Vec<ModelInfo>> {
        let discovered = self.list_models().await?;
        match self.configured_models() {
            None => Ok(discovered),
            Some(allowed) => Ok(allowed.iter()
                .filter_map(|wanted| discovered.iter().find(|m| &m.id == wanted).cloned())
                .collect()),
        }
    }

    async fn chat_with(
        &self,
        model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse>;

    async fn chat(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        self.chat_with(None, messages, tools).await
    }

    async fn chat_stream_with(
        &self,
        model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let response = self.chat_with(model, messages, tools).await?;
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

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        self.chat_stream_with(None, messages, tools, tx).await
    }

    fn used(&self, model: Option<&str>) -> UsedProvider {
        UsedProvider {
            provider_id: self.id().to_string(),
            kind: self.kind().to_string(),
            model: model.unwrap_or_else(|| self.default_model()).to_string(),
        }
    }

    async fn chat_routed(
        &self,
        selection: Option<&ProviderSelection>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<(LlmResponse, UsedProvider)> {
        let model = selection
            .filter(|s| s.provider_id == self.id())
            .and_then(|s| s.model.as_deref());
        let response = self.chat_with(model, messages, tools).await?;
        Ok((response, self.used(model)))
    }

    async fn chat_stream_routed(
        &self,
        selection: Option<&ProviderSelection>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<UsedProvider> {
        let model = selection
            .filter(|s| s.provider_id == self.id())
            .and_then(|s| s.model.as_deref());
        self.chat_stream_with(model, messages, tools, tx).await?;
        Ok(self.used(model))
    }

    fn children(&self) -> &[Arc<dyn LlmProvider>] {
        &[]
    }
}

pub struct FallbackProvider {
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

    fn ordered_for(
        &self,
        selection: Option<&ProviderSelection>,
    ) -> Vec<(&Arc<dyn LlmProvider>, Option<String>)> {
        match selection {
            Some(sel) if self.providers.iter().any(|p| p.id() == sel.provider_id) => {
                let (matched, rest): (Vec<_>, Vec<_>) =
                    self.providers.iter().partition(|p| p.id() == sel.provider_id);
                matched.into_iter().map(|p| (p, sel.model.clone()))
                    .chain(rest.into_iter().map(|p| (p, None)))
                    .collect()
            }
            _ => self.providers.iter().map(|p| (p, None)).collect(),
        }
    }
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    fn id(&self) -> &str { "fallback" }
    fn kind(&self) -> &str { "fallback" }
    fn default_model(&self) -> &str { self.providers[0].default_model() }
    fn children(&self) -> &[Arc<dyn LlmProvider>] { &self.providers }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let mut all = Vec::new();
        for provider in &self.providers {
            if let Ok(models) = provider.list_models().await {
                all.extend(models);
            }
        }
        Ok(all)
    }

    async fn chat_with(&self, _model: Option<&str>, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
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

    async fn chat_stream_with(
        &self,
        _model: Option<&str>,
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

    async fn chat_routed(
        &self,
        selection: Option<&ProviderSelection>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<(LlmResponse, UsedProvider)> {
        let order = self.ordered_for(selection);
        let mut last_err = anyhow::anyhow!("no LLM providers configured");
        for (provider, model_override) in order {
            match provider.chat_with(model_override.as_deref(), messages, tools).await {
                Ok(response) => return Ok((response, provider.used(model_override.as_deref()))),
                Err(e) if Self::is_rate_limited(&e) => {
                    tracing::warn!(error = %e, provider = provider.id(), "LLM provider rate limited — trying next provider");
                    last_err = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }

    async fn chat_stream_routed(
        &self,
        selection: Option<&ProviderSelection>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<UsedProvider> {
        let order = self.ordered_for(selection);
        let mut last_err = anyhow::anyhow!("no LLM providers configured");
        for (provider, model_override) in order {
            match provider.chat_stream_with(model_override.as_deref(), messages, tools, tx.clone()).await {
                Ok(()) => return Ok(provider.used(model_override.as_deref())),
                Err(e) if Self::is_rate_limited(&e) => {
                    tracing::warn!(error = %e, provider = provider.id(), "LLM provider stream rate limited — trying next provider");
                    last_err = e;
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err)
    }
}

fn build_meta(config: &LlmProviderConfig) -> ProviderMeta {
    ProviderMeta {
        id: config.id().to_string(),
        expose_to_ui: config.expose_to_ui(),
        name: config.name().map(str::to_string),
        models: config.models().map(|m| m.to_vec()),
    }
}

fn build_provider(config: &LlmProviderConfig) -> Arc<dyn LlmProvider> {
    let meta = build_meta(config);
    match config {
        LlmProviderConfig::Anthropic { model, api_key, timeout_secs, max_retries, .. } =>
            Arc::new(anthropic::AnthropicProvider::new(
                model.clone(), api_key.clone(), *timeout_secs, *max_retries, meta,
            )),
        LlmProviderConfig::Gemini { model, api_key, timeout_secs, max_retries, .. } =>
            Arc::new(gemini::GeminiProvider::new(
                model.clone(), api_key.clone(), *timeout_secs, *max_retries, meta,
            )),
        LlmProviderConfig::OpenAiCompat { base_url, api_key, model, timeout_secs, max_retries, .. } =>
            Arc::new(openai_compat::OpenAiCompatProvider::new(
                base_url.clone(), api_key.clone(), model.clone(), *timeout_secs, *max_retries, meta,
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
        id: String,
        responses: Mutex<VecDeque<Result<LlmResponse>>>,
        last_model: Mutex<Option<String>>,
        models: Vec<ModelInfo>,
        configured_models: Option<Vec<String>>,
    }

    impl MockProvider {
        fn new(responses: Vec<Result<LlmResponse>>) -> Arc<Self> {
            Self::with_id("mock", responses)
        }
        fn with_id(id: &str, responses: Vec<Result<LlmResponse>>) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                responses: Mutex::new(responses.into()),
                last_model: Mutex::new(None),
                models: vec![ModelInfo { id: "mock-model".into(), display_name: None }],
                configured_models: None,
            })
        }
        fn failing_discovery(id: &str, responses: Vec<Result<LlmResponse>>) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                responses: Mutex::new(responses.into()),
                last_model: Mutex::new(None),
                models: vec![],
                configured_models: None,
            })
        }
        fn with_discovered_and_allowlist(discovered: Vec<ModelInfo>, allowed: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                id: "mock".into(),
                responses: Mutex::new(VecDeque::new()),
                last_model: Mutex::new(None),
                models: discovered,
                configured_models: Some(allowed),
            })
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
        fn last_model(&self) -> Option<String> {
            self.last_model.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn id(&self) -> &str { &self.id }
        fn kind(&self) -> &str { "mock" }
        fn default_model(&self) -> &str { "mock-model" }
        fn configured_models(&self) -> Option<&[String]> { self.configured_models.as_deref() }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            if self.models.is_empty() {
                anyhow::bail!("discovery failed");
            }
            Ok(self.models.clone())
        }

        async fn chat_with(&self, model: Option<&str>, _: &[Message], _: &[ToolDefinition]) -> Result<LlmResponse> {
            *self.last_model.lock().unwrap() = model.map(String::from);
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
            id:           "gemini-0".into(),
            priority:     0,
            timeout_secs: 30,
            max_retries:  0,
            expose_to_ui: true,
            name:         None,
            models:       None,
        }];
        let _ = from_config(&cfg);
    }

    #[tokio::test]
    async fn routed_with_no_selection_matches_default_order() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![]);
        let fb = fallback(vec![p1, p2]);

        let (response, used) = fb.chat_routed(None, &[], &[]).await.unwrap();
        assert!(matches!(response, LlmResponse::Message { text } if text == "from p1"));
        assert_eq!(used.provider_id, "p1");
        assert_eq!(used.model, "mock-model");
    }

    #[tokio::test]
    async fn routed_selection_tries_matching_provider_first_with_model_override() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![MockProvider::ok("from p2")]);
        let fb = fallback(vec![p1.clone(), p2]);

        let selection = ProviderSelection { provider_id: "p2".into(), model: Some("custom-model".into()) };
        let (response, used) = fb.chat_routed(Some(&selection), &[], &[]).await.unwrap();

        assert!(matches!(response, LlmResponse::Message { text } if text == "from p2"));
        assert_eq!(used.provider_id, "p2");
        assert_eq!(used.model, "custom-model");
        assert_eq!(p1.last_model(), None, "p1 should never have been called");
    }

    #[tokio::test]
    async fn routed_selection_falls_back_when_selected_provider_rate_limited() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![MockProvider::rate_limited()]);
        let fb = fallback(vec![p1, p2]);

        let selection = ProviderSelection { provider_id: "p2".into(), model: None };
        let (response, used) = fb.chat_routed(Some(&selection), &[], &[]).await.unwrap();

        assert!(matches!(response, LlmResponse::Message { text } if text == "from p1"));
        assert_eq!(used.provider_id, "p1", "should report the provider that actually answered");
    }

    #[tokio::test]
    async fn routed_selection_with_unknown_provider_id_uses_default_order() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![]);
        let fb = fallback(vec![p1, p2]);

        let selection = ProviderSelection { provider_id: "does-not-exist".into(), model: None };
        let (response, used) = fb.chat_routed(Some(&selection), &[], &[]).await.unwrap();

        assert!(matches!(response, LlmResponse::Message { text } if text == "from p1"));
        assert_eq!(used.provider_id, "p1");
    }

    #[tokio::test]
    async fn stream_routed_selection_reports_used_provider_and_model() {
        let p1 = MockProvider::with_id("p1", vec![]);
        let p2 = MockProvider::with_id("p2", vec![MockProvider::ok("streamed")]);
        let fb = fallback(vec![p1, p2]);

        let selection = ProviderSelection { provider_id: "p2".into(), model: Some("m2".into()) };
        let (tx, mut rx) = mpsc::channel(16);
        let used = fb.chat_stream_routed(Some(&selection), &[], &[], tx).await.unwrap();

        assert_eq!(used.provider_id, "p2");
        assert_eq!(used.model, "m2");
        let mut texts = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let StreamEvent::TextDelta { text } = ev { texts.push(text); }
        }
        assert_eq!(texts, vec!["streamed"]);
    }

    #[tokio::test]
    async fn list_models_aggregates_across_providers_and_skips_failing_ones() {
        let p1 = MockProvider::with_id("p1", vec![]);
        let p2 = MockProvider::failing_discovery("p2", vec![]);
        let p3 = MockProvider::with_id("p3", vec![]);
        let fb = fallback(vec![p1, p2, p3]);

        let models = fb.list_models().await.unwrap();
        assert_eq!(models.len(), 2, "p2's failed discovery should not break the aggregate");
    }

    #[tokio::test]
    async fn available_models_returns_all_discovered_when_no_allowlist_configured() {
        let provider = MockProvider::with_id("p", vec![]);
        let available = provider.available_models().await.unwrap();
        assert_eq!(available, vec![ModelInfo { id: "mock-model".into(), display_name: None }]);
    }

    #[tokio::test]
    async fn available_models_filters_to_allowlist_and_preserves_its_order() {
        let discovered = vec![
            ModelInfo { id: "a".into(), display_name: None },
            ModelInfo { id: "b".into(), display_name: None },
            ModelInfo { id: "c".into(), display_name: None },
        ];
        let provider = MockProvider::with_discovered_and_allowlist(discovered, vec!["c".into(), "a".into()]);
        let available = provider.available_models().await.unwrap();
        let ids: Vec<&str> = available.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a"]);
    }

    #[tokio::test]
    async fn available_models_drops_allowlist_entries_that_do_not_exist_on_the_provider() {
        let discovered = vec![ModelInfo { id: "a".into(), display_name: None }];
        let provider = MockProvider::with_discovered_and_allowlist(discovered, vec!["a".into(), "does-not-exist".into()]);
        let available = provider.available_models().await.unwrap();
        let ids: Vec<&str> = available.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[tokio::test]
    async fn available_models_is_empty_when_no_allowlist_entries_validate() {
        let discovered = vec![ModelInfo { id: "a".into(), display_name: None }];
        let provider = MockProvider::with_discovered_and_allowlist(discovered, vec!["nonexistent".into()]);
        let available = provider.available_models().await.unwrap();
        assert!(available.is_empty());
    }
}
