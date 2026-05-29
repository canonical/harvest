use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::llm::types::ToolDefinition;

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, params: Value) -> Result<String>;
    /// Build the preview string sent to the UI after a tool result.
    /// The default truncates to 3000 chars; source-returning tools override
    /// this to send the full result so the UI can display scrollable code.
    fn preview(&self, result: &str) -> String {
        result.chars().take(3000).collect()
    }
}
