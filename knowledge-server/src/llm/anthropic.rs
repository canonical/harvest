use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    retry,
    types::{ContentPart, LlmResponse, Message, MessageContent, Role, ToolCall, ToolDefinition},
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
}
