use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::graph::model::{FunctionNode, ParsedFile};

const CHUNK_THRESHOLD_LINES: usize = 120;
const CHUNK_LINES: usize = 60;
const MAX_UNIT_BYTES: usize = 256 * 1024;
const MAX_SECTION_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Owner {
    pub name: String,
    pub impl_type: Option<String>,
    pub start_line: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitNode {
    pub kind: String,
    pub name: String,
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
    #[serde(default)]
    pub owner: Option<Owner>,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub registers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceNode {
    pub from: Owner,
    pub target: String,
    pub relation: String,
    pub line: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Markdown,
    Restructured,
    Ini,
    Yaml,
    Toml,
    Plain,
}

fn format_of(path: &Path) -> Option<(Format, &'static str)> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    Some(match (name.as_str(), extension.as_deref()) {
        (_, Some("md" | "markdown")) => (Format::Markdown, "doc"),
        (_, Some("rst")) => (Format::Restructured, "doc"),
        ("readme" | "readme.txt" | "changelog" | "contributing", _) => (Format::Plain, "doc"),
        (_, Some("yaml" | "yml")) => (Format::Yaml, "config"),
        (_, Some("toml")) => (Format::Toml, "config"),
        (_, Some("ini" | "cfg" | "conf")) => (Format::Ini, "config"),
        (_, Some("tf" | "hcl" | "sql" | "proto" | "graphql")) => (Format::Plain, "config"),
        (_, Some("json")) => (Format::Plain, "config"),
        ("dockerfile" | "makefile" | "jenkinsfile" | "tox.ini", _) => (Format::Plain, "config"),
        _ => return None,
    })
}

pub fn is_unit_file(path: &Path) -> bool {
    format_of(path).is_some()
}

fn backticked(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let inner = after[..end].trim().trim_end_matches("()");
        let identifier = inner
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':'))
            && inner.chars().any(char::is_alphabetic)
            && inner.len() >= 3;
        if identifier && !found.contains(&inner.to_string()) {
            found.push(inner.to_string());
        }
        rest = &after[end + 1..];
    }
    found
}

fn heading(line: &str, next: Option<&str>, format: Format) -> Option<String> {
    match format {
        Format::Markdown => line
            .strip_prefix('#')
            .map(|h| h.trim_start_matches('#').trim().to_string()),
        Format::Restructured => next
            .filter(|n| {
                let n = n.trim();
                n.len() >= 3 && n.chars().all(|c| matches!(c, '=' | '-' | '~' | '^' | '*'))
            })
            .filter(|_| !line.trim().is_empty())
            .map(|_| line.trim().to_string()),
        Format::Ini | Format::Toml => {
            let t = line.trim();
            (t.starts_with('[') && t.ends_with(']')).then(|| t.trim_matches(['[', ']']).to_string())
        }
        Format::Yaml => {
            let top_level = !line.starts_with([' ', '\t', '-', '#']) && line.contains(':');
            top_level.then(|| line.split(':').next().unwrap_or("").trim().to_string())
        }
        Format::Plain => None,
    }
}

