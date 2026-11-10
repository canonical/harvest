use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::llm::types::ToolDefinition;

pub const DEFAULT_PREVIEW_CHARS: usize = 3000;

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, params: Value) -> Result<String>;
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
    async fn prefetch(&self, _question: &str, _earlier: &str) -> Option<(Value, String)> {
        None
    }
}
