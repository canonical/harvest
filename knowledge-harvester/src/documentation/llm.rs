use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::LlmConfig;
use super::retry;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 8192;
const ANTHROPIC_OVERLOAD_CODES: &[u16] = &[529, 503];
const OPENAI_OVERLOAD_CODES: &[u16] = &[529, 502, 503];
const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const GEMINI_OVERLOAD_CODES: &[u16] = &[503];

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
        LlmConfig::Gemini { model, api_key, timeout_secs, max_retries } => {
            Box::new(GeminiClient::new(
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
            base_url: ANTHROPIC_API_URL.to_string(),
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
            "model":    self.model,
            "system":   system,
            "messages": [{"role": "user", "content": user}],
            "max_tokens": MAX_TOKENS,
        });

        let response = retry::send_with_retry(
            self.max_retries,
            ANTHROPIC_OVERLOAD_CODES,
            "Anthropic",
            || self.client
                .post(&self.base_url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send(),
        ).await?;

        let status = response.status();
        let body_text = response.text().await?;
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
        Ok(text)
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
            "max_tokens": MAX_TOKENS,
        });

        let response = retry::send_with_retry(
            self.max_retries,
            OPENAI_OVERLOAD_CODES,
            "OpenAI-compat",
            || {
                let mut req = self.client.post(&chat_url).json(&body);
                if !self.api_key.is_empty() {
                    req = req.bearer_auth(&self.api_key);
                }
                req.send()
            },
        ).await?;

        let status = response.status();
        let body_text = response.text().await?;
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
        Ok(text)
    }
}

pub struct GeminiClient {
    model: String,
    api_key: String,
    client: Client,
    base_url: String,
    max_retries: u32,
}

impl GeminiClient {
    pub fn new(model: String, api_key: String, timeout_secs: u64, max_retries: u32) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self { model, api_key, client, base_url: GEMINI_API_BASE.to_string(), max_retries }
    }

    #[cfg(test)]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn endpoint_url(&self) -> String {
        format!("{}/{}:generateContent?key={}", self.base_url, self.model, self.api_key)
    }
}

#[async_trait]
impl LlmClient for GeminiClient {
    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let mut body = json!({
            "contents": [{ "role": "user", "parts": [{ "text": user }] }],
            "generationConfig": { "maxOutputTokens": MAX_TOKENS },
        });

        if !system.is_empty() {
            body["system_instruction"] = json!({ "parts": [{ "text": system }] });
        }

        let url = self.endpoint_url();

        let response = retry::send_with_retry(
            self.max_retries,
            GEMINI_OVERLOAD_CODES,
            "Gemini",
            || self.client.post(&url).json(&body).send(),
        ).await?;

        let status = response.status();
        let body_text = response.text().await?;
        let json: Value = serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("Gemini API returned non-JSON (status {status}): {e}\nbody: {body_text}"))?;

        if !status.is_success() {
            bail!("Gemini API error {status}: {json}");
        }

        let text = json["candidates"][0]["content"]["parts"]
            .as_array()
            .and_then(|parts| parts.first())
            .and_then(|part| part["text"].as_str())
            .unwrap_or("")
            .to_string();
        Ok(text)
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

#[cfg(test)]
mod gemini_tests {
    use super::*;
    use httpmock::prelude::*;

    fn make_client(base_url: &str) -> GeminiClient {
        GeminiClient::new("gemini-test".into(), "test-key".into(), 30, 0)
            .with_base_url(base_url)
    }

    fn text_response() -> serde_json::Value {
        serde_json::json!({
            "candidates": [{
                "content": { "parts": [{ "text": "done" }], "role": "model" },
                "finishReason": "STOP"
            }]
        })
    }

    #[tokio::test]
    async fn http_200_returns_text() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/gemini-test:generateContent");
            then.status(200).json_body(text_response());
        });

        let client = make_client(&server.base_url());
        assert_eq!(client.complete("sys", "hi").await.unwrap(), "done");
    }

    #[tokio::test]
    async fn http_4xx_returns_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/gemini-test:generateContent");
            then.status(400).json_body(serde_json::json!({ "error": { "message": "bad" } }));
        });

        let client = make_client(&server.base_url());
        assert!(client.complete("sys", "hi").await.is_err());
    }

    #[tokio::test]
    async fn api_key_sent_as_query_param() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/gemini-test:generateContent")
                .query_param("key", "test-key");
            then.status(200).json_body(text_response());
        });

        let client = make_client(&server.base_url());
        client.complete("", "hi").await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn nonempty_system_goes_into_system_instruction() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/gemini-test:generateContent")
                .body_includes(r#""system_instruction""#);
            then.status(200).json_body(text_response());
        });

        let client = make_client(&server.base_url());
        client.complete("be helpful", "hi").await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn empty_system_omits_system_instruction() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/gemini-test:generateContent")
                .body_excludes(r#""system_instruction""#);
            then.status(200).json_body(text_response());
        });

        let client = make_client(&server.base_url());
        client.complete("", "hi").await.unwrap();
        mock.assert();
    }
}
