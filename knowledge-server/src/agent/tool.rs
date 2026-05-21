use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::llm::types::ToolDefinition;

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, params: Value) -> Result<String>;
}
