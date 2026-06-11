use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    retry,
    types::{ContentPart, LlmResponse, Message, MessageContent, Role, ToolCall, ToolDefinition},
    LlmProvider,
};

const API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const MAX_OUTPUT_TOKENS: u32 = 8192;
const OVERLOAD_STATUS_CODES: &[u16] = &[503];

pub struct GeminiProvider {
    model: String,
    api_key: String,
    client: Client,
    base_url: String,
    max_retries: u32,
}

impl GeminiProvider {
    pub fn new(model: String, api_key: String, timeout_secs: u64, max_retries: u32) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self { model, api_key, client, base_url: API_BASE.to_string(), max_retries }
    }

    #[cfg(test)]
    fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn endpoint_url(&self) -> String {
        format!("{}/{}:generateContent?key={}", self.base_url, self.model, self.api_key)
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    async fn chat(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        let system_text = messages
            .iter()
            .find(|m| matches!(m.role, Role::System))
            .and_then(|m| match &m.content {
                MessageContent::Text(t) => Some(t.clone()),
                _ => None,
            });

        let contents: Vec<Value> = messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .map(to_gemini_message)
            .collect();

        let mut body = json!({
            "contents": contents,
            "generationConfig": { "maxOutputTokens": MAX_OUTPUT_TOKENS },
        });

        if let Some(text) = system_text {
            body["system_instruction"] = json!({ "parts": [{ "text": text }] });
        }

        if !tools.is_empty() {
            body["tools"] = json!([{
                "function_declarations": tools.iter().map(to_gemini_tool).collect::<Vec<_>>()
            }]);
        }

        let url = self.endpoint_url();

        let response = retry::send_with_retry(
            self.max_retries,
            OVERLOAD_STATUS_CODES,
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

        parse_gemini_response(json)
    }
}

fn to_gemini_message(msg: &Message) -> Value {
    let role = match msg.role {
        Role::User | Role::Tool => "user",
        Role::Assistant         => "model",
        Role::System            => unreachable!("system filtered before calling to_gemini_message"),
    };

    match &msg.content {
        MessageContent::Text(t) => json!({ "role": role, "parts": [{ "text": t }] }),
        MessageContent::Parts(parts) => {
            let gemini_parts: Vec<Value> = parts.iter().map(|p| match p {
                ContentPart::Text { text, thought_signature } => {
                    let mut part = json!({ "text": text });
                    if let Some(sig) = thought_signature {
                        part["thoughtSignature"] = json!(sig);
                    }
                    part
                }
                ContentPart::ToolUse { name, input, thought_signature, .. } => {
                    let mut part = json!({ "functionCall": { "name": name, "args": input } });
                    if let Some(sig) = thought_signature {
                        part["thoughtSignature"] = json!(sig);
                    }
                    part
                }
                ContentPart::ToolResult { tool_use_id, content, .. } =>
                    json!({ "functionResponse": {
                        "name": tool_use_id,
                        "response": { "result": content }
                    }}),
                ContentPart::Image { media_type, data } =>
                    json!({ "inlineData": { "mimeType": media_type, "data": data } }),
                ContentPart::Document { media_type, data } =>
                    json!({ "inlineData": { "mimeType": media_type, "data": data } }),
            }).collect();
            json!({ "role": role, "parts": gemini_parts })
        }
    }
}

fn to_gemini_tool(tool: &ToolDefinition) -> Value {
    json!({
        "name":        tool.name,
        "description": tool.description,
        "parameters":  tool.parameters,
    })
}

fn parse_gemini_response(json: Value) -> Result<LlmResponse> {
    if let Some(err) = json.get("error") {
        bail!("Gemini API error: {err}");
    }

    let parts = json["candidates"][0]["content"]["parts"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let function_calls: Vec<&Value> = parts.iter()
        .filter(|p| p.get("functionCall").is_some())
        .collect();

    if !function_calls.is_empty() {
        let calls = function_calls.iter().map(|p| {
            let fc = &p["functionCall"];
            let name = fc["name"].as_str().unwrap_or("").to_string();
            ToolCall {
                id:                name.clone(),
                name,
                input:             fc["args"].clone(),
                thought_signature: p["thoughtSignature"].as_str().map(|s| s.to_string()),
            }
        }).collect();
        let preamble = parts.iter()
            .filter(|p| p.get("text").is_some() && p.get("thought").is_none())
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(LlmResponse::ToolCalls { calls, preamble });
    }

    let text = parts.iter()
        .filter(|p| p.get("text").is_some() && p["thought"].as_bool().unwrap_or(false) == false)
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(LlmResponse::Message { text })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;
    use serde_json::json;

    fn make_provider(base_url: &str) -> GeminiProvider {
        GeminiProvider::new("gemini-test".into(), "test-key".into(), 30, 0)
            .with_base_url(base_url)
    }

    // ── parse helpers ────────────────────────────────────────────────────────

    #[test]
    fn parse_stop_returns_message() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [{ "text": "Hello!" }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "Hello!"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_function_call_returns_tool_calls() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "search_symbols",
                            "args": { "query": "alpha" }
                        }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "search_symbols");
                assert_eq!(calls[0].input["query"], "alpha");
                assert!(calls[0].thought_signature.is_none());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_function_call_preserves_thought_signature() {
        // thoughtSignature (camelCase) is at the part level, not inside functionCall
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": { "name": "search_symbols", "args": { "query": "alpha" } },
                        "thoughtSignature": "CsIBCsMB=="
                    }],
                    "role": "model"
                }
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => {
                assert_eq!(calls[0].thought_signature.as_deref(), Some("CsIBCsMB=="));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn thought_signature_serialized_at_part_level_not_inside_function_call() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id:                "search".into(),
                name:              "search".into(),
                input:             json!({}),
                thought_signature: Some("CsIBCsMB==".into()),
            }]),
        };
        let v = to_gemini_message(&msg);
        // signature must be at part level with camelCase key
        assert_eq!(v["parts"][0]["thoughtSignature"], "CsIBCsMB==");
        // and NOT inside the functionCall object
        assert!(v["parts"][0]["functionCall"].get("thoughtSignature").is_none());
    }

    #[test]
    fn absent_thought_signature_omitted_from_part() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id:                "search".into(),
                name:              "search".into(),
                input:             json!({}),
                thought_signature: None,
            }]),
        };
        let v = to_gemini_message(&msg);
        assert!(v["parts"][0].get("thoughtSignature").is_none()
            || v["parts"][0]["thoughtSignature"].is_null());
    }

    #[test]
    fn parse_two_function_calls() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "functionCall": { "name": "tool_a", "args": {} } },
                        { "functionCall": { "name": "tool_b", "args": {} } }
                    ],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => assert_eq!(calls.len(), 2),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_multi_text_parts_joined() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "line one" },
                        { "text": "line two" }
                    ],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "line one\nline two"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_function_call_id_equals_name() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [{ "functionCall": { "name": "my_tool", "args": {} } }],
                    "role": "model"
                }
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { calls, .. } => assert_eq!(calls[0].id, calls[0].name),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parse_error_body_returns_err() {
        let json = json!({ "error": { "code": 400, "message": "bad request" } });
        assert!(parse_gemini_response(json).is_err());
    }

    // ── message serialisation ────────────────────────────────────────────────

    #[test]
    fn user_text_message_maps_to_user_role_with_parts() {
        let msg = Message::user("hello");
        let v = to_gemini_message(&msg);
        assert_eq!(v["role"], "user");
        assert_eq!(v["parts"][0]["text"], "hello");
    }

    #[test]
    fn assistant_text_message_maps_to_model_role() {
        let msg = Message::assistant_text("hi back");
        let v = to_gemini_message(&msg);
        assert_eq!(v["role"], "model");
        assert_eq!(v["parts"][0]["text"], "hi back");
    }

    #[test]
    fn tool_role_maps_to_user_role() {
        let msg = Message {
            role: Role::Tool,
            content: MessageContent::Text("result".into()),
        };
        let v = to_gemini_message(&msg);
        assert_eq!(v["role"], "user");
    }

    #[test]
    fn tool_use_part_maps_to_function_call() {
        let msg = Message {
            role: Role::Assistant,
            content: MessageContent::Parts(vec![ContentPart::ToolUse {
                id:                "ignored_id".into(),
                name:              "search".into(),
                input:             json!({ "q": "rust" }),
                thought_signature: None,
            }]),
        };
        let v = to_gemini_message(&msg);
        assert_eq!(v["role"], "model");
        let part = &v["parts"][0];
        assert_eq!(part["functionCall"]["name"], "search");
        assert_eq!(part["functionCall"]["args"]["q"], "rust");
    }

    #[test]
    fn tool_result_part_maps_to_function_response() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![ContentPart::ToolResult {
                tool_use_id: "search".into(),
                content:     "found it".into(),
                is_error:    false,
            }]),
        };
        let v = to_gemini_message(&msg);
        assert_eq!(v["role"], "user");
        let part = &v["parts"][0];
        assert_eq!(part["functionResponse"]["name"], "search");
        assert_eq!(part["functionResponse"]["response"]["result"], "found it");
    }

    #[test]
    fn image_content_maps_to_inline_data() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Text { text: "look".into(), thought_signature: None },
                ContentPart::Image { media_type: "image/png".into(), data: "abc123".into() },
            ]),
        };
        let v = to_gemini_message(&msg);
        let parts = v["parts"].as_array().unwrap();
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/png");
        assert_eq!(parts[1]["inlineData"]["data"], "abc123");
    }

    #[test]
    fn document_content_maps_to_inline_data() {
        let msg = Message {
            role: Role::User,
            content: MessageContent::Parts(vec![
                ContentPart::Document { media_type: "application/pdf".into(), data: "pdfdata".into() },
            ]),
        };
        let v = to_gemini_message(&msg);
        let parts = v["parts"].as_array().unwrap();
        assert_eq!(parts[0]["inlineData"]["mimeType"], "application/pdf");
        assert_eq!(parts[0]["inlineData"]["data"], "pdfdata");
    }

    // ── tool serialisation ───────────────────────────────────────────────────

    #[test]
    fn tool_definition_has_parameters_key() {
        let def = ToolDefinition {
            name:        "my_tool".into(),
            description: "does stuff".into(),
            parameters:  json!({ "type": "object", "properties": {} }),
        };
        let v = to_gemini_tool(&def);
        assert_eq!(v["name"], "my_tool");
        assert_eq!(v["description"], "does stuff");
        assert!(v.get("parameters").is_some(), "must use 'parameters' key");
        assert!(v.get("input_schema").is_none(), "must NOT use anthropic 'input_schema' key");
    }

    // ── HTTP integration ─────────────────────────────────────────────────────

    fn text_response_body() -> Value {
        json!({
            "candidates": [{
                "content": { "parts": [{ "text": "all good" }], "role": "model" },
                "finishReason": "STOP"
            }]
        })
    }

    fn function_call_response_body() -> Value {
        json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": { "name": "list_repositories", "args": {} }
                    }],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        })
    }

    #[tokio::test]
    async fn http_200_stop_returns_message() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/gemini-test:generateContent");
            then.status(200).json_body(text_response_body());
        });

        let provider = make_provider(&server.base_url());
        match provider.chat(&[Message::user("hi")], &[]).await.unwrap() {
            LlmResponse::Message { text } => assert_eq!(text, "all good"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_200_function_call_returns_tool_calls() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("POST").path("/gemini-test:generateContent");
            then.status(200).json_body(function_call_response_body());
        });

        let provider = make_provider(&server.base_url());
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
            when.method("POST").path("/gemini-test:generateContent");
            then.status(400).json_body(json!({ "error": { "message": "bad request" } }));
        });

        let provider = make_provider(&server.base_url());
        assert!(provider.chat(&[Message::user("hi")], &[]).await.is_err());
    }

    #[tokio::test]
    async fn http_request_carries_api_key_as_query_param() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/gemini-test:generateContent")
                .query_param("key", "test-key");
            then.status(200).json_body(text_response_body());
        });

        let provider = make_provider(&server.base_url());
        provider.chat(&[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn system_message_goes_into_system_instruction_field() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/gemini-test:generateContent")
                .body_includes(r#""system_instruction""#);
            then.status(200).json_body(text_response_body());
        });

        let provider = make_provider(&server.base_url());
        provider
            .chat(&[Message::system("be helpful"), Message::user("hi")], &[])
            .await
            .unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn tools_wrapped_in_function_declarations() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/gemini-test:generateContent")
                .body_includes(r#""function_declarations""#);
            then.status(200).json_body(text_response_body());
        });

        let tools = vec![ToolDefinition {
            name:        "search".into(),
            description: "search for things".into(),
            parameters:  json!({ "type": "object" }),
        }];
        let provider = make_provider(&server.base_url());
        provider.chat(&[Message::user("hi")], &tools).await.unwrap();
        mock.assert();
    }
}
