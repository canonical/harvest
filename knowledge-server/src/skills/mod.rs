use std::collections::HashMap;

const JUJU_MD:          &str = include_str!("../../skills/juju.md");
const LXD_MD:           &str = include_str!("../../skills/lxd.md");
const CEPH_MD:          &str = include_str!("../../skills/ceph.md");
const CANONICAL_K8S_MD: &str = include_str!("../../skills/canonical-k8s.md");
const LANDSCAPE_MD:     &str = include_str!("../../skills/landscape.md");

#[derive(Debug, Clone)]
pub struct SkillSummary {
    pub name:        String,
    pub description: String,
}

#[derive(Debug, Clone)]
struct Skill {
    pub name:        String,
    pub description: String,
    pub body:        String,
}

pub struct SkillRegistry {
    skills: Vec<Skill>,
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

fn load_skill(raw: &str) -> Skill {
    let fm   = parse_frontmatter(raw);
    let name = fm.get("name").cloned().unwrap_or_default();
    let desc = fm.get("description").cloned().unwrap_or_default();
    let body = skill_body(raw).to_string();
    Skill { name, description: desc, body }
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: vec![
                load_skill(JUJU_MD),
                load_skill(LXD_MD),
                load_skill(CEPH_MD),
                load_skill(CANONICAL_K8S_MD),
                load_skill(LANDSCAPE_MD),
            ],
        }
    }

    pub fn list(&self) -> Vec<SkillSummary> {
        self.skills.iter().map(|s| SkillSummary {
            name:        s.name.clone(),
            description: s.description.clone(),
        }).collect()
    }

    pub fn load(&self, name: &str) -> Option<&str> {
        self.skills.iter().find(|s| s.name == name).map(|s| s.body.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_frontmatter ────────────────────────────────────────────────

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

    // ── skill_body ───────────────────────────────────────────────────────

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

    // ── SkillRegistry ────────────────────────────────────────────────────

    #[test]
    fn registry_list_returns_all_five_skills() {
        let registry = SkillRegistry::new();
        assert_eq!(registry.list().len(), 5);
    }

    #[test]
    fn registry_list_contains_expected_names() {
        let registry = SkillRegistry::new();
        let list = registry.list();
        let names: Vec<&str> = list.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"juju"),          "missing juju");
        assert!(names.contains(&"lxd"),           "missing lxd");
        assert!(names.contains(&"ceph"),          "missing ceph");
        assert!(names.contains(&"canonical-k8s"), "missing canonical-k8s");
        assert!(names.contains(&"landscape"),     "missing landscape");
    }

    #[test]
    fn registry_list_all_descriptions_non_empty() {
        let registry = SkillRegistry::new();
        for s in registry.list() {
            assert!(!s.description.is_empty(), "{} has empty description", s.name);
        }
    }

    #[test]
    fn registry_load_known_skill_returns_some() {
        let registry = SkillRegistry::new();
        assert!(registry.load("juju").is_some());
        assert!(registry.load("lxd").is_some());
        assert!(registry.load("ceph").is_some());
        assert!(registry.load("canonical-k8s").is_some());
    }

    #[test]
    fn registry_load_returns_body_without_frontmatter() {
        let registry = SkillRegistry::new();
        let body = registry.load("juju").unwrap();
        assert!(!body.contains("name: juju"),  "frontmatter leaked into body");
        assert!(!body.starts_with("---"),       "body starts with frontmatter delimiter");
    }

    #[test]
    fn registry_load_unknown_skill_returns_none() {
        let registry = SkillRegistry::new();
        assert!(registry.load("nonexistent").is_none());
    }
}
