use anyhow::{bail, Result};
use async_trait::async_trait;
use futures::StreamExt as _;
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;

use super::{
    retry,
    types::{ContentPart, LlmResponse, Message, MessageContent, Role, StreamEvent, ToolCall, ToolDefinition},
    LlmProvider,
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 8192;
const OVERLOAD_STATUS_CODES: &[u16] = &[529, 503];

pub struct AnthropicProvider {
    model: String,
    api_key: String,
    client: Client,
    base_url: String,
    max_retries: u32,
}

impl AnthropicProvider {
    pub fn new(model: String, api_key: String, timeout_secs: u64, max_retries: u32) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self { model, api_key, client, base_url: API_URL.to_string(), max_retries }
    }

    #[cfg(test)]
    fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        let system_text = messages
            .iter()
            .find(|m| matches!(m.role, Role::System))
            .and_then(|m| match &m.content {
                MessageContent::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();

        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(to_anthropic_message)
            .collect();

        let api_tools: Vec<Value> = tools.iter().map(to_anthropic_tool).collect();

        let body = json!({
            "model":      self.model,
            "system":     system_text,
            "messages":   api_messages,
            "tools":      api_tools,
            "max_tokens": MAX_TOKENS,
        });

        let response = retry::send_with_retry(
            self.max_retries,
            OVERLOAD_STATUS_CODES,
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

        parse_anthropic_response(json)
    }

    async fn chat_stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        self.stream(messages, tools, tx).await
    }
}

fn to_anthropic_message(msg: &Message) -> Value {
    let role = match msg.role {
        Role::User      => "user",
        Role::Assistant => "assistant",
        Role::Tool      => "user",
        Role::System    => unreachable!("system filtered above"),
    };

    let content: Value = match &msg.content {
        MessageContent::Text(t) => Value::String(t.clone()),
        MessageContent::Parts(parts) => {
            let items: Vec<Value> = parts.iter().map(|p| match p {
                ContentPart::Text { text, .. } =>
                    json!({ "type": "text", "text": text }),
                ContentPart::ToolUse { id, name, input, .. } =>
                    json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                ContentPart::ToolResult { tool_use_id, content, is_error } =>
                    json!({ "type": "tool_result", "tool_use_id": tool_use_id,
                            "content": content, "is_error": is_error }),
                ContentPart::Image { media_type, data } =>
                    json!({ "type": "image", "source": {
                        "type": "base64", "media_type": media_type, "data": data
                    }}),
                ContentPart::Document { media_type, data } =>
                    json!({ "type": "document", "source": {
                        "type": "base64", "media_type": media_type, "data": data
                    }}),
            }).collect();
            Value::Array(items)
        }
    };

    json!({ "role": role, "content": content })
}

fn to_anthropic_tool(tool: &ToolDefinition) -> Value {
    json!({
        "name":         tool.name,
        "description":  tool.description,
        "input_schema": tool.parameters,
    })
}

fn parse_anthropic_response(json: Value) -> Result<LlmResponse> {
    let stop_reason = json["stop_reason"].as_str().unwrap_or("");
    let content = json["content"].as_array().cloned().unwrap_or_default();

    if stop_reason == "tool_use" {
        let calls = content
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| ToolCall {
                id:                b["id"].as_str().unwrap_or("").to_string(),
                name:              b["name"].as_str().unwrap_or("").to_string(),
                input:             b["input"].clone(),
                thought_signature: None,
            })
            .collect();
        let preamble = content
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(LlmResponse::ToolCalls { calls, preamble });
    }

    let text = content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(LlmResponse::Message { text })
}

// ── Streaming ─────────────────────────────────────────────────────────────────

/// Tracks the accumulated state of a single content block during streaming.
enum BlockAccum {
    Text,
    Thinking,
    ToolUse { id: String, name: String, json_buf: String },
}