fn entry_points(section: &str, lines: &[&str]) -> Vec<String> {
    let in_entry_points = section.starts_with("entry_points")
        || section.starts_with("project.entry-points")
        || section.starts_with("options.entry_points")
        || section.starts_with("tool.poetry.plugins");
    if !in_entry_points {
        return Vec::new();
    }
    lines
        .iter()
        .filter_map(|l| l.split_once('=').map(|(_, target)| target))
        .filter_map(|target| target.trim().trim_matches(['"', '\'']).rsplit_once(':'))
        .map(|(_, class)| class.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

pub fn file_units(path: &Path, source: &str) -> Vec<UnitNode> {
    let Some((format, kind)) = format_of(path) else {
        return Vec::new();
    };
    if source.len() > MAX_UNIT_BYTES || source.contains('\0') {
        return Vec::new();
    }
    let file = path.to_string_lossy().replace('\\', "/");
    let lines: Vec<&str> = source.lines().collect();
    let mut starts: Vec<(usize, String)> = vec![(0, file.clone())];
    for (i, line) in lines.iter().enumerate() {
        if let Some(title) = heading(line, lines.get(i + 1).copied(), format) {
            if i == 0 {
                starts[0].1 = title;
            } else {
                starts.push((i, title));
            }
        }
    }
    let mut units = Vec::new();
    for (n, (start, title)) in starts.iter().enumerate() {
        let end = starts.get(n + 1).map_or(lines.len(), |(s, _)| *s);
        let mut chunk_start = *start;
        while chunk_start < end {
            let chunk_end = (chunk_start + MAX_SECTION_LINES).min(end);
            let body = &lines[chunk_start..chunk_end];
            let text = body.join("\n");
            if !text.trim().is_empty() {
                units.push(UnitNode {
                    kind: kind.into(),
                    name: if title.is_empty() {
                        file.clone()
                    } else {
                        title.clone()
                    },
                    start_line: chunk_start as u32 + 1,
                    end_line: chunk_end as u32,
                    mentions: if kind == "doc" {
                        backticked(&text)
                    } else {
                        Vec::new()
                    },
                    registers: entry_points(title, body),
                    text,
                    owner: None,
                });
            }
            chunk_start = chunk_end;
        }
    }
    units
}

fn module_skeleton(parsed: &ParsedFile) -> Option<UnitNode> {
    if parsed.functions.is_empty() && parsed.classes.is_empty() {
        return None;
    }
    let mut lines = vec![format!("{} ({})", parsed.path, parsed.language)];
    for class in &parsed.classes {
        let doc = class
            .docstring
            .as_deref()
            .and_then(|d| d.lines().next())
            .unwrap_or("");
        lines.push(format!(
            "{} {}({}) {doc}",
            class.kind,
            class.name,
            class.bases.join(", ")
        ));
        for method in parsed
            .functions
            .iter()
            .filter(|f| f.impl_type.as_deref() == Some(class.name.as_str()))
        {
            lines.push(format!("    {}", method.signature));
        }
    }
    for function in parsed.functions.iter().filter(|f| f.impl_type.is_none()) {
        let doc = function
            .docstring
            .as_deref()
            .and_then(|d| d.lines().next())
            .unwrap_or("");
        lines.push(format!("{} {doc}", function.signature));
    }
    let end = parsed
        .functions
        .iter()
        .map(|f| f.end_line)
        .chain(parsed.classes.iter().map(|c| c.end_line))
        .max()
        .unwrap_or(1);
    Some(UnitNode {
        kind: "module".into(),
        name: parsed.path.clone(),
        start_line: 1,
        end_line: end,
        text: lines.join("\n"),
        owner: None,
        mentions: Vec::new(),
        registers: Vec::new(),
    })
}

fn chunk_boundaries(lines: &[&str]) -> Vec<usize> {
    let body_indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut boundaries = vec![1];
    let mut last = 1;
    for (i, line) in lines.iter().enumerate().skip(1) {
        let indent = line.len() - line.trim_start().len();
        let statement_start = !line.trim().is_empty() && indent == body_indent;
        if statement_start && i - last >= CHUNK_LINES {
            boundaries.push(i);
            last = i;
        }
    }
    if boundaries.len() > 1 && lines.len() - last < CHUNK_LINES / 2 {
        boundaries.pop();
    }
    boundaries
}

pub fn chunks(function: &FunctionNode) -> Vec<UnitNode> {
    let lines: Vec<&str> = function.source.lines().collect();
    if lines.len() < CHUNK_THRESHOLD_LINES {
        return Vec::new();
    }
    let boundaries = chunk_boundaries(&lines);
    let qualified = match &function.impl_type {
        Some(owner) => format!("{owner}.{}", function.name),
        None => function.name.clone(),
    };
    boundaries
        .iter()
        .enumerate()
        .map(|(n, start)| {
            let end = boundaries.get(n + 1).copied().unwrap_or(lines.len());
            UnitNode {
                kind: "chunk".into(),
                name: format!("{qualified} (part {})", n + 1),
                start_line: function.start_line + *start as u32,
                end_line: function.start_line + end as u32 - 1,
                text: format!(
                    "{}\n    ...\n{}",
                    function.signature,
                    lines[*start..end].join("\n")
                ),
                owner: Some(Owner {
                    name: function.name.clone(),
                    impl_type: function.impl_type.clone(),
                    start_line: function.start_line,
                }),
                mentions: Vec::new(),
                registers: Vec::new(),
            }
        })
        .collect()
}

pub fn add_code_units(parsed: &mut ParsedFile) {
    let mut units: Vec<UnitNode> = module_skeleton(parsed).into_iter().collect();
    for function in &parsed.functions {
        units.extend(chunks(function));
    }
    parsed.units.extend(units);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::model::ClassNode;

    #[test]
    fn markdown_splits_by_heading_and_records_mentions() {
        let units = file_units(
            Path::new("doc/drivers.md"),
            "# Drivers\nIntro\n## Multiattach\nSet `supports_multiattach` on `VolumeDriver`.\n",
        );
        assert_eq!(units.len(), 2);
        assert_eq!(units[0].name, "Drivers");
        assert_eq!(units[1].name, "Multiattach");
        assert_eq!(
            units[1].mentions,
            vec!["supports_multiattach", "VolumeDriver"]
        );
        assert_eq!((units[1].start_line, units[1].end_line), (3, 4));
    }

    #[test]
    fn config_sections_and_entry_points_are_units() {
        let units = file_units(
            Path::new("setup.cfg"),
            "[metadata]\nname = cinder\n[entry_points]\ncinder.volume.drivers =\n    lvm = cinder.volume.drivers.lvm:LVMVolumeDriver\n",
        );
        assert_eq!(units.len(), 2);
        assert_eq!(units[1].kind, "config");
        assert_eq!(units[1].registers, vec!["LVMVolumeDriver"]);
        let yaml = file_units(
            Path::new("charm/config.yaml"),
            "options:\n  retry_limit:\n    default: 3\nactions:\n  x: 1\n",
        );
        assert_eq!(
            yaml.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
            vec!["options", "actions"]
        );
        assert!(file_units(Path::new("src/main.rs"), "fn main() {}").is_empty());
        assert!(is_unit_file(Path::new("Dockerfile")));
    }

    fn function(name: &str, lines: usize) -> FunctionNode {
        let mut source = format!("def {name}(self):\n");
        for i in 0..lines {
            source.push_str(&format!("    step_{i}()\n"));
        }
        FunctionNode {
            repo: "r".into(),
            version: "v".into(),
            file: "a.py".into(),
            name: name.into(),
            kind: "method".into(),
            signature: format!("def {name}(self):"),
            start_line: 10,
            end_line: 10 + lines as u32,
            source,
            impl_type: Some("Manager".into()),
            docstring: None,
            calls: vec![],
        }
    }

    #[test]
    fn long_functions_are_chunked_at_statement_boundaries_with_their_signature() {
        let long = function("run", 200);
        let parts = chunks(&long);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].text.starts_with("def run(self):"));
        assert_eq!(parts[0].start_line, 11);
        assert_eq!(parts[1].start_line, parts[0].end_line + 1);
        assert_eq!(
            parts[0].owner.as_ref().unwrap().impl_type.as_deref(),
            Some("Manager")
        );
        assert!(chunks(&function("short", 20)).is_empty());
    }

    #[test]
    fn code_files_get_a_skeleton_unit() {
        let mut parsed = ParsedFile {
            path: "a.py".into(),
            language: "python".into(),
            functions: vec![function("run", 3)],
            classes: vec![ClassNode {
                repo: "r".into(),
                version: "v".into(),
                file: "a.py".into(),
                name: "Manager".into(),
                kind: "class".into(),
                start_line: 1,
                end_line: 20,
                source: String::new(),
                bases: vec!["Base".into()],
                traits: vec![],
                embeds: vec![],
                uses: vec![],
                docstring: Some("Manages things.".into()),
                decorators: vec![],
            }],
            ..ParsedFile::default()
        };
        add_code_units(&mut parsed);
        let skeleton = &parsed.units[0];
        assert_eq!(skeleton.kind, "module");
        assert!(skeleton
            .text
            .contains("class Manager(Base) Manages things."));
        assert!(skeleton.text.contains("    def run(self):"));
    }
}
