use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::StreamExt;
use futures_util::Stream;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct ClientConfig {
    pub base_url: String,
    pub token: Option<String>,
    pub cookie_store: Option<Arc<reqwest::cookie::Jar>>,
}

#[derive(Clone)]
pub struct Client {
    pub base_url: String,
    pub http: reqwest::Client,
}

impl Client {
    pub fn new(cfg: ClientConfig) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(15));

        if let Some(store) = cfg.cookie_store {
            builder = builder.cookie_provider(store);
        }

        let http = builder.build()?;

        Ok(Self {
            base_url: cfg.base_url,
            http,
        })
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}/{}", self.base_url, path)
        }
    }

    pub async fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let resp = self.http.get(self.url(path)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("GET {path} -> {status}: {body}"));
        }
        Ok(resp.json().await?)
    }

    pub async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self.http.post(self.url(path)).json(body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("POST {path} -> {status}: {body}"));
        }
        Ok(resp.json().await?)
    }

    pub async fn post_text<B: Serialize>(&self, path: &str, body: &B) -> Result<String> {
        let resp = self.http.post(self.url(path)).json(body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("POST {path} -> {status}: {body}"));
        }
        Ok(resp.text().await?)
    }

    pub async fn put_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self.http.put(self.url(path)).json(body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("PUT {path} -> {status}: {body}"));
        }
        Ok(resp.json().await?)
    }

    pub async fn delete_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let resp = self.http.delete(self.url(path)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("DELETE {path} -> {status}: {body}"));
        }
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, path: &str) -> Result<()> {
        let resp = self.http.delete(self.url(path)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("DELETE {path} -> {status}: {body}"));
        }
        Ok(())
    }

    pub async fn patch_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self.http.patch(self.url(path)).json(body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("PATCH {path} -> {status}: {body}"));
        }
        Ok(resp.json().await?)
    }

    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let resp = self.http.get(self.url(path)).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("GET {path} -> {status}: {body}"));
        }
        Ok(resp.bytes().await?.to_vec())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RepositoryInfo {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub group_id: String,
    #[serde(default)]
    pub group_name: String,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_by_name: String,
    #[serde(default)]
    pub message_count: i64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConversationDetail {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub messages: Vec<serde_json::Value>,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    pub start_line: i64,
    #[serde(default)]
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub total_nodes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SymbolSource {
    pub name: String,
    pub file: String,
    pub kind: String,
    pub start_line: i64,
    #[serde(default)]
    pub end_line: Option<i64>,
    #[serde(default)]
    pub signature: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub repo: String,
    pub version: String,
    pub file: String,
    pub line: u32,
    #[serde(default)]
    pub end_line: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UsedProvider {
    pub provider_id: String,
    pub kind: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryRequest {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProvidersResponse {
    pub providers: Vec<ProviderInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentInfo {
    pub id: String,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub online: bool,
    #[serde(default)]
    pub last_seen: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtifactInfo {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub created_by: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SkillInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UserInfo {
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub groups: Vec<GroupInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GroupInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub member_count: i64,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PortForward {
    pub id: String,
    #[serde(default)]
    pub port: i64,
    #[serde(default)]
    pub route_name: String,
    #[serde(default)]
    pub route_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentFlavor {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub cpu: String,
    #[serde(default)]
    pub memory: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstallTokenResponse {
    #[serde(default)]
    pub install_token: String,
    #[serde(default)]
    pub install_command: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MeResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub is_admin: bool,
    #[serde(default)]
    pub features: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeploymentInfo {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub infra_state: String,
    #[serde(default)]
    pub design_artifact_id: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    Intent {
        #[serde(default)]
        mode: IntentMode,
    },
    Phase {
        label: String,
    },
    Thinking {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    TextDelta {
        text: String,
    },
    ToolCall {
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        name: String,
        #[serde(default)]
        preview: String,
    },
    Done {
        answer: String,
        #[serde(default)]
        sources: Vec<Source>,
        #[serde(default)]
        tool_calls_made: usize,
        #[serde(default)]
        provider_used: Option<UsedProvider>,
        #[serde(default)]
        duration_ms: u64,
        #[serde(default)]
        hit_max_iterations: bool,
    },
    Error {
        message: String,
    },
    Question {
        question: String,
        #[serde(default)]
        choices: Vec<String>,
    },
    ConfirmAction {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
        description: String,
    },
    TitleUpdated {
        title: String,
    },
    ParallelResearchStarted {
        #[serde(default)]
        leads: Vec<String>,
    },
    ParallelResearchLeadDone {
        #[serde(default)]
        index: usize,
        #[serde(default)]
        iterations: usize,
        preview: String,
        #[serde(default)]
        duration_ms: u64,
    },
    ParallelResearchMergeStarted {
        #[serde(default)]
        duration_ms: u64,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum IntentMode {
    Conversational,
    Research,
    Action,
    Hybrid,
}

impl Default for IntentMode {
    fn default() -> Self {
        IntentMode::Conversational
    }
}

pub fn stream_query<'a>(
    client: &'a Client,
    path: &str,
    body: &'a QueryRequest,
) -> impl Stream<Item = Result<AgentEvent>> + 'a {
    let url = client.url(path);
    async_stream::stream! {
        let resp = match client.http.post(&url).json(body).send().await {
            Ok(r) => r,
            Err(e) => {
                yield Err(anyhow!(e));
                return;
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            yield Err(anyhow!("query stream -> {status}: {body}"));
            return;
        }
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Err(anyhow!(e));
                    return;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            loop {
                let Some(nl) = buf.find('\n') else { break };
                let line = buf[..nl].trim_end_matches('\r').to_string();
                buf.drain(..=nl);
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        return;
                    }
                    match serde_json::from_str::<AgentEvent>(data) {
                        Ok(ev) => yield Ok(ev),
                        Err(_) => {}
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_text_delta() {
        let s = r#"{"type":"text_delta","text":"hello"}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::TextDelta { text } => assert_eq!(text, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn deserializes_intent() {
        let s = r#"{"type":"intent","mode":"research"}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::Intent { mode } => assert_eq!(mode, IntentMode::Research),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn deserializes_tool_call() {
        let s = r#"{"type":"tool_call","name":"search_symbols","input":{"q":"retry"}}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::ToolCall { name, input } => {
                assert_eq!(name, "search_symbols");
                assert_eq!(input["q"], "retry");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn deserializes_done() {
        let s = r#"{"type":"done","answer":"ok","sources":[{"repo":"r","version":"v1","file":"f.rs","line":12}],"tool_calls_made":2,"provider_used":{"provider_id":"p","kind":"anthropic","model":"claude"},"duration_ms":500,"hit_max_iterations":false}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::Done { answer, sources, .. } => {
                assert_eq!(answer, "ok");
                assert_eq!(sources.len(), 1);
                assert_eq!(sources[0].file, "f.rs");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn deserializes_question() {
        let s = r#"{"type":"question","question":"which?","choices":["a","b"]}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::Question { question, choices } => {
                assert_eq!(question, "which?");
                assert_eq!(choices, vec!["a", "b"]);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn deserializes_confirm_action() {
        let s = r#"{"type":"confirm_action","id":"c1","name":"run_cmd","input":{"cmd":"rm"},"description":"dangerous"}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::ConfirmAction { id, name, description, .. } => {
                assert_eq!(id, "c1");
                assert_eq!(name, "run_cmd");
                assert_eq!(description, "dangerous");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn deserializes_error() {
        let s = r#"{"type":"error","message":"boom"}"#;
        let ev: AgentEvent = serde_json::from_str(s).unwrap();
        match ev {
            AgentEvent::Error { message } => assert_eq!(message, "boom"),
            _ => panic!("wrong variant"),
        }
    }
}
