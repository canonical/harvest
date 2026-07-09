pub mod handlers;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use uuid::Uuid;

use crate::neo4j::Neo4jClient;

const JUJU_MD:          &str = include_str!("../../skills/juju.md");
const LXD_MD:           &str = include_str!("../../skills/lxd.md");
const CEPH_MD:          &str = include_str!("../../skills/ceph.md");
const CANONICAL_K8S_MD: &str = include_str!("../../skills/canonical-k8s.md");
const LANDSCAPE_MD:     &str = include_str!("../../skills/landscape.md");
const OPENSTACK_MD:     &str = include_str!("../../skills/openstack.md");

#[derive(Debug, Clone)]
pub struct SkillSummary {
    pub name:        String,
    pub description: String,
}

pub fn parse_frontmatter(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(after_open) = content.strip_prefix("---\n") else {
        return map;
    };
    let Some(close_pos) = after_open.find("\n---\n").or_else(|| {
        after_open.strip_suffix("\n---").map(|s| s.len())
    }) else {
        return map;
    };
    for line in after_open[..close_pos].lines() {
        if let Some((k, v)) = line.split_once(':') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

pub fn skill_body(content: &str) -> &str {
    let Some(after_open) = content.strip_prefix("---\n") else {
        return content;
    };
    let Some(close_pos) = after_open.find("\n---\n") else {
        return content;
    };
    after_open[close_pos + 5..].trim_start()
}

pub struct SkillStore {
    pub neo4j: Arc<Neo4jClient>,
}

impl SkillStore {
    pub fn new(neo4j: Arc<Neo4jClient>) -> Self {
        Self { neo4j }
    }

    pub async fn setup_constraints(&self) -> Result<()> {
        self.neo4j
            .run("CREATE CONSTRAINT skill_id IF NOT EXISTS FOR (s:Skill) REQUIRE s.id IS UNIQUE")
            .await
    }

    pub async fn list_for_project(&self, project_id: &str) -> Vec<SkillSummary> {
        let rows = self.neo4j.query_read(
            "MATCH (s:Skill)
             WHERE s.is_global = true
                OR EXISTS { MATCH (:Project {id: $pid})-[:HAS_SKILL]->(s) }
             RETURN s.name AS name, s.description AS description
             ORDER BY s.is_global DESC, s.name ASC",
            json!({ "pid": project_id }),
        ).await.unwrap_or_default();

        rows.into_iter().map(|r| SkillSummary {
            name:        r["name"].as_str().unwrap_or_default().to_string(),
            description: r["description"].as_str().unwrap_or_default().to_string(),
        }).collect()
    }

    pub async fn load_content(&self, name: &str, project_id: &str) -> Option<String> {
        let rows = self.neo4j.query_read(
            "MATCH (s:Skill {name: $name})
             WHERE s.is_global = true
                OR EXISTS { MATCH (:Project {id: $pid})-[:HAS_SKILL]->(s) }
             RETURN s.content AS content
             LIMIT 1",
            json!({ "name": name, "pid": project_id }),
        ).await.unwrap_or_default();

        rows.into_iter().next().and_then(|r| r["content"].as_str().map(|s| s.to_string()))
    }
}

pub async fn seed_defaults_if_needed(neo4j: &Neo4jClient) -> Result<()> {
    let marker = neo4j.query_read(
        "MATCH (m:SkillsSeeded) RETURN m.seeded_at AS seeded_at LIMIT 1",
        json!({}),
    ).await?;
    if !marker.is_empty() {
        return Ok(());
    }

    let now = chrono::Utc::now().to_rfc3339();
    for raw in [JUJU_MD, LXD_MD, CEPH_MD, CANONICAL_K8S_MD, LANDSCAPE_MD, OPENSTACK_MD] {
        let fm          = parse_frontmatter(raw);
        let name        = fm.get("name").cloned().unwrap_or_default();
        let description = fm.get("description").cloned().unwrap_or_default();
        let content     = skill_body(raw).to_string();
        let id          = Uuid::new_v4().to_string();

        neo4j.query_read(
            "MERGE (s:Skill {name: $name})
             ON CREATE SET s.id = $id, s.description = $description, s.content = $content,
                           s.is_global = true, s.created_by = 'system',
                           s.created_at = $now, s.updated_at = $now",
            json!({
                "id": id, "name": name, "description": description,
                "content": content, "now": now,
            }),
        ).await?;
    }

    neo4j.query_read(
        "CREATE (:SkillsSeeded {seeded_at: $now})",
        json!({ "now": now }),
    ).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_extracts_name_and_description() {
        let content = "---\nname: juju\ndescription: Juju guide\n---\n\nBody here";
        let fm = parse_frontmatter(content);
        assert_eq!(fm.get("name").map(String::as_str), Some("juju"));
        assert_eq!(fm.get("description").map(String::as_str), Some("Juju guide"));
    }

    #[test]
    fn parse_frontmatter_no_frontmatter_returns_empty() {
        let fm = parse_frontmatter("No frontmatter here");
        assert!(fm.is_empty());
    }

    #[test]
    fn parse_frontmatter_ignores_body_content() {
        let content = "---\nname: x\n---\nname: should_not_appear";
        let fm = parse_frontmatter(content);
        assert_eq!(fm.get("name").map(String::as_str), Some("x"));
        assert_eq!(fm.len(), 1);
    }

    #[test]
    fn parse_frontmatter_trims_values() {
        let content = "---\nname:  spaced  \n---\n";
        let fm = parse_frontmatter(content);
        assert_eq!(fm.get("name").map(String::as_str), Some("spaced"));
    }

    #[test]
    fn skill_body_strips_frontmatter() {
        let content = "---\nname: test\n---\n\n# Heading\nBody text";
        let body = skill_body(content);
        assert!(!body.contains("name: test"));
        assert!(body.contains("# Heading"));
    }

    #[test]
    fn skill_body_no_frontmatter_returns_whole_content() {
        let content = "# Plain\nNo frontmatter";
        assert_eq!(skill_body(content), content);
    }

    #[test]
    fn skill_body_trims_leading_blank_lines_after_delimiter() {
        let content = "---\nname: x\n---\n\n\nActual body";
        let body = skill_body(content);
        assert!(body.starts_with("Actual body"));
    }
}
