use axum::{extract::{Query, State}, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::llm::LlmProvider;

const CACHE_TTL: Duration = Duration::from_secs(300);

pub struct LlmState {
    pub llm: Arc<dyn LlmProvider>,
    cache: RwLock<Option<(Instant, Value)>>,
}

impl LlmState {
    pub fn new(llm: Arc<dyn LlmProvider>) -> Self {
        Self { llm, cache: RwLock::new(None) }
    }
}

fn effective_providers(llm: &Arc<dyn LlmProvider>) -> Vec<Arc<dyn LlmProvider>> {
    let children = llm.children();
    let candidates: Vec<Arc<dyn LlmProvider>> = if children.is_empty() {
        vec![Arc::clone(llm)]
    } else {
        children.to_vec()
    };
    candidates.into_iter().filter(|p| p.expose_to_ui()).collect()
}

async fn build_providers_response(providers: &[Arc<dyn LlmProvider>]) -> Value {
    let mut list = Vec::with_capacity(providers.len());
    for provider in providers {
        let models = provider.available_models().await.unwrap_or_default();
        list.push(json!({
            "id": provider.id(),
            "kind": provider.kind(),
            "name": provider.name().unwrap_or_else(|| provider.kind()),
            "default_model": provider.default_model(),
            "models": models,
        }));
    }
    json!({ "providers": list })
}

async fn cached_or_fresh(state: &LlmState, force_refresh: bool) -> Value {
    if !force_refresh {
        if let Some((fetched_at, value)) = state.cache.read().await.as_ref() {
            if fetched_at.elapsed() < CACHE_TTL {
                return value.clone();
            }
        }
    }
    let providers = effective_providers(&state.llm);
    let response = build_providers_response(&providers).await;
    *state.cache.write().await = Some((Instant::now(), response.clone()));
    response
}

#[derive(Deserialize)]
pub struct ListProvidersQuery {
    #[serde(default)]
    refresh: bool,
}

pub async fn list_providers(
    State(state): State<Arc<LlmState>>,
    Query(params): Query<ListProvidersQuery>,
) -> Json<Value> {
    Json(cached_or_fresh(&state, params.refresh).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::types::ModelInfo;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestProvider {
        id: String,
        kind: String,
        default_model: String,
        models: Vec<ModelInfo>,
        children: Vec<Arc<dyn LlmProvider>>,
        list_models_calls: AtomicUsize,
        expose_to_ui: bool,
        name: Option<String>,
        configured_models: Option<Vec<String>>,
    }

    impl TestProvider {
        fn leaf(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                kind: "mock".into(),
                default_model: "mock-model".into(),
                models: vec![ModelInfo { id: "mock-model".into(), display_name: None }],
                children: vec![],
                list_models_calls: AtomicUsize::new(0),
                expose_to_ui: true,
                name: None,
                configured_models: None,
            })
        }

        fn named(id: &str, name: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                kind: "mock".into(),
                default_model: "mock-model".into(),
                models: vec![ModelInfo { id: "mock-model".into(), display_name: None }],
                children: vec![],
                list_models_calls: AtomicUsize::new(0),
                expose_to_ui: true,
                name: Some(name.into()),
                configured_models: None,
            })
        }

        fn hidden(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                kind: "mock".into(),
                default_model: "mock-model".into(),
                models: vec![ModelInfo { id: "mock-model".into(), display_name: None }],
                children: vec![],
                list_models_calls: AtomicUsize::new(0),
                expose_to_ui: false,
                name: None,
                configured_models: None,
            })
        }

        fn with_allowlist(id: &str, discovered: Vec<ModelInfo>, allowed: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                kind: "mock".into(),
                default_model: "mock-model".into(),
                models: discovered,
                children: vec![],
                list_models_calls: AtomicUsize::new(0),
                expose_to_ui: true,
                name: None,
                configured_models: Some(allowed),
            })
        }

        fn root(children: Vec<Arc<dyn LlmProvider>>) -> Arc<Self> {
            Arc::new(Self {
                id: "fallback".into(),
                kind: "fallback".into(),
                default_model: "mock-model".into(),
                models: vec![],
                children,
                list_models_calls: AtomicUsize::new(0),
                expose_to_ui: true,
                name: None,
                configured_models: None,
            })
        }
    }

    #[async_trait]
    impl LlmProvider for TestProvider {
        fn id(&self) -> &str { &self.id }
        fn kind(&self) -> &str { &self.kind }
        fn default_model(&self) -> &str { &self.default_model }
        fn configured_models(&self) -> Option<&[String]> { self.configured_models.as_deref() }
        fn name(&self) -> Option<&str> { self.name.as_deref() }
        fn children(&self) -> &[Arc<dyn LlmProvider>] { &self.children }
        fn expose_to_ui(&self) -> bool { self.expose_to_ui }

        async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
            self.list_models_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.models.clone())
        }

        async fn chat_with(
            &self,
            _model: Option<&str>,
            _messages: &[crate::llm::types::Message],
            _tools: &[crate::llm::types::ToolDefinition],
        ) -> anyhow::Result<crate::llm::types::LlmResponse> {
            unimplemented!("not exercised by these tests")
        }
    }

    #[test]
    fn effective_providers_returns_self_when_no_children() {
        let leaf = TestProvider::leaf("solo");
        let providers = effective_providers(&(leaf as Arc<dyn LlmProvider>));
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].id(), "solo");
    }

    #[test]
    fn effective_providers_returns_children_when_present() {
        let a = TestProvider::leaf("a");
        let b = TestProvider::leaf("b");
        let root = TestProvider::root(vec![a, b]);
        let providers = effective_providers(&(root as Arc<dyn LlmProvider>));
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0].id(), "a");
        assert_eq!(providers[1].id(), "b");
    }

    #[test]
    fn effective_providers_excludes_children_with_expose_to_ui_false() {
        let a = TestProvider::leaf("a");
        let hidden = TestProvider::hidden("hidden-failover-only");
        let b = TestProvider::leaf("b");
        let root = TestProvider::root(vec![a, hidden, b]);
        let providers = effective_providers(&(root as Arc<dyn LlmProvider>));
        let ids: Vec<&str> = providers.iter().map(|p| p.id()).collect();
        assert_eq!(ids, vec!["a", "b"]);
    }

    #[test]
    fn effective_providers_returns_empty_when_single_provider_is_hidden() {
        let hidden = TestProvider::hidden("solo-hidden");
        let providers = effective_providers(&(hidden as Arc<dyn LlmProvider>));
        assert!(providers.is_empty());
    }

    #[tokio::test]
    async fn build_providers_response_includes_models_per_provider() {
        let a = TestProvider::leaf("a") as Arc<dyn LlmProvider>;
        let b = TestProvider::leaf("b") as Arc<dyn LlmProvider>;
        let response = build_providers_response(&[a, b]).await;
        let providers = response["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0]["id"], "a");
        assert_eq!(providers[0]["default_model"], "mock-model");
        assert_eq!(providers[0]["models"][0]["id"], "mock-model");
    }

    #[tokio::test]
    async fn build_providers_response_uses_configured_name_when_set() {
        let a = TestProvider::named("a", "Lemonade (local)") as Arc<dyn LlmProvider>;
        let response = build_providers_response(&[a]).await;
        assert_eq!(response["providers"][0]["name"], "Lemonade (local)");
    }

    #[tokio::test]
    async fn build_providers_response_falls_back_to_kind_when_name_unset() {
        let a = TestProvider::leaf("a") as Arc<dyn LlmProvider>;
        let response = build_providers_response(&[a]).await;
        assert_eq!(response["providers"][0]["name"], "mock");
    }

    #[tokio::test]
    async fn build_providers_response_honors_configured_models_allowlist() {
        let discovered = vec![
            ModelInfo { id: "gemini-2.5-flash".into(), display_name: None },
            ModelInfo { id: "gemini-2.5-pro".into(), display_name: None },
            ModelInfo { id: "gemini-3-pro-preview".into(), display_name: None },
        ];
        let a = TestProvider::with_allowlist(
            "a", discovered, vec!["gemini-2.5-pro".into(), "does-not-exist".into()],
        ) as Arc<dyn LlmProvider>;
        let response = build_providers_response(&[a]).await;
        let models = response["providers"][0]["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["id"], "gemini-2.5-pro");
    }

    #[tokio::test]
    async fn second_call_within_ttl_uses_cache_not_a_fresh_fetch() {
        let leaf = TestProvider::leaf("a");
        let state = LlmState::new(Arc::clone(&leaf) as Arc<dyn LlmProvider>);

        let _ = cached_or_fresh(&state, false).await;
        let _ = cached_or_fresh(&state, false).await;

        assert_eq!(leaf.list_models_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn force_refresh_bypasses_cache() {
        let leaf = TestProvider::leaf("a");
        let state = LlmState::new(Arc::clone(&leaf) as Arc<dyn LlmProvider>);

        let _ = cached_or_fresh(&state, false).await;
        let _ = cached_or_fresh(&state, true).await;

        assert_eq!(leaf.list_models_calls.load(Ordering::SeqCst), 2);
    }
}
