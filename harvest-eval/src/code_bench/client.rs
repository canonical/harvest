use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use futures::StreamExt;
use knowledge_server::llm::types::Usage;
use reqwest::{Method, RequestBuilder};
use serde_json::{json, Value};

use super::report::TurnMetrics;

const POLL: Duration = Duration::from_secs(5);
const IMAGE_WAIT: Duration = Duration::from_secs(45 * 60);
const SESSION_WAIT: Duration = Duration::from_secs(10 * 60);

pub struct Harvest {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Harvest {
    pub fn new(base: &str, token: &str) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder().build()?,
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        })
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token)
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self.request(method.clone(), path);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.with_context(|| format!("{method} {path}"))?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            bail!("{method} {path} returned {status}: {text}");
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn open_project(&self, repository: &str) -> Result<String> {
        let project = self
            .call(Method::POST, "/code/projects", Some(json!({ "url": repository })))
            .await?;
        id_of(&project)
    }

    pub async fn wait_for_image(&self, project_id: &str) -> Result<()> {
        let started = Instant::now();
        loop {
            let project = self.call(Method::GET, &format!("/code/projects/{project_id}"), None).await?;
            match project["image"]["status"].as_str() {
                Some("ready") => return Ok(()),
                Some("failed") => bail!("the project's container setup failed: {}", project["image"]["log"]),
                _ if started.elapsed() > IMAGE_WAIT => bail!("the project's image was not ready in time"),
                _ => tokio::time::sleep(POLL).await,
            }
        }
    }

    pub async fn start_session(&self, project_id: &str, branch: &str, base: &str) -> Result<String> {
        let session = self
            .call(
                Method::POST,
                &format!("/code/projects/{project_id}/sessions"),
                Some(json!({ "branch": branch, "base_branch": base })),
            )
            .await?;
        let id = id_of(&session)?;
        let started = Instant::now();
        loop {
            let state = self.call(Method::GET, &format!("/code/sessions/{id}"), None).await?;
            match state["session"]["status"].as_str() {
                Some("active") => return Ok(id),
                Some("failed") => bail!("the session failed to start: {}", state["session"]["error"]),
                _ if started.elapsed() > SESSION_WAIT => bail!("the session did not start in time"),
                _ => tokio::time::sleep(POLL).await,
            }
        }
    }

    pub async fn new_chat(&self, session_id: &str) -> Result<String> {
        let chat = self
            .call(Method::POST, &format!("/code/sessions/{session_id}/chats"), None)
            .await?;
        id_of(&chat["chat"])
    }

    pub async fn send(&self, session_id: &str, chat_id: &str, prompt: &str) -> Result<TurnMetrics> {
        let started = Instant::now();
        let response = self
            .request(Method::POST, &format!("/code/sessions/{session_id}/query/stream"))
            .json(&json!({ "query": prompt, "conversation_id": chat_id }))
            .send()
            .await?;
        if !response.status().is_success() {
            bail!("the message was refused: {} {}", response.status(), response.text().await?);
        }
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(end) = buffer.find("\n\n") {
                let event: String = buffer.drain(..end + 2).collect();
                if let Some(outcome) = turn_outcome(&event, started.elapsed())? {
                    return Ok(outcome);
                }
            }
        }
        bail!("the stream ended without an answer")
    }

    pub async fn end_session(&self, session_id: &str) -> Result<()> {
        self.call(Method::DELETE, &format!("/code/sessions/{session_id}"), None)
            .await
            .map(|_| ())
    }
}

fn id_of(value: &Value) -> Result<String> {
    value["id"]
        .as_str()
        .map(String::from)
        .with_context(|| format!("the response has no id: {value}"))
}

pub fn turn_outcome(event: &str, elapsed: Duration) -> Result<Option<TurnMetrics>> {
    let Some(data) = event.lines().find_map(|line| line.strip_prefix("data:")) else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
        return Ok(None);
    };
    match value["type"].as_str() {
        Some("done") => Ok(Some(TurnMetrics {
            seconds: elapsed.as_secs_f64(),
            llm_calls: value["llm_call_count"].as_u64().unwrap_or(0),
            tool_calls: value["tool_calls_made"].as_u64().unwrap_or(0),
            usage: serde_json::from_value::<Usage>(value["usage"].clone()).unwrap_or_default(),
            cost_microusd: value["cost_microusd"].as_i64().unwrap_or(0),
        })),
        Some("error") => bail!("the agent failed: {}", value["message"]),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_done_event_becomes_turn_metrics() {
        let event = "data: {\"type\":\"done\",\"llm_call_count\":6,\"tool_calls_made\":5,\"cost_microusd\":90000,\"usage\":{\"input_tokens\":14523,\"output_tokens\":1758,\"cache_read_tokens\":68265,\"cache_creation_tokens\":0,\"reasoning_tokens\":0}}\n\n";
        let metrics = turn_outcome(event, Duration::from_secs(21)).unwrap().unwrap();
        assert_eq!(metrics.llm_calls, 6);
        assert_eq!(metrics.usage.cache_read_tokens, 68_265);
        assert_eq!(metrics.cost_microusd, 90_000);
        assert_eq!(metrics.seconds, 21.0);
    }

    #[test]
    fn other_events_are_skipped_and_errors_fail_the_turn() {
        assert_eq!(turn_outcome("data: {\"type\":\"text_delta\",\"text\":\"hi\"}\n\n", Duration::ZERO).unwrap(), None);
        assert_eq!(turn_outcome(": keep-alive\n\n", Duration::ZERO).unwrap(), None);
        assert!(turn_outcome("data: {\"type\":\"error\",\"message\":\"no key\"}\n\n", Duration::ZERO).is_err());
    }
}
