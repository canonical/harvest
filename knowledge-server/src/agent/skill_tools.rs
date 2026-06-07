use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use crate::skills::SkillRegistry;
use super::tool::{self, Tool};

pub struct ListSkillsTool {
    pub registry: Arc<SkillRegistry>,
}

pub struct LoadSkillTool {
    pub registry: Arc<SkillRegistry>,
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
        let summaries: Vec<Value> = self.registry.list().into_iter().map(|s| json!({
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
        self.registry
            .load(name)
            .map(|body| body.to_string())
            .ok_or_else(|| anyhow::anyhow!("unknown skill '{name}'"))
    }

    fn preview(&self, result: &str) -> String {
        let truncated: String = result.chars().take(tool::DEFAULT_PREVIEW_CHARS * 4).collect();
        serde_json::to_string(&json!({ "__type": "markdown", "content": truncated }))
            .unwrap_or_else(|_| result.chars().take(tool::DEFAULT_PREVIEW_CHARS).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<SkillRegistry> {
        Arc::new(SkillRegistry::new())
    }

    // ── definition names ─────────────────────────────────────────────────

    #[test]
    fn list_skills_definition_has_correct_name() {
        assert_eq!(ListSkillsTool { registry: registry() }.definition().name, "list_skills");
    }

    #[test]
    fn load_skill_definition_has_correct_name() {
        assert_eq!(LoadSkillTool { registry: registry() }.definition().name, "load_skill");
    }

    // ── list_skills execute ───────────────────────────────────────────────

    #[tokio::test]
    async fn list_skills_returns_json_array() {
        let result = ListSkillsTool { registry: registry() }.execute(json!({})).await.unwrap();
        let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert!(!arr.is_empty());
    }

    #[tokio::test]
    async fn list_skills_each_item_has_name_and_description() {
        let result = ListSkillsTool { registry: registry() }.execute(json!({})).await.unwrap();
        let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
        for item in &arr {
            assert!(item["name"].is_string(),        "item missing name: {item}");
            assert!(item["description"].is_string(), "item missing description: {item}");
        }
    }

    #[tokio::test]
    async fn list_skills_contains_all_five_skills() {
        let result = ListSkillsTool { registry: registry() }.execute(json!({})).await.unwrap();
        let arr: Vec<Value> = serde_json::from_str(&result).unwrap();
        let names: Vec<&str> = arr.iter().filter_map(|v| v["name"].as_str()).collect();
        assert!(names.contains(&"juju"),          "missing juju");
        assert!(names.contains(&"lxd"),           "missing lxd");
        assert!(names.contains(&"ceph"),          "missing ceph");
        assert!(names.contains(&"canonical-k8s"), "missing canonical-k8s");
        assert!(names.contains(&"landscape"),     "missing landscape");
    }

    // ── load_skill execute ────────────────────────────────────────────────

    #[tokio::test]
    async fn load_skill_returns_body_for_known_skill() {
        let result = LoadSkillTool { registry: registry() }
            .execute(json!({ "name": "juju" })).await.unwrap();
        assert!(!result.is_empty());
        assert!(!result.contains("name: juju"), "frontmatter must not appear in output");
    }

    #[tokio::test]
    async fn load_skill_missing_name_param_returns_error() {
        let result = LoadSkillTool { registry: registry() }.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn load_skill_unknown_name_returns_error() {
        let result = LoadSkillTool { registry: registry() }
            .execute(json!({ "name": "nonexistent" })).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    #[test]
    fn load_skill_preview_is_markdown_envelope() {
        let tool = LoadSkillTool { registry: registry() };
        let preview = tool.preview("# Heading\nsome text");
        let parsed: Value = serde_json::from_str(&preview).expect("preview must be valid JSON");
        assert_eq!(parsed["__type"].as_str(), Some("markdown"), "__type must be 'markdown'");
        assert!(parsed["content"].as_str().map(|s| s.contains("# Heading")).unwrap_or(false));
    }

    #[tokio::test]
    async fn load_skill_works_for_all_five_skills() {
        let tool = LoadSkillTool { registry: registry() };
        for name in &["juju", "lxd", "ceph", "canonical-k8s", "landscape"] {
            let result = tool.execute(json!({ "name": name })).await;
            assert!(result.is_ok(), "failed to load skill '{name}'");
            assert!(!result.unwrap().is_empty(), "skill '{name}' body is empty");
        }
    }
}
