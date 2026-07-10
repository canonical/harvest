use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    retry,
    types::{ContentPart, LlmResponse, Message, MessageContent, ModelInfo, ProviderMeta, Role, ToolCall, ToolDefinition},
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
    meta: ProviderMeta,
}

impl GeminiProvider {
    pub fn new(model: String, api_key: String, timeout_secs: u64, max_retries: u32, meta: ProviderMeta) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .build()
            .expect("failed to build HTTP client");
        Self { model, api_key, client, base_url: API_BASE.to_string(), max_retries, meta }
    }

    #[cfg(test)]
    fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn endpoint_url(&self, model: &str) -> String {
        format!("{}/{}:generateContent?key={}", self.base_url, model, self.api_key)
    }

    fn models_url(&self) -> String {
        format!("{}?key={}", self.base_url, self.api_key)
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn id(&self) -> &str { &self.meta.id }
    fn kind(&self) -> &str { "gemini" }
    fn default_model(&self) -> &str { &self.model }
    fn expose_to_ui(&self) -> bool { self.meta.expose_to_ui }
    fn name(&self) -> Option<&str> { self.meta.name.as_deref() }
    fn configured_models(&self) -> Option<&[String]> { self.meta.models.as_deref() }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let fallback = vec![ModelInfo { id: self.model.clone(), display_name: None }];
        let Ok(response) = self.client.get(self.models_url()).send().await else { return Ok(fallback) };
        if !response.status().is_success() {
            return Ok(fallback);
        }
        let Ok(json) = response.json::<Value>().await else { return Ok(fallback) };
        let Some(models) = json["models"].as_array() else { return Ok(fallback) };
        let parsed: Vec<ModelInfo> = models.iter()
            .filter(|m| {
                m["supportedGenerationMethods"].as_array()
                    .map(|methods| methods.iter().any(|v| v.as_str() == Some("generateContent")))
                    .unwrap_or(false)
            })
            .filter(|m| {
                m["name"].as_str()
                    .map(|name| is_chat_model(name.strip_prefix("models/").unwrap_or(name)))
                    .unwrap_or(false)
            })
            .filter_map(|m| m["name"].as_str().map(|name| ModelInfo {
                id: name.strip_prefix("models/").unwrap_or(name).to_string(),
                display_name: m["displayName"].as_str().map(str::to_string),
            }))
            .collect();
        if parsed.is_empty() { Ok(fallback) } else { Ok(parsed) }
    }

    async fn chat_with(&self, model: Option<&str>, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        let resolved_model = model.unwrap_or(&self.model);
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

        let url = self.endpoint_url(resolved_model);

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

const NON_CHAT_KEYWORDS: &[&str] = &["tts", "image", "robotics", "embedding", "aqa"];

fn is_chat_model(id: &str) -> bool {
    let is_gemini_family = id.starts_with("gemini-") || id.starts_with("gemma-");
    let has_excluded_capability = NON_CHAT_KEYWORDS.iter().any(|kw| id.contains(kw));
    is_gemini_family && !has_excluded_capability
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
            .filter(|p| p.get("text").is_some())
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
        GeminiProvider::new("gemini-test".into(), "test-key".into(), 30, 0, ProviderMeta::new("gemini-1"))
            .with_base_url(base_url)
    }

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
        assert_eq!(v["parts"][0]["thoughtSignature"], "CsIBCsMB==");
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
    async fn chat_with_model_override_hits_that_models_endpoint() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST").path("/override-model:generateContent");
            then.status(200).json_body(text_response_body());
        });

        let provider = make_provider(&server.base_url());
        provider.chat_with(Some("override-model"), &[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn chat_with_none_uses_configured_default_model() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method("POST").path("/gemini-test:generateContent");
            then.status(200).json_body(text_response_body());
        });

        let provider = make_provider(&server.base_url());
        provider.chat_with(None, &[Message::user("hi")], &[]).await.unwrap();
        mock.assert();
    }

    #[tokio::test]
    async fn list_models_parses_gemini_shape_and_filters_unsupported() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/");
            then.status(200).json_body(json!({
                "models": [
                    {
                        "name": "models/gemini-2.5-flash",
                        "displayName": "Gemini 2.5 Flash",
                        "supportedGenerationMethods": ["generateContent"]
                    },
                    {
                        "name": "models/embedding-001",
                        "displayName": "Embedding",
                        "supportedGenerationMethods": ["embedContent"]
                    }
                ]
            }));
        });

        let provider = make_provider(&server.base_url());
        let models = provider.list_models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-2.5-flash");
        assert_eq!(models[0].display_name.as_deref(), Some("Gemini 2.5 Flash"));
    }

    #[test]
    fn is_chat_model_accepts_plain_gemini_and_gemma_models() {
        assert!(is_chat_model("gemini-2.5-flash"));
        assert!(is_chat_model("gemini-3.1-pro-preview"));
        assert!(is_chat_model("gemma-4-26b-a4b-it"));
    }

    #[test]
    fn is_chat_model_rejects_tts_image_and_robotics_variants() {
        assert!(!is_chat_model("gemini-2.5-flash-preview-tts"));
        assert!(!is_chat_model("gemini-2.5-flash-image"));
        assert!(!is_chat_model("gemini-3-pro-image-preview"));
        assert!(!is_chat_model("gemini-robotics-er-1.5-preview"));
    }

    #[test]
    fn is_chat_model_rejects_non_gemini_families() {
        assert!(!is_chat_model("lyria-3-pro-preview"));
        assert!(!is_chat_model("nano-banana-pro-preview"));
        assert!(!is_chat_model("deep-research-preview-04-2026"));
        assert!(!is_chat_model("antigravity-preview-05-2026"));
    }

    #[tokio::test]
    async fn list_models_excludes_tts_image_and_non_gemini_entries() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/");
            then.status(200).json_body(json!({
                "models": [
                    { "name": "models/gemini-2.5-flash", "displayName": "Gemini 2.5 Flash", "supportedGenerationMethods": ["generateContent"] },
                    { "name": "models/gemini-2.5-flash-preview-tts", "displayName": "Gemini 2.5 Flash Preview TTS", "supportedGenerationMethods": ["generateContent"] },
                    { "name": "models/gemini-2.5-flash-image", "displayName": "Nano Banana", "supportedGenerationMethods": ["generateContent"] },
                    { "name": "models/lyria-3-pro-preview", "displayName": "Lyria 3 Pro Preview", "supportedGenerationMethods": ["generateContent"] },
                    { "name": "models/gemini-robotics-er-1.5-preview", "displayName": "Gemini Robotics-ER 1.5 Preview", "supportedGenerationMethods": ["generateContent"] },
                    { "name": "models/gemma-4-31b-it", "displayName": "Gemma 4 31B IT", "supportedGenerationMethods": ["generateContent"] }
                ]
            }));
        });

        let provider = make_provider(&server.base_url());
        let models = provider.list_models().await.unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["gemini-2.5-flash", "gemma-4-31b-it"]);
    }

    #[tokio::test]
    async fn list_models_falls_back_to_default_on_error_status() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method("GET").path("/");
            then.status(500);
        });

        let provider = make_provider(&server.base_url());
        let models = provider.list_models().await.unwrap();
        assert_eq!(models, vec![ModelInfo { id: "gemini-test".into(), display_name: None }]);
    }

    #[test]
    fn expose_to_ui_reflects_constructor_value() {
        let visible = GeminiProvider::new("m".into(), "k".into(), 30, 0, ProviderMeta::new("a"));
        let hidden  = GeminiProvider::new("m".into(), "k".into(), 30, 0, ProviderMeta { id: "b".into(), expose_to_ui: false, name: None, models: None });
        assert!(visible.expose_to_ui());
        assert!(!hidden.expose_to_ui());
    }

    #[test]
    fn name_reflects_constructor_value() {
        let named   = GeminiProvider::new("m".into(), "k".into(), 30, 0, ProviderMeta { id: "a".into(), expose_to_ui: true, name: Some("Gemini Direct".into()), models: None });
        let unnamed = GeminiProvider::new("m".into(), "k".into(), 30, 0, ProviderMeta::new("b"));
        assert_eq!(named.name(), Some("Gemini Direct"));
        assert_eq!(unnamed.name(), None);
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

    #[test]
    fn thought_part_text_captured_as_preamble() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "Let me think about this.", "thought": true },
                        { "functionCall": { "name": "search_symbols", "args": { "query": "foo" } } }
                    ],
                    "role": "model"
                }
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { preamble, calls } => {
                assert_eq!(calls.len(), 1);
                assert!(preamble.contains("Let me think about this."),
                    "thought text should appear in preamble, got: {preamble:?}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn multiple_thought_parts_joined_in_preamble() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "First thought.", "thought": true },
                        { "text": "Second thought.", "thought": true },
                        { "functionCall": { "name": "my_tool", "args": {} } }
                    ],
                    "role": "model"
                }
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { preamble, .. } => {
                assert!(preamble.contains("First thought."),  "first thought missing");
                assert!(preamble.contains("Second thought."), "second thought missing");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn thought_parts_and_normal_text_both_in_preamble() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "I'll search for X.", "thought": true },
                        { "text": "Using the search tool." },
                        { "functionCall": { "name": "search_symbols", "args": {} } }
                    ],
                    "role": "model"
                }
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::ToolCalls { preamble, .. } => {
                assert!(preamble.contains("I'll search for X."),   "thought text missing");
                assert!(preamble.contains("Using the search tool."), "normal text missing");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn thought_parts_excluded_from_final_message_text() {
        let json = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "Internal reasoning here.", "thought": true },
                        { "text": "Here is the answer." }
                    ],
                    "role": "model"
                },
                "finishReason": "STOP"
            }]
        });
        match parse_gemini_response(json).unwrap() {
            LlmResponse::Message { text } => {
                assert!(!text.contains("Internal reasoning here."),
                    "thought text must not appear in final message");
                assert!(text.contains("Here is the answer."));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
