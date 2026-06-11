pub mod anthropic;
pub mod gemini;
pub mod openai_compat;
pub mod types;
mod retry;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::config::LlmConfig;
use types::{LlmResponse, Message, ToolDefinition};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse>;
}

pub fn from_config(config: &LlmConfig) -> Arc<dyn LlmProvider> {
    match config {
        LlmConfig::Anthropic { model, api_key, timeout_secs, max_retries, .. } => Arc::new(
            anthropic::AnthropicProvider::new(
                model.clone(),
                api_key.clone(),
                *timeout_secs,
                *max_retries,
            ),
        ),
        LlmConfig::Gemini { model, api_key, timeout_secs, max_retries, .. } => Arc::new(
            gemini::GeminiProvider::new(
                model.clone(),
                api_key.clone(),
                *timeout_secs,
                *max_retries,
            ),
        ),
        LlmConfig::OpenAiCompat { base_url, api_key, model, timeout_secs, max_retries, .. } => Arc::new(
            openai_compat::OpenAiCompatProvider::new(
                base_url.clone(),
                api_key.clone(),
                model.clone(),
                *timeout_secs,
                *max_retries,
            ),
        ),
    }
}
