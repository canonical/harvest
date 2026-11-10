use std::collections::HashMap;

use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::gold::{GoldEvidence, GoldQuery};

const MESSAGES_SQL: &str = r#"
SELECT conv_id, seq, payload
FROM conversation_messages
ORDER BY conv_id, seq
"#;

const RESOLVE_SQL: &str = r#"
SELECT file, name, impl_type, coalesce(start_line, 0) AS start_line, coalesce(end_line, 0) AS end_line
FROM code_symbols
WHERE repo = $repo AND version = $version
  AND (name = $name OR (impl_type || '.' || name) = $name)
ORDER BY (label = 'Class') DESC, id
LIMIT 3
"#;

const READ_TOOLS: &[&str] = &[
    "read_sources",
    "get_symbol_source",
    "get_evidence_pack",
    "get_capability_matrix",
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Read {
    pub repo: String,
    pub version: String,
    pub names: Vec<String>,
    pub files: Vec<String>,
}

fn strings(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .flat_map(|k| match &value[*k] {
            Value::String(s) => vec![s.clone()],
            Value::Array(items) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        })
        .filter(|s| !s.trim().is_empty())
        .collect()
}

pub fn reads(chain: &Value) -> Vec<Read> {
    chain
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter(|e| e["type"] == "tool_call")
                .filter(|e| READ_TOOLS.contains(&e["name"].as_str().unwrap_or_default()))
                .map(|e| {
                    let input = &e["input"];
                    Read {
                        repo: input["repo"].as_str().unwrap_or_default().to_string(),
                        version: input["version"].as_str().unwrap_or_default().to_string(),
                        names: strings(
                            input,
                            &["names", "name", "symbols", "classes", "class_names"],
                        ),
                        files: strings(input, &["files", "file"]),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn cited(message: &Value) -> Vec<(String, String, GoldEvidence)> {
    message["sources"]
        .as_array()
        .map(|sources| {
            sources
                .iter()
                .filter_map(|s| {
                    let repo = s["repo"].as_str()?.to_string();
                    let version = s["version"].as_str()?.to_string();
                    let file = s["file"].as_str()?;
                    let line = s["line"].as_u64().unwrap_or(0) as u32;
                    let end = s["end_line"].as_u64().map(|e| e as u32).unwrap_or(line);
                    let gold = if line == 0 {
                        GoldEvidence::file(file, true)
                    } else {
                        GoldEvidence::span(file, (line, end), true)
                    };
                    Some((repo, version, gold))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn dominant(scopes: impl Iterator<Item = (String, String)>) -> Option<(String, String)> {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for scope in scopes.filter(|(r, v)| !r.is_empty() && !v.is_empty()) {
        *counts.entry(scope).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .map(|(scope, _)| scope)
}

async fn resolve(db: &Db, repo: &str, version: &str, name: &str) -> Result<Vec<GoldEvidence>> {
    Ok(db
        .query(
            RESOLVE_SQL,
            json!({ "repo": repo, "version": version, "name": name }),
        )
        .await?
        .iter()
        .map(|row| {
            let name = row["name"].as_str().unwrap_or_default();
            let qualified = match row["impl_type"].as_str().filter(|p| !p.is_empty()) {
                Some(parent) => format!("{parent}.{name}"),
                None => name.to_string(),
            };
            GoldEvidence::symbol(
                row["file"].as_str().unwrap_or_default(),
                &qualified,
                (
                    row["start_line"].as_i64().unwrap_or(0) as u32,
                    row["end_line"].as_i64().unwrap_or(0) as u32,
                ),
                false,
            )
        })
        .collect())
}

pub async fn extract(db: &Db) -> Result<Vec<GoldQuery>> {
    let rows = db.query(MESSAGES_SQL, json!({})).await?;
    let mut queries = Vec::new();
    let mut question: Option<(String, String)> = None;
    for row in rows {
        let conv = row["conv_id"].as_str().unwrap_or_default().to_string();
        let payload = &row["payload"];
        match payload["role"].as_str() {
            Some("user") => {
                question = payload["text"]
                    .as_str()
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| (conv.clone(), t.to_string()));
            }
            Some("assistant") => {
                let Some((asked_in, text)) = question.take() else {
                    continue;
                };
                if asked_in != conv {
                    continue;
                }
                let citations = cited(payload);
                let reads = reads(&payload["chain"]);
                let Some((repo, version)) = dominant(
                    citations
                        .iter()
                        .map(|(r, v, _)| (r.clone(), v.clone()))
                        .chain(reads.iter().map(|r| (r.repo.clone(), r.version.clone()))),
                ) else {
                    continue;
                };
                let mut evidence: Vec<GoldEvidence> = citations
                    .into_iter()
                    .filter(|(r, v, _)| *r == repo && *v == version)
                    .map(|(_, _, g)| g)
                    .collect();
                for read in reads
                    .iter()
                    .filter(|r| r.repo == repo && r.version == version)
                {
                    for name in &read.names {
                        for gold in resolve(db, &repo, &version, name).await? {
                            if !evidence
                                .iter()
                                .any(|e| e.file == gold.file && e.name == gold.name)
                            {
                                evidence.push(gold);
                            }
                        }
                    }
                    for file in &read.files {
                        if !evidence.iter().any(|e| &e.file == file) {
                            evidence.push(GoldEvidence::file(file, false));
                        }
                    }
                }
                if evidence.is_empty() {
                    continue;
                }
                if !evidence.iter().any(|e| e.essential) {
                    for e in &mut evidence {
                        e.essential = true;
                    }
                }
                queries.push(GoldQuery {
                    id: format!("log-{conv}-{}", row["seq"].as_i64().unwrap_or(0)),
                    question: text,
                    repo,
                    version,
                    kind: String::new(),
                    source: "logs".into(),
                    evidence,
                    reference_answer: payload["text"].as_str().map(str::to_string),
                });
            }
            _ => {}
        }
    }
    Ok(queries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_come_from_read_tools_only() {
        let chain = json!([
            {"type": "thinking", "text": "x"},
            {"type": "tool_call", "name": "read_sources", "input": {"repo": "r", "version": "v", "names": ["A", "B.f"], "files": ["a.py"]}},
            {"type": "tool_call", "name": "search_symbols", "input": {"repo": "r", "version": "v", "query": "A"}},
            {"type": "tool_call", "name": "get_symbol_source", "input": {"repo": "r", "version": "v", "name": "C"}}
        ]);
        let found = reads(&chain);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].names, vec!["A", "B.f"]);
        assert_eq!(found[0].files, vec!["a.py"]);
        assert_eq!(found[1].names, vec!["C"]);
    }

    #[test]
    fn citations_become_essential_spans_or_files() {
        let message = json!({"sources": [
            {"repo": "r", "version": "v", "file": "a.py", "line": 10, "end_line": 20},
            {"repo": "r", "version": "v", "file": "b.py", "line": 0}
        ]});
        let found = cited(&message);
        assert_eq!(found[0].2, GoldEvidence::span("a.py", (10, 20), true));
        assert_eq!(found[1].2, GoldEvidence::file("b.py", true));
        assert_eq!(
            dominant(
                vec![
                    ("r".into(), "v".into()),
                    ("r".into(), "v".into()),
                    ("s".into(), "w".into())
                ]
                .into_iter()
            ),
            Some(("r".into(), "v".into()))
        );
    }
}
