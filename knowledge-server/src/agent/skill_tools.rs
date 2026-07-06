use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use crate::skills::SkillStore;
use super::tool::{self, Tool};

pub struct ListSkillsTool {
    pub store:      Arc<SkillStore>,
    pub project_id: String,
}

pub struct LoadSkillTool {
    pub store:      Arc<SkillStore>,
    pub project_id: String,
}

#[async_trait]
impl Tool for ListSkillsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_skills".into(),
            description: "List available skill guides by name and description. \
                          Call this when a task may involve infrastructure technologies \
                          (e.g. Juju, LXD, Ceph, Kubernetes). \
                          Then call load_skill to retrieve the full guide for a relevant skill."
                .into(),
            parameters: json!({
                "type":       "object",
                "properties": {},
                "required":   []
            }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let summaries: Vec<Value> = self.store.list_for_project(&self.project_id).await
            .into_iter().map(|s| json!({
                "name":        s.name,
                "description": s.description,
            })).collect();
        Ok(serde_json::to_string_pretty(&summaries)?)
    }
}

#[async_trait]
impl Tool for LoadSkillTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "load_skill".into(),
            description: "Load the full content of a named skill guide. \
                          Use list_skills first to discover available skill names."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type":        "string",
                        "description": "The skill name (from list_skills)"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let name = params["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("name is required"))?;
        self.store
            .load_content(name, &self.project_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("unknown skill '{name}'"))
    }

    fn preview(&self, result: &str) -> String {
        let truncated: String = result.chars().take(tool::DEFAULT_PREVIEW_CHARS * 4).collect();
        serde_json::to_string(&json!({ "__type": "markdown", "content": truncated }))
            .unwrap_or_else(|_| result.chars().take(tool::DEFAULT_PREVIEW_CHARS).collect())
    }
}
