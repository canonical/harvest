pub mod llm;
pub mod workflow;
mod retry;

use anyhow::{Result, bail};
use harvest_db::Db;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::{DocumentationConfig, LlmConfig};
use workflow::{StructureRow, SymbolInfo, Workflow};

#[derive(Serialize, Deserialize, Debug)]
pub struct IndexEntry {
    pub filename: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DocIndex {
    pub repo: String,
    pub version: String,
    pub generated_at: String,
    pub sections: HashMap<String, Vec<IndexEntry>>,
}

pub struct DocumentationPipeline {
    db: Db,
    llm: Box<dyn llm::LlmClient>,
    docs_dir: PathBuf,
}

impl DocumentationPipeline {
    pub fn new(db: Db, llm_config: &LlmConfig, doc_config: &DocumentationConfig) -> Self {
        let llm = llm::from_config(llm_config);
        Self { db, llm, docs_dir: doc_config.docs_dir.clone() }
    }

    pub async fn document(&self, repo: &str, version: &str) -> Result<()> {
        self.verify_ingested(repo, version).await?;

        let structure = self.fetch_structure(repo, version).await?;
        if structure.is_empty() {
            bail!("no files found for {repo}:{version} — make sure it has been ingested first");
        }

        let sources = self.fetch_sources(repo, version).await?;

        let workflow = Workflow { llm: self.llm.as_ref(), docs_dir: &self.docs_dir };
        workflow.run(repo, version, &structure, &sources).await
    }

    async fn verify_ingested(&self, repo: &str, version: &str) -> Result<()> {
        let rows = self.db.query(
            "SELECT 1 AS ok FROM code_versions WHERE repo = $repo AND tag = $tag AND ingested LIMIT 1",
            json!({ "repo": repo, "tag": version }),
        ).await?;
        if rows.is_empty() {
            bail!("repository {repo}:{version} not found or not ingested");
        }
        Ok(())
    }

    async fn fetch_structure(&self, repo: &str, version: &str) -> Result<Vec<StructureRow>> {
        let rows = self.db.query(
            "SELECT f.path, f.language,
                    COALESCE(json_agg(json_build_object('name', s.name, 'kind', s.label,
                                                        'signature', s.signature)
                                      ORDER BY s.start_line) FILTER (WHERE s.id IS NOT NULL),
                             '[]') AS symbols
             FROM code_files f
             LEFT JOIN symbols s ON s.file_id = f.id
             WHERE f.repo = $repo AND f.version = $version
             GROUP BY f.id, f.path, f.language
             ORDER BY f.path",
            json!({ "repo": repo, "version": version }),
        ).await?;

        Ok(rows.into_iter().map(|row| {
            let symbols = row["symbols"].as_array().cloned().unwrap_or_default()
                .into_iter()
                .filter_map(|s| {
                    let name = s["name"].as_str()?.to_string();
                    let kind = s["kind"].as_str().unwrap_or("Unknown").to_string();
                    let signature = s["signature"].as_str().map(String::from);
                    Some(SymbolInfo { name, kind, signature })
                })
                .collect();
            StructureRow {
                path: row["path"].as_str().unwrap_or_default().to_string(),
                language: row["language"].as_str().unwrap_or_default().to_string(),
                symbols,
            }
        }).collect())
    }

    async fn fetch_sources(&self, repo: &str, version: &str) -> Result<Vec<(String, String)>> {
        let rows = self.db.query(
            "SELECT name, source FROM code_symbols
             WHERE repo = $repo AND version = $version AND source <> ''
             ORDER BY file, start_line
             LIMIT 200",
            json!({ "repo": repo, "version": version }),
        ).await?;
        Ok(rows.into_iter()
            .filter_map(|r| Some((r["name"].as_str()?.to_string(), r["source"].as_str()?.to_string())))
            .filter(|(name, source)| !name.is_empty() && !source.is_empty())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_index_serializes_all_sections() {
        let mut sections = HashMap::new();
        sections.insert(
            "tutorials".to_string(),
            vec![IndexEntry {
                filename: "getting-started.md".to_string(),
                title: "Getting Started".to_string(),
            }],
        );
        sections.insert("how-to-guides".to_string(), vec![]);
        sections.insert("explanations".to_string(), vec![]);
        sections.insert("reference".to_string(), vec![]);

        let index = DocIndex {
            repo: "testrepo".to_string(),
            version: "v1.0".to_string(),
            generated_at: "2026-05-30T00:00:00Z".to_string(),
            sections,
        };
        let json = serde_json::to_string(&index).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["repo"], "testrepo");
        assert_eq!(parsed["sections"]["tutorials"][0]["filename"], "getting-started.md");
    }
}