/// Process one parsed SSE event JSON object from Anthropic's streaming API.
async fn process_stream_event(
    event: &Value,
    blocks: &mut HashMap<usize, BlockAccum>,
    tx: &mpsc::Sender<StreamEvent>,
) -> Result<()> {
    match event["type"].as_str().unwrap_or("") {
        "content_block_start" => {
            let idx = event["index"].as_u64().unwrap_or(0) as usize;
            let cb  = &event["content_block"];
            match cb["type"].as_str().unwrap_or("") {
                "text"     => { blocks.insert(idx, BlockAccum::Text); }
                "thinking" => { blocks.insert(idx, BlockAccum::Thinking); }
                "tool_use" => {
                    blocks.insert(idx, BlockAccum::ToolUse {
                        id:       cb["id"].as_str().unwrap_or("").to_string(),
                        name:     cb["name"].as_str().unwrap_or("").to_string(),
                        json_buf: String::new(),
                    });
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let idx   = event["index"].as_u64().unwrap_or(0) as usize;
            let delta = &event["delta"];
            match delta["type"].as_str().unwrap_or("") {
                "text_delta" => {
                    let text = delta["text"].as_str().unwrap_or("").to_string();
                    if !text.is_empty() {
                        let _ = tx.send(StreamEvent::TextDelta { text }).await;
                    }
                }
                "thinking_delta" => {
                    let text = delta["thinking"].as_str().unwrap_or("").to_string();
                    if !text.is_empty() {
                        let _ = tx.send(StreamEvent::ThinkingDelta { text }).await;
                    }
                }
                "input_json_delta" => {
                    let partial = delta["partial_json"].as_str().unwrap_or("");
                    if let Some(BlockAccum::ToolUse { json_buf, .. }) = blocks.get_mut(&idx) {
                        json_buf.push_str(partial);
                    }
                }
                _ => {}
            }
        }
        "content_block_stop" => {
            let idx = event["index"].as_u64().unwrap_or(0) as usize;
            if let Some(BlockAccum::ToolUse { id, name, json_buf }) = blocks.remove(&idx) {
                let input: Value = serde_json::from_str(&json_buf)
                    .unwrap_or(Value::Object(serde_json::Map::new()));
                let _ = tx.send(StreamEvent::ToolCallReady(ToolCall {
                    id,
                    name,
                    input,
                    thought_signature: None,
                })).await;
            }
        }
        "message_delta" => {
            let stop_reason = event["delta"]["stop_reason"]
                .as_str()
                .unwrap_or("end_turn")
                .to_string();
            let _ = tx.send(StreamEvent::Done { stop_reason }).await;
        }
        // "message_start", "message_stop", "ping" → nothing to emit
        _ => {}
    }
    Ok(())
}

impl AnthropicProvider {
    /// Native Anthropic streaming: POST with `stream: true`, parse SSE events.
    pub async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<()> {
        let system_text = messages
            .iter()
            .find(|m| matches!(m.role, Role::System))
            .and_then(|m| match &m.content {
                MessageContent::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();

        let api_messages: Vec<Value> = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(to_anthropic_message)
            .collect();

        let api_tools: Vec<Value> = tools.iter().map(to_anthropic_tool).collect();

        let body = json!({
            "model":      self.model,
            "system":     system_text,
            "messages":   api_messages,
            "tools":      api_tools,
            "max_tokens": MAX_TOKENS,
            "stream":     true,
        });

        let response = retry::send_with_retry(
            self.max_retries,
            OVERLOAD_STATUS_CODES,
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
        if !status.is_success() {
            let body_text = response.text().await?;
            bail!("Anthropic API error {status}: {body_text}");
        }

        let mut byte_stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut blocks: HashMap<usize, BlockAccum> = HashMap::new();

        while let Some(chunk) = byte_stream.next().await {
            let bytes = chunk?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            // SSE events are separated by "\n\n"; process all complete ones.
            while let Some(pos) = buffer.find("\n\n") {
                let event_text = buffer[..pos].to_string();
                buffer.drain(..pos + 2);

                let data_line = event_text
                    .lines()
                    .find(|l| l.starts_with("data:"))
                    .map(|l| l[5..].trim().to_string());

                if let Some(data) = data_line {
                    if let Ok(json) = serde_json::from_str::<Value>(&data) {
                        process_stream_event(&json, &mut blocks, &tx).await?;
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    #[test]
    fn parse_end_turn_returns_message() {
        let json = json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "Hello!" }]
        });
        match parse_anthropic_response(json).unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "Hello!"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_tool_use_returns_tool_calls() {
        let json = json!({
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "id": "tu_001",
                "name": "search_symbols",
                "input": { "query": "alpha" }
            }]
        });
        match parse_anthropic_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "tu_001");
                assert_eq!(calls[0].name, "search_symbols");
                assert_eq!(calls[0].input["query"], "alpha");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_tool_use_multiple_calls() {
        let json = json!({
            "stop_reason": "tool_use",
            "content": [
                { "type": "tool_use", "id": "a", "name": "tool_a", "input": {} },
                { "type": "tool_use", "id": "b", "name": "tool_b", "input": {} }
            ]
        });
        match parse_anthropic_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => assert_eq!(calls.len(), 2),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_multi_text_blocks_joined() {
        let json = json!({
            "stop_reason": "end_turn",
            "content": [
                { "type": "text", "text": "line one" },
                { "type": "text", "text": "line two" }
            ]
        });
        match parse_anthropic_response(json).unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "line one\nline two"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn user_text_message_serializes_correctly() {
        let msg = Message::user("hello");
        let v = to_anthropic_message(&msg);
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"], "hello");
    }

    #[test]
    fn assistant_tool_use_parts_serialize_correctly() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id: "tu_1".into(),
                name: "my_tool".into(),
                input: json!({ "k": "v" }),
                thought_signature: None,
            }]),
        };
        let v = to_anthropic_message(&msg);
        assert_eq!(v["role"], "assistant");
        let parts = v["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "tool_use");
        assert_eq!(parts[0]["id"], "tu_1");
        assert_eq!(parts[0]["name"], "my_tool");
        assert_eq!(parts[0]["input"]["k"], "v");
    }

    #[test]
    fn user_tool_result_part_serializes_correctly() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "result text".into(),
                is_error: false,
            }]),
        };
        let v = to_anthropic_message(&msg);
        assert_eq!(v["role"], "user");
        let parts = v["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "tool_result");
        assert_eq!(parts[0]["tool_use_id"], "tu_1");
        assert_eq!(parts[0]["content"], "result text");
    }

    #[test]
    fn image_content_part_serializes_for_anthropic() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Text { text: "look at this".into(), thought_signature: None },
                ContentPart::Image { media_type: "image/png".into(), data: "abc123".into() },
            ]),
        };
        let v = to_anthropic_message(&msg);
        let parts = v["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image");
        assert_eq!(parts[1]["source"]["type"], "base64");
        assert_eq!(parts[1]["source"]["media_type"], "image/png");
        assert_eq!(parts[1]["source"]["data"], "abc123");
    }

    #[test]
    fn document_content_part_serializes_for_anthropic() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Document { media_type: "application/pdf".into(), data: "pdfdata".into() },
            ]),
        };
        let v = to_anthropic_message(&msg);
        let parts = v["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "document");
        assert_eq!(parts[0]["source"]["type"], "base64");
        assert_eq!(parts[0]["source"]["media_type"], "application/pdf");
        assert_eq!(parts[0]["source"]["data"], "pdfdata");
    }

    #[test]
    fn tool_definition_uses_input_schema_key() {
        let def = ToolDefinition {
            name: "my_tool".into(),
            description: "does stuff".into(),
            parameters: json!({ "type": "object", "properties": {} }),
        };
        let v = to_anthropic_tool(&def);
        assert_eq!(v["name"], "my_tool");
        assert_eq!(v["description"], "does stuff");
        assert!(v.get("input_schema").is_some(), "must use 'input_schema' key");
        assert!(v.get("parameters").is_none(), "must NOT use 'parameters' key");
    }

    fn text_response_body() -> serde_json::Value {
        json!({
            "stop_reason": "end_turn",
            "content": [{ "type": "text", "text": "all good" }]
        })
    }

    fn tool_use_response_body() -> serde_json::Value {
        json!({
            "stop_reason": "tool_use",
            "content": [{
                "type": "tool_use",
                "id": "tu_1",
                "name": "list_repositories",
                "input": {}
            }]
        })
    }

    #[tokio::test]
    async fn http_200_end_turn_returns_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200).json_body(text_response_body());
        });

        let provider = AnthropicProvider::new("claude-test".into(), "key".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        match provider.chat(&[Message::user("hi")], &[]).await.unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "all good"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_200_tool_use_returns_tool_calls() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200).json_body(tool_use_response_body());
        });

        let provider = AnthropicProvider::new("claude-test".into(), "key".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        match provider.chat(&[Message::user("hi")], &[]).await.unwrap() {
            LlmResponse::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_repositories");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_4xx_returns_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(401).json_body(json!({ "error": "unauthorized" }));
        });

        let provider = AnthropicProvider::new("claude-test".into(), "bad-key".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        assert!(provider.chat(&[Message::user("hi")], &[]).await.is_err());
    }

    #[tokio::test]
    async fn http_request_carries_api_key_and_version_headers() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/v1/messages")
                .header("x-api-key", "test-key")
                .header("anthropic-version", ANTHROPIC_VERSION);
            then.status(200).json_body(text_response_body());
        });

        let provider = AnthropicProvider::new("claude-test".into(), "test-key".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        provider.chat(&[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn system_message_goes_into_system_field_not_messages_array() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/v1/messages")
                .body_includes(r#""system":"be helpful""#);
            then.status(200).json_body(text_response_body());
        });

        let provider = AnthropicProvider::new("claude-test".into(), "k".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let result = provider
            .chat(&[Message::system("be helpful"), Message::user("hi")], &[])
            .await
            .unwrap();
        mock.assert();
        match result {
            LlmResponse::Message { .. } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    // ── Streaming tests ────────────────────────────────────────────────────────

    fn sse_body(events: &[Value]) -> String {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {}\n\n", e["type"].as_str().unwrap_or(""), e))
            .collect::<String>()
    }

    async fn collect_stream_events(provider: &AnthropicProvider) -> Vec<StreamEvent> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        provider.stream(&[Message::user("hi")], &[], tx).await.unwrap();
        let mut events = Vec::new();
        while let Ok(e) = rx.try_recv() {
            events.push(e);
        }
        events
    }

    #[tokio::test]
    async fn stream_text_delta_emits_text_delta_event() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse_body(&[
                    json!({ "type": "content_block_start",  "index": 0, "content_block": { "type": "text", "text": "" } }),
                    json!({ "type": "content_block_delta",  "index": 0, "delta": { "type": "text_delta", "text": "Hello" } }),
                    json!({ "type": "content_block_delta",  "index": 0, "delta": { "type": "text_delta", "text": " world" } }),
                    json!({ "type": "content_block_stop",   "index": 0 }),
                    json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } }),
                    json!({ "type": "message_stop" }),
                ]));
        });

        let provider = AnthropicProvider::new("m".into(), "k".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let events = collect_stream_events(&provider).await;

        let text_deltas: Vec<_> = events.iter().filter_map(|e| match e {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        }).collect();
        assert_eq!(text_deltas, vec!["Hello", " world"]);

        let done = events.iter().any(|e| matches!(e, StreamEvent::Done { stop_reason } if stop_reason == "end_turn"));
        assert!(done, "expected Done(end_turn) event");
    }

    #[tokio::test]
    async fn stream_thinking_delta_emits_thinking_delta_event() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse_body(&[
                    json!({ "type": "content_block_start",  "index": 0, "content_block": { "type": "thinking", "thinking": "" } }),
                    json!({ "type": "content_block_delta",  "index": 0, "delta": { "type": "thinking_delta", "thinking": "I should search" } }),
                    json!({ "type": "content_block_stop",   "index": 0 }),
                    json!({ "type": "content_block_start",  "index": 1, "content_block": { "type": "tool_use", "id": "tu_1", "name": "search", "input": {} } }),
                    json!({ "type": "content_block_delta",  "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{}" } }),
                    json!({ "type": "content_block_stop",   "index": 1 }),
                    json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
                ]));
        });

        let provider = AnthropicProvider::new("m".into(), "k".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let events = collect_stream_events(&provider).await;

        let thinking: Vec<_> = events.iter().filter_map(|e| match e {
            StreamEvent::ThinkingDelta { text } => Some(text.as_str()),
            _ => None,
        }).collect();
        assert_eq!(thinking, vec!["I should search"]);
    }

    #[tokio::test]
    async fn stream_tool_call_accumulates_partial_json() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse_body(&[
                    json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "tool_use", "id": "tu_42", "name": "search_symbols", "input": {} } }),
                    json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "input_json_delta", "partial_json": "{\"query\":" } }),
                    json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "input_json_delta", "partial_json": "\"hello\"}" } }),
                    json!({ "type": "content_block_stop",  "index": 0 }),
                    json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
                ]));
        });

        let provider = AnthropicProvider::new("m".into(), "k".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let events = collect_stream_events(&provider).await;

        let calls: Vec<_> = events.iter().filter_map(|e| match e {
            StreamEvent::ToolCallReady(c) => Some(c),
            _ => None,
        }).collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id,   "tu_42");
        assert_eq!(calls[0].name, "search_symbols");
        assert_eq!(calls[0].input["query"], "hello");
    }

    #[tokio::test]
    async fn stream_multiple_tool_calls_in_one_response() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse_body(&[
                    json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "tool_use", "id": "a", "name": "tool_a", "input": {} } }),
                    json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "input_json_delta", "partial_json": "{}" } }),
                    json!({ "type": "content_block_stop",  "index": 0 }),
                    json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "tool_use", "id": "b", "name": "tool_b", "input": {} } }),
                    json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{}" } }),
                    json!({ "type": "content_block_stop",  "index": 1 }),
                    json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
                ]));
        });

        let provider = AnthropicProvider::new("m".into(), "k".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let events = collect_stream_events(&provider).await;

        let call_names: Vec<_> = events.iter().filter_map(|e| match e {
            StreamEvent::ToolCallReady(c) => Some(c.name.as_str()),
            _ => None,
        }).collect();
        assert_eq!(call_names, vec!["tool_a", "tool_b"]);
    }

    #[tokio::test]
    async fn stream_4xx_returns_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(401).body("unauthorized");
        });

        let provider = AnthropicProvider::new("m".into(), "bad-key".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let result = provider.stream(&[Message::user("hi")], &[], tx).await;
        assert!(result.is_err(), "expected error on 4xx");
    }

    #[tokio::test]
    async fn stream_ping_events_are_ignored() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/v1/messages");
            then.status(200)
                .header("content-type", "text/event-stream")
                .body(sse_body(&[
                    json!({ "type": "ping" }),
                    json!({ "type": "content_block_start",  "index": 0, "content_block": { "type": "text", "text": "" } }),
                    json!({ "type": "content_block_delta",  "index": 0, "delta": { "type": "text_delta", "text": "Hi!" } }),
                    json!({ "type": "content_block_stop",   "index": 0 }),
                    json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" } }),
                ]));
        });

        let provider = AnthropicProvider::new("m".into(), "k".into(), 30, 0)
            .with_base_url(server.url("/v1/messages"));
        let events = collect_stream_events(&provider).await;

        // Only TextDelta + Done — no "ping" events
        assert!(events.iter().any(|e| matches!(e, StreamEvent::TextDelta { .. })));
        assert!(events.iter().any(|e| matches!(e, StreamEvent::Done { .. })));
        assert_eq!(events.len(), 2, "unexpected extra events: {events:?}");
    }
}
