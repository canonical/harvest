use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    retry,
    types::{ContentPart, LlmResponse, Message, MessageContent, ModelInfo, ProviderMeta, Role, ToolCall, ToolDefinition},
    LlmProvider,
};

const OVERLOAD_STATUS_CODES: &[u16] = &[502, 503];

pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: Client,
    max_retries: u32,
    meta: ProviderMeta,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: String, api_key: String, model: String, timeout_secs: u64, max_retries: u32, meta: ProviderMeta) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self { base_url, api_key, model, client, max_retries, meta }
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    fn id(&self) -> &str { &self.meta.id }
    fn kind(&self) -> &str { "openai-compatible" }
    fn default_model(&self) -> &str { &self.model }
    fn name(&self) -> Option<&str> { self.meta.name.as_deref() }
    fn expose_to_ui(&self) -> bool { self.meta.expose_to_ui }
    fn configured_models(&self) -> Option<&[String]> { self.meta.models.as_deref() }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let fallback = vec![ModelInfo { id: self.model.clone(), display_name: None }];
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let mut req = self.client.get(&url);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let Ok(response) = req.send().await else { return Ok(fallback) };
        if !response.status().is_success() {
            return Ok(fallback);
        }
        let Ok(json) = response.json::<Value>().await else { return Ok(fallback) };
        let Some(data) = json["data"].as_array() else { return Ok(fallback) };
        let models: Vec<ModelInfo> = data.iter()
            .filter_map(|m| m["id"].as_str())
            .map(|id| ModelInfo { id: id.to_string(), display_name: None })
            .collect();
        if models.is_empty() { Ok(fallback) } else { Ok(models) }
    }

    async fn chat_with(&self, model: Option<&str>, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        let api_messages: Vec<Value> = messages.iter().map(to_openai_message).collect();
        let api_tools: Vec<Value> = tools.iter().map(to_openai_function).collect();

        let mut body = json!({
            "model":    model.unwrap_or(&self.model),
            "messages": api_messages,
        });
        if !api_tools.is_empty() {
            body["tools"] = Value::Array(api_tools);
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let response = retry::send_with_retry(
            self.max_retries,
            OVERLOAD_STATUS_CODES,
            "OpenAI-compat",
            || {
                let mut req = self.client.post(&url).json(&body);
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

        parse_openai_response(json)
    }
}

fn to_openai_message(msg: &Message) -> Value {
    let role = match msg.role {
        Role::System    => "system",
        Role::User      => "user",
        Role::Assistant => "assistant",
        Role::Tool      => "tool",
    };

    match &msg.content {
        MessageContent::Text(t) => json!({ "role": role, "content": t }),
        MessageContent::Parts(parts) => {
            let tool_calls: Vec<Value> = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::ToolUse { id, name, input, .. } => Some(json!({
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": input.to_string() }
                    })),
                    _ => None,
                })
                .collect();

            if !tool_calls.is_empty() {
                return json!({ "role": "assistant", "tool_calls": tool_calls });
            }

            if let Some(ContentPart::ToolResult { tool_use_id, content, .. }) = parts.first() {
                return json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": content,
                });
            }

            let has_media = parts.iter().any(|p| {
                matches!(p, ContentPart::Image { .. } | ContentPart::Document { .. })
            });

            if has_media {
                let content_items: Vec<Value> = parts.iter().filter_map(|p| match p {
                    ContentPart::Text { text, .. } =>
                        Some(json!({ "type": "text", "text": text })),
                    ContentPart::Image { media_type, data } =>
                        Some(json!({ "type": "image_url", "image_url": {
                            "url": format!("data:{};base64,{}", media_type, data)
                        }})),
                    ContentPart::Document { .. } =>
                        Some(json!({ "type": "text", "text": "[Attached PDF document]" })),
                    _ => None,
                }).collect();
                return json!({ "role": role, "content": content_items });
            }

            let text: String = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            json!({ "role": role, "content": text })
        }
    }
}

fn to_openai_function(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name":        tool.name,
            "description": tool.description,
            "parameters":  tool.parameters,
        }
    })
}

