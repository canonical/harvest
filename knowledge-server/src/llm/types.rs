use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.cache_read_tokens
            + self.cache_creation_tokens
            + self.reasoning_tokens
    }
}

impl std::ops::Add for Usage {
    type Output = Usage;
    fn add(self, rhs: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens + rhs.input_tokens,
            output_tokens: self.output_tokens + rhs.output_tokens,
            cache_read_tokens: self.cache_read_tokens + rhs.cache_read_tokens,
            cache_creation_tokens: self.cache_creation_tokens + rhs.cache_creation_tokens,
            reasoning_tokens: self.reasoning_tokens + rhs.reasoning_tokens,
        }
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, rhs: Usage) {
        *self = self.clone() + rhs;
    }
}

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

#[derive(Debug, Clone)]
pub enum LlmResponse {
    Message { text: String, usage: Usage },
    ToolCalls { calls: Vec<ToolCall>, preamble: String, usage: Usage },
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    ThinkingDelta { text: String },
    TextDelta { text: String },
    ToolCallReady(ToolCall),
    Done { stop_reason: String, usage: Usage },
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

#[derive(Debug, Clone, Default)]
pub struct ProviderMeta {
    pub id: String,
    pub expose_to_ui: bool,
    pub name: Option<String>,
    pub models: Option<Vec<String>>,
    pub user_provided_key: bool,
}

impl ProviderMeta {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into(), expose_to_ui: true, name: None, models: None, user_provided_key: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_default_is_zero() {
        let u = Usage::default();
        assert_eq!(u.total_tokens(), 0);
    }

    #[test]
    fn usage_total_sums_all_buckets() {
        let u = Usage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 30,
            cache_creation_tokens: 20,
            reasoning_tokens: 10,
        };
        assert_eq!(u.total_tokens(), 210);
    }

    #[test]
    fn usage_add_sums_each_field() {
        let a = Usage { input_tokens: 10, output_tokens: 5, cache_read_tokens: 2, cache_creation_tokens: 1, reasoning_tokens: 0 };
        let b = Usage { input_tokens: 20, output_tokens: 15, cache_read_tokens: 8, cache_creation_tokens: 4, reasoning_tokens: 3 };
        let c = a + b;
        assert_eq!(c, Usage { input_tokens: 30, output_tokens: 20, cache_read_tokens: 10, cache_creation_tokens: 5, reasoning_tokens: 3 });
    }

    #[test]
    fn usage_add_assign_accumulates() {
        let mut u = Usage { input_tokens: 10, output_tokens: 5, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        u += Usage { input_tokens: 5, output_tokens: 5, cache_read_tokens: 0, cache_creation_tokens: 0, reasoning_tokens: 0 };
        assert_eq!(u.input_tokens, 15);
        assert_eq!(u.output_tokens, 10);
    }
}
