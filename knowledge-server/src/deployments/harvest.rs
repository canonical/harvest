use std::collections::BTreeMap;
use std::io::Read;

use serde_json::{json, Value};

use crate::skills;

pub struct ParsedSkill {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

pub struct ParsedArtifact {
    pub name:    String,
    pub kind:    String,
    pub content: String,
}

pub struct ParsedHarvest {
    pub skills:    Vec<ParsedSkill>,
    pub artifacts: Vec<ParsedArtifact>,
}

fn infer_kind(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".tf") || lower.ends_with(".tf.json")      { return Some("terraform"); }
    if lower.ends_with(".tg.hcl") || lower.ends_with(".tg")      { return Some("terragrunt"); }
    if lower.ends_with(".sh") || lower.ends_with(".bash")        { return Some("bash"); }
    if lower.ends_with(".md") || lower.ends_with(".markdown")    { return Some("markdown"); }
    None
}

fn read_zip_file(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> anyhow::Result<String> {
    let mut entry = archive.by_name(name)
        .map_err(|e| anyhow::anyhow!("file '{name}' not found in archive: {e}"))?;
    let mut buf = String::new();
    entry.read_to_string(&mut buf)?;
    Ok(buf)
}

pub fn parse_harvest_archive(bytes: &[u8]) -> anyhow::Result<ParsedHarvest> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow::anyhow!("invalid zip archive: {e}"))?;

    let index_text = read_zip_file(&mut archive, "index.json")
        .map_err(|e| anyhow::anyhow!("index.json is required: {e}"))?;
    let index: Value = serde_json::from_str(&index_text)
        .map_err(|e| anyhow::anyhow!("index.json is not valid JSON: {e}"))?;

    let mut skills = Vec::new();
    if let Some(skill_entries) = index["skills"].as_array() {
        for entry in skill_entries {
            let file = entry["file"].as_str()
                .ok_or_else(|| anyhow::anyhow!("each skill entry must have a 'file' field"))?;
            let md = read_zip_file(&mut archive, file)?;
            let fm = skills::parse_frontmatter(&md);
            let name = fm.get("name").cloned()
                .ok_or_else(|| anyhow::anyhow!("skill '{file}' has no 'name' in frontmatter"))?;
            let description = fm.get("description").cloned().unwrap_or_default();
            let content = skills::skill_body(&md).to_string();
            skills.push(ParsedSkill { name, description, content });
        }
    }

    let mut artifacts = Vec::new();
    if let Some(artifact_entries) = index["artifacts"].as_array() {
        for entry in artifact_entries {
            let name = entry["name"].as_str()
                .ok_or_else(|| anyhow::anyhow!("each artifact entry must have a 'name' field"))?;
            let file = entry["file"].as_str()
                .ok_or_else(|| anyhow::anyhow!("each artifact entry must have a 'file' field"))?;
            let kind = infer_kind(file)
                .ok_or_else(|| anyhow::anyhow!("unsupported artifact file extension: {file}"))?;
            let file_content = read_zip_file(&mut archive, file)?;
            let content = if kind == "terraform" || kind == "terragrunt" {
                let mut bundle = BTreeMap::new();
                let path = std::path::Path::new(file)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(file);
                bundle.insert(path.to_string(), file_content);
                serde_json::to_string(&bundle)?
            } else {
                file_content
            };
            artifacts.push(ParsedArtifact {
                name: name.to_string(),
                kind: kind.to_string(),
                content,
            });
        }
    }

    Ok(ParsedHarvest { skills, artifacts })
}

pub fn harvest_to_json(parsed: &ParsedHarvest) -> Value {
    json!({
        "skills": parsed.skills.iter().map(|s| json!({
            "name": s.name,
            "description": s.description,
            "content": s.content,
        })).collect::<Vec<_>>(),
        "artifacts": parsed.artifacts.iter().map(|a| json!({
            "name": a.name,
            "kind": a.kind,
            "content": a.content,
        })).collect::<Vec<_>>(),
    })
}

pub fn derive_template_name(parsed: &ParsedHarvest) -> String {
    if let Some(first) = parsed.skills.first() {
        return first.name.clone();
    }
    if let Some(first) = parsed.artifacts.first() {
        return first.name.clone();
    }
    "Untitled template".to_string()
}