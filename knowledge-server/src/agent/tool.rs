use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::llm::types::ToolDefinition;

pub const DEFAULT_PREVIEW_CHARS: usize = 3000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolOutput {
    pub text: String,
    pub view: Option<Value>,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            view: None,
        }
    }

    pub fn with_view(text: impl Into<String>, view: Value) -> Self {
        Self {
            text: text.into(),
            view: Some(view),
        }
    }

    pub fn is_error(&self) -> bool {
        self.text.starts_with("error:")
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, params: Value) -> Result<String>;
    async fn run(&self, params: Value) -> Result<ToolOutput> {
        Ok(ToolOutput::text(self.execute(params).await?))
    }
    fn preview(&self, result: &str) -> String {
        result.chars().take(DEFAULT_PREVIEW_CHARS).collect()
    }
    fn requires_confirmation(&self) -> bool {
        false
    }
    fn reads_code(&self) -> bool {
        false
    }
    fn read_budget(&self) -> Option<usize> {
        None
    }
    async fn prefetch(&self, _question: &str, _earlier: &str) -> Option<(Value, ToolOutput)> {
        None
    }
}
