use std::collections::BTreeMap;
use std::io::Read;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::skills;

#[derive(Debug)]
pub struct ParsedSkill {
    pub name:        String,
    pub description: String,
    pub content:     String,
}

#[derive(Debug)]
pub struct ParsedArtifact {
    pub name:    String,
    pub kind:    String,
    pub content: String,
}

#[derive(Debug)]
pub struct ParsedHarvest {
    pub name:            String,
    pub description:     String,
    pub design_template: String,
    pub skills:           Vec<ParsedSkill>,
    pub artifacts:        Vec<ParsedArtifact>,
}

#[derive(Deserialize)]
struct Metadata {
    name:        String,
    description: String,
}

fn infer_kind(filename: &str) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    if lower.ends_with(".tf") || lower.ends_with(".tf.json")      { return Some("terraform"); }
    if lower.ends_with(".tg.hcl") || lower.ends_with(".tg")      { return Some("terragrunt"); }
    if lower.ends_with(".sh") || lower.ends_with(".bash")        { return Some("bash"); }
    if lower.ends_with(".md") || lower.ends_with(".markdown")    { return Some("markdown"); }
    None
}

fn artifact_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
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

fn entries_under(
    archive: &zip::ZipArchive<std::io::Cursor<&[u8]>>,
    prefix: &str,
) -> Vec<String> {
    let mut names: Vec<String> = archive.file_names()
        .filter(|n| n.starts_with(prefix) && !n.ends_with('/'))
        .map(|n| n.to_string())
        .collect();
    names.sort();
    names
}

pub fn parse_harvest_archive(bytes: &[u8]) -> anyhow::Result<ParsedHarvest> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| anyhow::anyhow!("invalid zip archive: {e}"))?;

    let metadata_text = read_zip_file(&mut archive, "metadata.yaml")
        .map_err(|e| anyhow::anyhow!("metadata.yaml is required: {e}"))?;
    let metadata: Metadata = serde_yaml::from_str(&metadata_text)
        .map_err(|e| anyhow::anyhow!("metadata.yaml is not valid: {e}"))?;

    let design_template = read_zip_file(&mut archive, "design.md")
        .map_err(|e| anyhow::anyhow!("design.md is required: {e}"))?;

    let skill_files = entries_under(&archive, "skills/")
        .into_iter()
        .filter(|f| f.to_lowercase().ends_with(".md"))
        .collect::<Vec<_>>();
    let mut skills = Vec::new();
    for file in skill_files {
        let md = read_zip_file(&mut archive, &file)?;
        let fm = skills::parse_frontmatter(&md);
        let name = fm.get("name").cloned()
            .ok_or_else(|| anyhow::anyhow!("skill '{file}' has no 'name' in frontmatter"))?;
        let description = fm.get("description").cloned().unwrap_or_default();
        let content = skills::skill_body(&md).to_string();
        skills.push(ParsedSkill { name, description, content });
    }

    let artifact_files = entries_under(&archive, "artifacts/");
    let mut artifacts = Vec::new();
    for file in artifact_files {
        let kind = infer_kind(&file)
            .ok_or_else(|| anyhow::anyhow!("unsupported artifact file extension: {file}"))?;
        let file_content = read_zip_file(&mut archive, &file)?;
        let content = if kind == "terraform" || kind == "terragrunt" {
            let mut bundle = BTreeMap::new();
            let path = std::path::Path::new(&file)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&file);
            bundle.insert(path.to_string(), file_content);
            serde_json::to_string(&bundle)?
        } else {
            file_content
        };
        artifacts.push(ParsedArtifact {
            name: artifact_name(&file),
            kind: kind.to_string(),
            content,
        });
    }

    Ok(ParsedHarvest {
        name:            metadata.name,
        description:     metadata.description,
        design_template,
        skills,
        artifacts,
    })
}

pub fn harvest_to_json(parsed: &ParsedHarvest) -> Value {
    json!({
        "design_template": parsed.design_template,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn zip_with(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, content) in entries {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    fn full_archive_with_extra(extra: &[(&str, &str)]) -> Vec<u8> {
        let mut entries = vec![
            ("metadata.yaml", "name: Charmed Landscape\ndescription: Fleet management"),
            ("design.md", "# 1. Introduction\n${CUSTOMER}"),
            ("skills/landscape.md", "---\nname: landscape\ndescription: Deploy Landscape\n---\n# Landscape\nBody."),
            ("artifacts/main.tf", "resource \"null_resource\" \"x\" {}"),
        ];
        entries.extend_from_slice(extra);
        zip_with(&entries)
    }

    #[test]
    fn parses_metadata_and_design_template() {
        let parsed = parse_harvest_archive(&full_archive_with_extra(&[])).unwrap();
        assert_eq!(parsed.name, "Charmed Landscape");
        assert_eq!(parsed.description, "Fleet management");
        assert_eq!(parsed.design_template, "# 1. Introduction\n${CUSTOMER}");
    }

    #[test]
    fn discovers_skills_and_artifacts_by_directory_listing() {
        let parsed = parse_harvest_archive(&full_archive_with_extra(&[])).unwrap();
        assert_eq!(parsed.skills.len(), 1);
        assert_eq!(parsed.skills[0].name, "landscape");
        assert_eq!(parsed.skills[0].description, "Deploy Landscape");
        assert_eq!(parsed.artifacts.len(), 1);
        assert_eq!(parsed.artifacts[0].name, "main");
        assert_eq!(parsed.artifacts[0].kind, "terraform");
    }

    #[test]
    fn artifact_name_is_derived_from_filename_stem() {
        let bytes = full_archive_with_extra(&[("artifacts/deploy.sh", "echo hi")]);
        let parsed = parse_harvest_archive(&bytes).unwrap();
        let names: Vec<&str> = parsed.artifacts.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"deploy"));
    }

    #[test]
    fn rejects_missing_metadata_yaml() {
        let bytes = zip_with(&[
            ("design.md", "# Intro"),
            ("skills/x.md", "---\nname: x\ndescription: y\n---\nbody"),
        ]);
        let err = parse_harvest_archive(&bytes).unwrap_err();
        assert!(err.to_string().contains("metadata.yaml"));
    }

    #[test]
    fn rejects_missing_design_md() {
        let bytes = zip_with(&[
            ("metadata.yaml", "name: x\ndescription: y"),
            ("skills/x.md", "---\nname: x\ndescription: y\n---\nbody"),
        ]);
        let err = parse_harvest_archive(&bytes).unwrap_err();
        assert!(err.to_string().contains("design.md"));
    }

    #[test]
    fn rejects_skill_without_name_in_frontmatter() {
        let bytes = full_archive_with_extra(&[("skills/broken.md", "no frontmatter here")]);
        let err = parse_harvest_archive(&bytes).unwrap_err();
        assert!(err.to_string().contains("frontmatter"));
    }

    #[test]
    fn harvest_to_json_round_trips_design_template() {
        let parsed = parse_harvest_archive(&full_archive_with_extra(&[])).unwrap();
        let value = harvest_to_json(&parsed);
        assert_eq!(value["design_template"], "# 1. Introduction\n${CUSTOMER}");
    }
}
