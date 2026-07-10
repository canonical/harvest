use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

impl Message {
    pub fn system(text: impl Into<String>) -> Self {
        Self { role: Role::System, content: MessageContent::Text(text.into()) }
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, content: MessageContent::Text(text.into()) }
    }
    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: MessageContent::Text(text.into()) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    Image {
        media_type: String,
        data: String,
    },
    Document {
        media_type: String,
        data: String,
    },
}

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
    pub thought_signature: Option<String>,
}

#[derive(Debug)]
pub enum LlmResponse {
    Message { text: String },
    ToolCalls { calls: Vec<ToolCall>, preamble: String },
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    ThinkingDelta { text: String },
    TextDelta { text: String },
    ToolCallReady(ToolCall),
    Done { stop_reason: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSelection {
    pub provider_id: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsedProvider {
    pub provider_id: String,
    pub kind: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub struct ProviderMeta {
    pub id: String,
    pub expose_to_ui: bool,
    pub name: Option<String>,
    pub models: Option<Vec<String>>,
}

impl ProviderMeta {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), expose_to_ui: true, name: None, models: None }
    }
}
