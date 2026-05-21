use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    types::{ContentPart, LlmResponse, Message, MessageContent, Role, ToolCall, ToolDefinition},
    LlmProvider,
};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    model: String,
    api_key: String,
    client: Client,
}

impl AnthropicProvider {
    pub fn new(model: String, api_key: String) -> Self {
        Self { model, api_key, client: Client::new() }
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
            "model":    self.model,
            "system":   system_text,
            "messages": api_messages,
            "tools":    api_tools,
            "max_tokens": 8192,
        });

        let resp = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;

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
                ContentPart::Text { text } =>
                    json!({ "type": "text", "text": text }),
                ContentPart::ToolUse { id, name, input } =>
                    json!({ "type": "tool_use", "id": id, "name": name, "input": input }),
                ContentPart::ToolResult { tool_use_id, content, is_error } =>
                    json!({ "type": "tool_result", "tool_use_id": tool_use_id,
                            "content": content, "is_error": is_error }),
            }).collect();
            Value::Array(items)
        }
    };

    json!({ "role": role, "content": content })
}

fn to_anthropic_tool(t: &ToolDefinition) -> Value {
    json!({
        "name":         t.name,
        "description":  t.description,
        "input_schema": t.parameters,
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
                id:    b["id"].as_str().unwrap_or("").to_string(),
                name:  b["name"].as_str().unwrap_or("").to_string(),
                input: b["input"].clone(),
            })
            .collect();
        return Ok(LlmResponse::ToolCalls(calls));
    }

    let text = content
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(LlmResponse::Message { text })
}
