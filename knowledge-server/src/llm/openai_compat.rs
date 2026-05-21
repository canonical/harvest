use anyhow::{bail, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use super::{
    types::{ContentPart, LlmResponse, Message, MessageContent, Role, ToolCall, ToolDefinition},
    LlmProvider,
};

pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: String,
    model: String,
    client: Client,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self { base_url, api_key, model, client: Client::new() }
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn chat(&self, messages: &[Message], tools: &[ToolDefinition]) -> Result<LlmResponse> {
        let api_messages: Vec<Value> = messages.iter().map(to_openai_message).collect();
        let api_tools: Vec<Value> = tools.iter().map(to_openai_function).collect();

        let mut body = json!({
            "model":    self.model,
            "messages": api_messages,
        });
        if !api_tools.is_empty() {
            body["tools"] = Value::Array(api_tools);
        }

        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let json: Value = resp.json().await?;

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
                    ContentPart::ToolUse { id, name, input } => Some(json!({
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

            let text: String = parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            json!({ "role": role, "content": text })
        }
    }
}

fn to_openai_function(t: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name":        t.name,
            "description": t.description,
            "parameters":  t.parameters,
        }
    })
}

fn parse_openai_response(json: Value) -> Result<LlmResponse> {
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
                id:   tc["id"].as_str().unwrap_or("").to_string(),
                name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                input: serde_json::from_str(
                    tc["function"]["arguments"].as_str().unwrap_or("{}"),
                ).unwrap_or(Value::Null),
            })
            .collect();
        return Ok(LlmResponse::ToolCalls(calls));
    }

    let text = message["content"].as_str().unwrap_or("").to_string();
    Ok(LlmResponse::Message { text })
}