fn parse_openai_response(json: Value) -> Result<LlmResponse> {
    if let Some(err) = json.get("error") {
        bail!("LLM API error: {err}");
    }
    let choice = &json["choices"][0];
    let message = &choice["message"];
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

    if finish_reason == "tool_calls" {
        let calls = message["tool_calls"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|tc| ToolCall {
                id:                tc["id"].as_str().unwrap_or("").to_string(),
                name:              tc["function"]["name"].as_str().unwrap_or("").to_string(),
                input:             serde_json::from_str(
                    tc["function"]["arguments"].as_str().unwrap_or("{}"),
                ).unwrap_or(Value::Null),
                thought_signature: None,
            })
            .collect();
        let preamble = message["content"].as_str().unwrap_or("").to_string();
        return Ok(LlmResponse::ToolCalls { calls, preamble });
    }

    let text = message["content"].as_str().unwrap_or("").to_string();
    Ok(LlmResponse::Message { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    fn make_provider(base_url: &str) -> OpenAiCompatProvider {
        OpenAiCompatProvider::new(base_url.into(), "test-key".into(), "test-model".into(), 30, 0, ProviderMeta::new("oai-1"))
    }

    #[test]
    fn parse_stop_returns_message() {
        let json = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "Hello!" }
            }]
        });
        match parse_openai_response(json).unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "Hello!"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_tool_calls_finish_reason_returns_tool_calls() {
        let json = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "tool_calls": [{
                        "id": "call_1",
                        "function": {
                            "name": "search_symbols",
                            "arguments": "{\"query\":\"alpha\"}"
                        }
                    }]
                }
            }]
        });
        match parse_openai_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "search_symbols");
                assert_eq!(calls[0].input["query"], "alpha");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_two_tool_calls() {
        let json = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "tool_calls": [
                        { "id": "a", "function": { "name": "tool_a", "arguments": "{}" } },
                        { "id": "b", "function": { "name": "tool_b", "arguments": "{}" } }
                    ]
                }
            }]
        });
        match parse_openai_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => assert_eq!(calls.len(), 2),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_malformed_arguments_defaults_to_null() {
        let json = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "tool_calls": [{
                        "id": "x",
                        "function": { "name": "t", "arguments": "NOT JSON" }
                    }]
                }
            }]
        });
        match parse_openai_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => assert_eq!(calls[0].input, serde_json::Value::Null),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn system_message_maps_to_system_role() {
        let msg = Message::system("be helpful");
        let v = to_openai_message(&msg);
        assert_eq!(v["role"], "system");
        assert_eq!(v["content"], "be helpful");
    }

    #[test]
    fn user_text_message_maps_to_user_role() {
        let msg = Message::user("hello");
        let v = to_openai_message(&msg);
        assert_eq!(v["role"], "user");
        assert_eq!(v["content"], "hello");
    }

    #[test]
    fn assistant_tool_use_parts_produce_tool_calls_array() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id: "call_1".into(),
                name: "my_tool".into(),
                input: json!({ "k": "v" }),
                thought_signature: None,
            }]),
        };
        let v = to_openai_message(&msg);
        assert_eq!(v["role"], "assistant");
        let tool_calls = v["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls[0]["id"], "call_1");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "my_tool");
    }

    #[test]
    fn tool_result_part_maps_to_tool_role() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "call_1".into(),
                content: "result text".into(),
                is_error: false,
            }]),
        };
        let v = to_openai_message(&msg);
        assert_eq!(v["role"], "tool");
        assert_eq!(v["tool_call_id"], "call_1");
        assert_eq!(v["content"], "result text");
    }

    #[test]
    fn image_content_part_serializes_as_image_url() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Text { text: "look at this".into(), thought_signature: None },
                ContentPart::Image { media_type: "image/jpeg".into(), data: "abc123".into() },
            ]),
        };
        let v = to_openai_message(&msg);
        let content = v["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "look at this");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/jpeg;base64,abc123");
    }

    #[test]
    fn document_content_part_falls_back_to_text_for_openai() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Text { text: "read this".into(), thought_signature: None },
                ContentPart::Document { media_type: "application/pdf".into(), data: "pdfdata".into() },
            ]),
        };
        let v = to_openai_message(&msg);
        let content = v["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "text");
        assert!(content[1]["text"].as_str().unwrap().contains("PDF"));
    }

    #[test]
    fn tool_definition_has_function_type_wrapper() {
        let def = ToolDefinition {
            name: "my_tool".into(),
            description: "does stuff".into(),
            parameters: json!({ "type": "object" }),
        };
        let v = to_openai_function(&def);
        assert_eq!(v["type"], "function");
        assert_eq!(v["function"]["name"], "my_tool");
        assert_eq!(v["function"]["description"], "does stuff");
        assert!(v["function"].get("parameters").is_some());
        assert!(v.get("name").is_none(), "name must be nested, not top-level");
    }

    fn text_response() -> serde_json::Value {
        json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "done" }
            }]
        })
    }

    fn tool_response() -> serde_json::Value {
        json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "tool_calls": [{
                        "id": "c1",
                        "function": { "name": "list_repositories", "arguments": "{}" }
                    }]
                }
            }]
        })
    }

    #[tokio::test]
    async fn http_200_stop_returns_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/chat/completions");
            then.status(200).json_body(text_response());
        });

        let provider = make_provider(&server.base_url());
        match provider.chat(&[Message::user("hi")], &[]).await.unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "done"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_200_tool_calls_returns_tool_calls() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/chat/completions");
            then.status(200).json_body(tool_response());
        });

        let provider = make_provider(&server.base_url());
        match provider.chat(&[Message::user("hi")], &[]).await.unwrap() {
            LlmResponse::ToolCalls { calls, .. } => assert_eq!(calls[0].name, "list_repositories"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_4xx_returns_error() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/chat/completions");
            then.status(403).json_body(json!({ "error": "forbidden" }));
        });

        let provider = make_provider(&server.base_url());
        assert!(provider.chat(&[Message::user("hi")], &[]).await.is_err());
    }

    #[tokio::test]
    async fn http_request_carries_bearer_auth() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/chat/completions")
                .header("authorization", "Bearer test-key");
            then.status(200).json_body(text_response());
        });

        let provider = make_provider(&server.base_url());
        provider.chat(&[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn chat_with_model_override_sends_that_model() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/chat/completions")
                .body_includes(r#""model":"override-model""#);
            then.status(200).json_body(text_response());
        });

        let provider = make_provider(&server.base_url());
        provider.chat_with(Some("override-model"), &[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn chat_with_none_uses_configured_default_model() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/chat/completions")
                .body_includes(r#""model":"test-model""#);
            then.status(200).json_body(text_response());
        });

        let provider = make_provider(&server.base_url());
        provider.chat_with(None, &[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn list_models_parses_openai_shape() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/models");
            then.status(200).json_body(json!({
                "object": "list",
                "data": [
                    { "id": "gpt-4o", "object": "model" },
                    { "id": "gpt-4o-mini", "object": "model" },
                ]
            }));
        });

        let provider = make_provider(&server.base_url());
        let models = provider.list_models().await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
        assert_eq!(models[1].id, "gpt-4o-mini");
    }

    #[tokio::test]
    async fn list_models_falls_back_to_default_on_error_status() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/models");
            then.status(404);
        });

        let provider = make_provider(&server.base_url());
        let models = provider.list_models().await.unwrap();
        assert_eq!(models, vec![ModelInfo { id: "test-model".into(), display_name: None }]);
    }

    #[test]
    fn expose_to_ui_reflects_constructor_value() {
        let visible = OpenAiCompatProvider::new("http://x".into(), "k".into(), "m".into(), 30, 0, ProviderMeta::new("a"));
        let hidden  = OpenAiCompatProvider::new("http://x".into(), "k".into(), "m".into(), 30, 0, ProviderMeta { id: "b".into(), expose_to_ui: false, name: None, models: None });
        assert!(visible.expose_to_ui());
        assert!(!hidden.expose_to_ui());
    }

    #[test]
    fn name_reflects_constructor_value() {
        let named   = OpenAiCompatProvider::new("http://x".into(), "k".into(), "m".into(), 30, 0, ProviderMeta { id: "a".into(), expose_to_ui: true, name: Some("Lemonade (local)".into()), models: None });
        let unnamed = OpenAiCompatProvider::new("http://x".into(), "k".into(), "m".into(), 30, 0, ProviderMeta::new("b"));
        assert_eq!(named.name(), Some("Lemonade (local)"));
        assert_eq!(unnamed.name(), None);
    }

    #[tokio::test]
    async fn list_models_falls_back_to_default_on_malformed_body() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/models");
            then.status(200).body("not json");
        });

        let provider = make_provider(&server.base_url());
        let models = provider.list_models().await.unwrap();
        assert_eq!(models, vec![ModelInfo { id: "test-model".into(), display_name: None }]);
    }

    #[tokio::test]
    async fn url_appends_chat_completions_path() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/chat/completions");
            then.status(200).json_body(text_response());
        });

        let provider = make_provider(&server.base_url());
        assert!(provider.chat(&[Message::user("hi")], &[]).await.is_ok());
    }
}
