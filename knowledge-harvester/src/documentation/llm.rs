use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::LlmConfig;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<String>;
}

pub fn from_config(config: &LlmConfig) -> Box<dyn LlmClient> {
    match config {
        LlmConfig::Anthropic { model, api_key, timeout_secs, max_retries } => {
            Box::new(AnthropicClient::new(
                model.clone(),
                api_key.clone(),
                *timeout_secs,
                *max_retries,
            ))
        }
        LlmConfig::OpenAiCompat { base_url, api_key, model, timeout_secs, max_retries } => {
            Box::new(OpenAiCompatClient::new(
                base_url.clone(),
                api_key.clone(),
                model.clone(),
                *timeout_secs,
                *max_retries,
            ))
        }
    }
}

pub struct AnthropicClient {
    model: String,
    api_key: String,
    client: Client,
    base_url: String,
    max_retries: u32,
}

impl AnthropicClient {
    pub fn new(model: String, api_key: String, timeout_secs: u64, max_retries: u32) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self {
            model,
            api_key,
            client,
            base_url: "https://api.anthropic.com/v1/messages".to_string(),
            max_retries,
        }
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "system": system,
            "messages": [{"role": "user", "content": user}],
            "max_tokens": 8192,
        });

        let mut attempt = 0u32;
        loop {
            let resp = match self.client
                .post(&self.base_url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) if e.is_timeout() && attempt < self.max_retries => {
                    attempt += 1;
                    let delay = 2u64 * (1u64 << attempt.min(4));
                    tracing::warn!(attempt, delay_secs = delay, "Anthropic request timed out — retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let status = resp.status();

            match status.as_u16() {
                429 if attempt < self.max_retries => {
                    let delay = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(30u64 * (1u64 << attempt.min(4)));
                    attempt += 1;
                    tracing::warn!(attempt, delay_secs = delay, "Anthropic rate limited — retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }
                529 | 503 if attempt < self.max_retries => {
                    attempt += 1;
                    tracing::warn!(attempt, status = %status, "Anthropic overloaded — retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                _ => {}
            }

            let body_text = resp.text().await?;
            let json: Value = serde_json::from_str(&body_text)
                .map_err(|e| anyhow::anyhow!("Anthropic API returned non-JSON (status {status}): {e}\nbody: {body_text}"))?;

            if !status.is_success() {
                bail!("Anthropic API error {status}: {json}");
            }

            let text = json["content"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|block| block["text"].as_str())
                .unwrap_or("")
                .to_string();
            return Ok(text);
        }
    }
}

pub struct OpenAiCompatClient {
    base_url: String,
    api_key: String,
    model: String,
    client: Client,
    max_retries: u32,
}

impl OpenAiCompatClient {
    pub fn new(
        base_url: String,
        api_key: String,
        model: String,
        timeout_secs: u64,
        max_retries: u32,
    ) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self { base_url, api_key, model, client, max_retries }
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl LlmClient for OpenAiCompatClient {
    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let chat_url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "max_tokens": 8192,
        });

        let mut attempt = 0u32;
        loop {
            let mut req = self.client.post(&chat_url).json(&body);
            if !self.api_key.is_empty() {
                req = req.bearer_auth(&self.api_key);
            }

            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) if e.is_timeout() && attempt < self.max_retries => {
                    attempt += 1;
                    let delay = 2u64 * (1u64 << attempt.min(4));
                    tracing::warn!(attempt, delay_secs = delay, "LLM request timed out — retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            let status = resp.status();

            match status.as_u16() {
                429 if attempt < self.max_retries => {
                    let delay = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(30u64 * (1u64 << attempt.min(4)));
                    attempt += 1;
                    tracing::warn!(attempt, delay_secs = delay, "rate limited — retrying");
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                    continue;
                }
                529 | 502 | 503 if attempt < self.max_retries => {
                    attempt += 1;
                    tracing::warn!(attempt, status = %status, "transient server error — retrying in 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
                _ => {}
            }

            let body_text = resp.text().await?;
            let json: Value = serde_json::from_str(&body_text)
                .map_err(|e| anyhow::anyhow!("OpenAI-compat API returned non-JSON (status {status}): {e}\nbody: {body_text}"))?;

            if !status.is_success() {
                bail!("OpenAI-compat API error {status}: {json}");
            }

            let text = json["choices"]
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|choice| choice["message"]["content"].as_str())
                .unwrap_or("")
                .to_string();
            return Ok(text);
        }
    }
}

#[cfg(test)]
pub struct MockLlmClient {
    pub response: String,
}

#[cfg(test)]
#[async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
        Ok(self.response.clone())
    }
}
