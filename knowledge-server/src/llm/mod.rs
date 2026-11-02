pub mod anthropic;
pub mod breaker;
pub mod gemini;
pub mod limited;
pub mod mock;
pub mod openai_compat;
pub mod pricing;
mod retry;
pub mod system_one;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::config::LlmProviderConfig;
use breaker::{CircuitBreaker, Failure};
use types::{
    LlmResponse, Message, ModelInfo, ProviderMeta, ProviderSelection, StreamEvent, ToolDefinition,
    Usage, UsedProvider,
};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> &str;
    fn default_model(&self) -> &str;

    fn expose_to_ui(&self) -> bool {
        true
    }

    fn user_provided_key(&self) -> bool {
        false
    }

    fn name(&self) -> Option<&str> {
        None
    }

    fn configured_models(&self) -> Option<&[String]> {
        None
    }

    fn context_window(&self) -> Option<usize> {
        None
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>>;

    async fn available_models(&self) -> Result<Vec<ModelInfo>> {
        let discovered = self.list_models().await?;
        match self.configured_models() {
            None => Ok(discovered),
            Some(allowed) => Ok(allowed
                .iter()
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
            LlmResponse::Message { text, usage } => {
                let _ = tx.send(StreamEvent::TextDelta { text }).await;
                let _ = tx
                    .send(StreamEvent::Done {
                        stop_reason: "end_turn".into(),
                        usage,
                    })
                    .await;
            }
            LlmResponse::ToolCalls {
                calls,
                preamble,
                usage,
            } => {
                if !preamble.is_empty() {
                    let _ = tx.send(StreamEvent::TextDelta { text: preamble }).await;
                }
                for call in calls {
                    let _ = tx.send(StreamEvent::ToolCallReady(call)).await;
                }
                let _ = tx
                    .send(StreamEvent::Done {
                        stop_reason: "tool_use".into(),
                        usage,
                    })
                    .await;
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
    ) -> Result<(LlmResponse, UsedProvider, Usage)> {
        let model = selection
            .filter(|s| s.provider_id == self.id())
            .and_then(|s| s.model.as_deref());
        let response = self.chat_with(model, messages, tools).await?;
        let usage = match &response {
            LlmResponse::Message { usage, .. } => usage.clone(),
            LlmResponse::ToolCalls { usage, .. } => usage.clone(),
        };
        Ok((response, self.used(model), usage))
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
    breakers: Vec<CircuitBreaker>,
}

impl FallbackProvider {
    fn new(providers: Vec<Arc<dyn LlmProvider>>) -> Self {
        assert!(
            !providers.is_empty(),
            "FallbackProvider requires at least one provider"
        );
        let breakers = providers
            .iter()
            .map(|_| CircuitBreaker::default())
            .collect();
        Self {
            providers,
            breakers,
        }
    }

    fn ordered_for(&self, selection: Option<&ProviderSelection>) -> Vec<(usize, Option<String>)> {
        let preferred =
            selection.filter(|sel| self.providers.iter().any(|p| p.id() == sel.provider_id));
        let mut order: Vec<(usize, Option<String>)> = match preferred {
            Some(sel) => {
                let (matched, rest): (Vec<usize>, Vec<usize>) = (0..self.providers.len())
                    .partition(|&i| self.providers[i].id() == sel.provider_id);
                matched
                    .into_iter()
                    .map(|i| (i, sel.model.clone()))
                    .chain(rest.into_iter().map(|i| (i, None)))
                    .collect()
            }
            None => (0..self.providers.len()).map(|i| (i, None)).collect(),
        };
        order.sort_by_key(|(i, _)| self.breakers[*i].is_open());
        order
    }

    fn record(&self, index: usize, outcome: &Result<impl Sized>) -> Option<Failure> {
        let provider = self.providers[index].id();
        let breaker = &self.breakers[index];
        let failure = match outcome {
            Ok(_) => {
                breaker.succeeded();
                None
            }
            Err(e) => {
                let failure = Failure::of(e);
                breaker.failed(failure);
                tracing::warn!(error = %e, provider, ?failure, "LLM provider call failed");
                Some(failure)
            }
        };
        let outcome_label = match failure {
            None => "ok",
            Some(Failure::RateLimited) => "rate_limited",
            Some(Failure::Transient) => "transient",
            Some(Failure::Permanent) => "permanent",
        };
        crate::metrics::LLM_CALLS.inc(&[("provider", provider), ("outcome", outcome_label)]);
        crate::metrics::LLM_BREAKER_OPEN
            .set(&[("provider", provider)], u64::from(breaker.is_open()));
        failure
    }

    async fn first_success<T, F, Fut>(
        &self,
        order: Vec<(usize, Option<String>)>,
        mut call: F,
    ) -> Result<T>
    where
        F: FnMut(usize, Option<String>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_err = anyhow::anyhow!("no LLM providers configured");
        for (index, model) in order {
            let outcome = call(index, model).await;
            match self.record(index, &outcome) {
                None => return outcome,
                Some(failure) if failure.moves_on() => {
                    last_err = outcome.err().expect("a failed call")
                }
                Some(_) => return outcome,
            }
        }
        Err(last_err)
    }

    async fn stream_once(
        &self,
        index: usize,
        model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> (Result<()>, bool) {
        let (relay_tx, mut relay_rx) = mpsc::channel::<StreamEvent>(64);
        let relay = tokio::spawn(async move {
            let mut emitted = false;
            while let Some(event) = relay_rx.recv().await {
                emitted = true;
                if tx.send(event).await.is_err() {
                    break;
                }
            }
            emitted
        });
        let provider = &self.providers[index];
        let outcome = match model {
            Some(_) => {
                provider
                    .chat_stream_with(model, messages, tools, relay_tx)
                    .await
            }
            None => provider.chat_stream(messages, tools, relay_tx).await,
        };
        (outcome, relay.await.unwrap_or(true))
    }

    async fn stream_first_success(
        &self,
        order: Vec<(usize, Option<String>)>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<UsedProvider> {
        let mut last_err = anyhow::anyhow!("no LLM providers configured");
        for (index, model) in order {
            let (outcome, emitted) = self
                .stream_once(index, model.as_deref(), messages, tools, tx.clone())
                .await;
            match self.record(index, &outcome) {
                None => return Ok(self.providers[index].used(model.as_deref())),
                Some(failure) if failure.moves_on() && !emitted => {
                    last_err = outcome.err().expect("a failed call")
                }
                Some(_) => return Err(outcome.err().expect("a failed call")),
            }
        }
        Err(last_err)
    }
}

#[async_trait]
impl LlmProvider for FallbackProvider {
    fn id(&self) -> &str {
        "fallback"
    }
    fn kind(&self) -> &str {
        "fallback"
    }
    fn default_model(&self) -> &str {
        self.providers[0].default_model()
    }
    fn children(&self) -> &[Arc<dyn LlmProvider>] {
        &self.providers
    }

    fn context_window(&self) -> Option<usize> {
        self.providers
            .iter()
            .filter_map(|p| p.context_window())
            .min()
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let mut all = Vec::new();
        for provider in &self.providers {
            if let Ok(models) = provider.list_models().await {
                all.extend(models);
            }
        }
        Ok(all)
    }

    async fn chat_with(
        &self,
        _model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        self.first_success(self.ordered_for(None), |index, _| {
            self.providers[index].chat(messages, tools)
        })
        .await
    }

    async fn chat_stream_with(
        &self,
        _model: Option<&str>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        self.stream_first_success(self.ordered_for(None), messages, tools, tx)
            .await
            .map(|_| ())
    }

    async fn chat_routed(
        &self,
        selection: Option<&ProviderSelection>,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<(LlmResponse, UsedProvider, Usage)> {
        self.first_success(self.ordered_for(selection), |index, model| async move {
            let provider = &self.providers[index];
            let response = provider
                .chat_with(model.as_deref(), messages, tools)
                .await?;
            let usage = match &response {
                LlmResponse::Message { usage, .. } => usage.clone(),
                LlmResponse::ToolCalls { usage, .. } => usage.clone(),
            };
            Ok((response, provider.used(model.as_deref()), usage))
        })
        .await
    }

    async fn chat_stream_routed(
        &self,
        selection: Option<&ProviderSelection>,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<UsedProvider> {
        self.stream_first_success(self.ordered_for(selection), messages, tools, tx)
            .await
    }
}

fn build_meta(config: &LlmProviderConfig) -> ProviderMeta {
    ProviderMeta {
        id: config.id().to_string(),
        expose_to_ui: config.expose_to_ui(),
        name: config.name().map(str::to_string),
        models: config.models().map(|m| m.to_vec()),
        user_provided_key: config.user_provided_key(),
    }
}

fn build_provider(config: &LlmProviderConfig) -> Arc<dyn LlmProvider> {
    build_provider_with_key(config, None)
}

fn build_provider_with_key(
    config: &LlmProviderConfig,
    api_key_override: Option<&str>,
) -> Arc<dyn LlmProvider> {
    limited::ProviderLimits::wrap(
        build_bare_provider(config, api_key_override),
        config.max_concurrency(),
        config.context_window(),
    )
}

fn build_bare_provider(
    config: &LlmProviderConfig,
    api_key_override: Option<&str>,
) -> Arc<dyn LlmProvider> {
    let meta = build_meta(config);
    let api_key = api_key_override.unwrap_or_else(|| config.api_key());
    match config {
        LlmProviderConfig::Anthropic {
            model,
            timeout_secs,
            max_retries,
            ..
        } => Arc::new(anthropic::AnthropicProvider::new(
            model.clone(),
            api_key.to_string(),
            *timeout_secs,
            *max_retries,
            meta,
        )),
        LlmProviderConfig::Gemini {
            model,
            timeout_secs,
            max_retries,
            ..
        } => Arc::new(gemini::GeminiProvider::new(
            model.clone(),
            api_key.to_string(),
            *timeout_secs,
            *max_retries,
            meta,
        )),
        LlmProviderConfig::OpenAiCompat {
            base_url,
            model,
            timeout_secs,
            max_retries,
            ..
        } => Arc::new(
            openai_compat::OpenAiCompatProvider::new(
                base_url.clone(),
                api_key.to_string(),
                model.clone(),
                *timeout_secs,
                *max_retries,
                meta,
            )
            .with_prompt_caching(config.enable_prompt_caching()),
        ),
        LlmProviderConfig::Mock {
            model, behaviour, ..
        } => Arc::new(mock::MockProvider::new(
            model.clone(),
            behaviour.clone(),
            meta,
        )),
    }
}

pub fn from_config(configs: &[LlmProviderConfig]) -> Arc<dyn LlmProvider> {
    let mut ordered: Vec<&LlmProviderConfig> = configs.iter().collect();
    ordered.sort_by_key(|c| c.priority());
    let providers = ordered.iter().map(|c| build_provider(c)).collect();
    Arc::new(FallbackProvider::new(providers))
}

pub fn with_user_keys(
    llm: &Arc<dyn LlmProvider>,
    configs: &[LlmProviderConfig],
    keys: &std::collections::HashMap<String, String>,
) -> Arc<dyn LlmProvider> {
    let children = llm.children();
    if children.is_empty() || keys.is_empty() {
        return Arc::clone(llm);
    }
    let config_map: std::collections::HashMap<&str, &LlmProviderConfig> =
        configs.iter().map(|c| (c.id(), c)).collect();
    let new_children: Vec<Arc<dyn LlmProvider>> = children
        .iter()
        .map(|child| {
            if let Some(user_key) = keys.get(child.id()) {
                if let Some(config) = config_map.get(child.id()) {
                    return build_provider_with_key(config, Some(user_key));
                }
            }
            Arc::clone(child)
        })
        .collect();
    Arc::new(FallbackProvider::new(new_children))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;

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
                models: vec![ModelInfo {
                    id: "mock-model".into(),
                    display_name: None,
                }],
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
        fn with_discovered_and_allowlist(
            discovered: Vec<ModelInfo>,
            allowed: Vec<String>,
        ) -> Arc<Self> {
            Arc::new(Self {
                id: "mock".into(),
                responses: Mutex::new(VecDeque::new()),
                last_model: Mutex::new(None),
                models: discovered,
                configured_models: Some(allowed),
            })
        }
        fn ok(text: &str) -> Result<LlmResponse> {
            Ok(LlmResponse::Message {
                text: text.into(),
                usage: Usage::default(),
            })
        }
        fn rate_limited() -> Result<LlmResponse> {
            Err(anyhow!("provider error 429 Too Many Requests"))
        }
        fn auth_error() -> Result<LlmResponse> {
            Err(anyhow!("provider error 401 Unauthorized"))
        }
        fn overloaded() -> Result<LlmResponse> {
            Err(anyhow!("provider error 503 overloaded"))
        }
        fn last_model(&self) -> Option<String> {
            self.last_model.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn id(&self) -> &str {
            &self.id
        }
        fn kind(&self) -> &str {
            "mock"
        }
        fn default_model(&self) -> &str {
            "mock-model"
        }
        fn configured_models(&self) -> Option<&[String]> {
            self.configured_models.as_deref()
        }

        async fn list_models(&self) -> Result<Vec<ModelInfo>> {
            if self.models.is_empty() {
                anyhow::bail!("discovery failed");
            }
            Ok(self.models.clone())
        }

        async fn chat_with(
            &self,
            model: Option<&str>,
            _: &[Message],
            _: &[ToolDefinition],
        ) -> Result<LlmResponse> {
            *self.last_model.lock().unwrap() = model.map(String::from);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(anyhow!("MockProvider: exhausted")))
        }
    }

    fn fallback(providers: Vec<Arc<dyn LlmProvider>>) -> FallbackProvider {
        FallbackProvider::new(providers)
    }

    #[tokio::test]
    async fn a_transient_error_falls_back_to_the_next_provider() {
        let p1 = MockProvider::new(vec![MockProvider::overloaded()]);
        let p2 = MockProvider::new(vec![MockProvider::ok("from second")]);
        let fb = fallback(vec![p1, p2]);
        let r = fb.chat(&[], &[]).await.unwrap();
        assert!(matches!(r, LlmResponse::Message { text, .. } if text == "from second"));
    }

    #[tokio::test]
    async fn a_provider_whose_breaker_is_open_is_tried_last() {
        let p1 = MockProvider::with_id(
            "flaky",
            vec![
                MockProvider::overloaded(),
                MockProvider::overloaded(),
                MockProvider::overloaded(),
                MockProvider::ok("flaky recovered"),
            ],
        );
        let p2 = MockProvider::with_id(
            "steady",
            vec![
                MockProvider::ok("steady 1"),
                MockProvider::ok("steady 2"),
                MockProvider::ok("steady 3"),
                MockProvider::ok("steady 4"),
            ],
        );
        let fb = fallback(vec![p1, p2]);
        for _ in 0..3 {
            fb.chat(&[], &[]).await.unwrap();
        }
        let r = fb.chat(&[], &[]).await.unwrap();
        assert!(
            matches!(r, LlmResponse::Message { text, .. } if text == "steady 4"),
            "an open breaker sends traffic to the healthy provider first"
        );
    }

    #[tokio::test]
    async fn single_provider_returns_its_response() {
        let p = fallback(vec![MockProvider::new(vec![MockProvider::ok("hello")])]);
        let r = p.chat(&[], &[]).await.unwrap();
        assert!(matches!(r, LlmResponse::Message { text, .. } if text == "hello"));
    }

    #[tokio::test]
    async fn first_rate_limited_falls_back_to_second() {
        let p1 = MockProvider::new(vec![MockProvider::rate_limited()]);
        let p2 = MockProvider::new(vec![MockProvider::ok("from second")]);
        let fb = fallback(vec![p1, p2]);
        let r = fb.chat(&[], &[]).await.unwrap();
        assert!(matches!(r, LlmResponse::Message { text, .. } if text == "from second"));
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
        assert!(matches!(r, LlmResponse::Message { text, .. } if text == "first wins"));
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
            if let StreamEvent::TextDelta { text } = ev {
                texts.push(text);
            }
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
            model: "m".into(),
            api_key: "k".into(),
            id: "gemini-0".into(),
            priority: 0,
            timeout_secs: 30,
            max_retries: 0,
            expose_to_ui: true,
            name: None,
            models: None,
            user_provided_key: false,
            pricing: None,
            max_concurrency: 0,
            context_window: None,
        }];
        let _ = from_config(&cfg);
    }

    #[tokio::test]
    async fn routed_with_no_selection_matches_default_order() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![]);
        let fb = fallback(vec![p1, p2]);

        let (response, used, _) = fb.chat_routed(None, &[], &[]).await.unwrap();
        assert!(matches!(response, LlmResponse::Message { text, .. } if text == "from p1"));
        assert_eq!(used.provider_id, "p1");
        assert_eq!(used.model, "mock-model");
    }

    #[tokio::test]
    async fn routed_selection_tries_matching_provider_first_with_model_override() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![MockProvider::ok("from p2")]);
        let fb = fallback(vec![p1.clone(), p2]);

        let selection = ProviderSelection {
            provider_id: "p2".into(),
            model: Some("custom-model".into()),
            cache_breakpoint_index: None,
        };
        let (response, used, _) = fb.chat_routed(Some(&selection), &[], &[]).await.unwrap();

        assert!(matches!(response, LlmResponse::Message { text, .. } if text == "from p2"));
        assert_eq!(used.provider_id, "p2");
        assert_eq!(used.model, "custom-model");
        assert_eq!(p1.last_model(), None, "p1 should never have been called");
    }

    #[tokio::test]
    async fn routed_selection_falls_back_when_selected_provider_rate_limited() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![MockProvider::rate_limited()]);
        let fb = fallback(vec![p1, p2]);

        let selection = ProviderSelection {
            provider_id: "p2".into(),
            model: None,
            cache_breakpoint_index: None,
        };
        let (response, used, _) = fb.chat_routed(Some(&selection), &[], &[]).await.unwrap();

        assert!(matches!(response, LlmResponse::Message { text, .. } if text == "from p1"));
        assert_eq!(
            used.provider_id, "p1",
            "should report the provider that actually answered"
        );
    }

    #[tokio::test]
    async fn routed_selection_with_unknown_provider_id_uses_default_order() {
        let p1 = MockProvider::with_id("p1", vec![MockProvider::ok("from p1")]);
        let p2 = MockProvider::with_id("p2", vec![]);
        let fb = fallback(vec![p1, p2]);

        let selection = ProviderSelection {
            provider_id: "does-not-exist".into(),
            model: None,
            cache_breakpoint_index: None,
        };
        let (response, used, _) = fb.chat_routed(Some(&selection), &[], &[]).await.unwrap();

        assert!(matches!(response, LlmResponse::Message { text, .. } if text == "from p1"));
        assert_eq!(used.provider_id, "p1");
    }

    #[tokio::test]
    async fn stream_routed_selection_reports_used_provider_and_model() {
        let p1 = MockProvider::with_id("p1", vec![]);
        let p2 = MockProvider::with_id("p2", vec![MockProvider::ok("streamed")]);
        let fb = fallback(vec![p1, p2]);

        let selection = ProviderSelection {
            provider_id: "p2".into(),
            model: Some("m2".into()),
            cache_breakpoint_index: None,
        };
        let (tx, mut rx) = mpsc::channel(16);
        let used = fb
            .chat_stream_routed(Some(&selection), &[], &[], tx)
            .await
            .unwrap();

        assert_eq!(used.provider_id, "p2");
        assert_eq!(used.model, "m2");
        let mut texts = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let StreamEvent::TextDelta { text } = ev {
                texts.push(text);
            }
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
        assert_eq!(
            models.len(),
            2,
            "p2's failed discovery should not break the aggregate"
        );
    }

    #[tokio::test]
    async fn available_models_returns_all_discovered_when_no_allowlist_configured() {
        let provider = MockProvider::with_id("p", vec![]);
        let available = provider.available_models().await.unwrap();
        assert_eq!(
            available,
            vec![ModelInfo {
                id: "mock-model".into(),
                display_name: None
            }]
        );
    }

    #[tokio::test]
    async fn available_models_filters_to_allowlist_and_preserves_its_order() {
        let discovered = vec![
            ModelInfo {
                id: "a".into(),
                display_name: None,
            },
            ModelInfo {
                id: "b".into(),
                display_name: None,
            },
            ModelInfo {
                id: "c".into(),
                display_name: None,
            },
        ];
        let provider =
            MockProvider::with_discovered_and_allowlist(discovered, vec!["c".into(), "a".into()]);
        let available = provider.available_models().await.unwrap();
        let ids: Vec<&str> = available.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a"]);
    }

    #[tokio::test]
    async fn available_models_drops_allowlist_entries_that_do_not_exist_on_the_provider() {
        let discovered = vec![ModelInfo {
            id: "a".into(),
            display_name: None,
        }];
        let provider = MockProvider::with_discovered_and_allowlist(
            discovered,
            vec!["a".into(), "does-not-exist".into()],
        );
        let available = provider.available_models().await.unwrap();
        let ids: Vec<&str> = available.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["a"]);
    }

    #[tokio::test]
    async fn available_models_is_empty_when_no_allowlist_entries_validate() {
        let discovered = vec![ModelInfo {
            id: "a".into(),
            display_name: None,
        }];
        let provider =
            MockProvider::with_discovered_and_allowlist(discovered, vec!["nonexistent".into()]);
        let available = provider.available_models().await.unwrap();
        assert!(available.is_empty());
    }

    #[test]
    fn from_config_openai_compat_without_caching() {
        let cfg = vec![crate::config::LlmProviderConfig::OpenAiCompat {
            base_url: "https://x".into(),
            api_key: "k".into(),
            model: "m".into(),
            id: "oai-0".into(),
            priority: 0,
            timeout_secs: 30,
            max_retries: 0,
            expose_to_ui: true,
            name: None,
            models: None,
            user_provided_key: false,
            pricing: None,
            enable_prompt_caching: false,
            max_concurrency: 0,
            context_window: None,
        }];
        let _ = from_config(&cfg);
    }

    #[test]
    fn from_config_openai_compat_with_caching() {
        let cfg = vec![crate::config::LlmProviderConfig::OpenAiCompat {
            base_url: "https://x".into(),
            api_key: "k".into(),
            model: "m".into(),
            id: "oai-0".into(),
            priority: 0,
            timeout_secs: 30,
            max_retries: 0,
            expose_to_ui: true,
            name: None,
            models: None,
            user_provided_key: false,
            pricing: None,
            enable_prompt_caching: true,
            max_concurrency: 0,
            context_window: None,
        }];
        let _ = from_config(&cfg);
    }
}
