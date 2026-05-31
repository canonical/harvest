pub mod llm;
pub mod workflow;

use anyhow::{Result, bail};
use neo4rs::{query, Graph};
use serde::{Deserialize, Serialize};
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
    graph: Graph,
    llm: Box<dyn llm::LlmClient>,
    docs_dir: PathBuf,
}

impl DocumentationPipeline {
    pub async fn new(
        neo4j_uri: &str,
        neo4j_user: &str,
        neo4j_password: &str,
        llm_config: &LlmConfig,
        doc_config: &DocumentationConfig,
    ) -> Result<Self> {
        let graph = Graph::new(neo4j_uri, neo4j_user, neo4j_password).await?;
        let llm = llm::from_config(llm_config);
        Ok(Self { graph, llm, docs_dir: doc_config.docs_dir.clone() })
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
        let mut result = self
            .graph
            .execute(
                query(
                    "MATCH (v:Version {repo: $repo, tag: $tag, ingested: true}) \
                     RETURN v LIMIT 1",
                )
                .param("repo", repo)
                .param("tag", version),
            )
            .await?;
        if result.next().await?.is_none() {
            bail!("repository {repo}:{version} not found or not ingested");
        }
        Ok(())
    }

    async fn fetch_structure(&self, repo: &str, version: &str) -> Result<Vec<StructureRow>> {
        let mut result = self
            .graph
            .execute(
                query(
                    "MATCH (f:File {repo: $repo, version: $version}) \
                     OPTIONAL MATCH (f)-[:DEFINES]->(s) \
                     RETURN f.path AS path, f.language AS language, \
                            collect({name: s.name, kind: labels(s)[0], \
                                     signature: s.signature}) AS symbols \
                     ORDER BY f.path",
                )
                .param("repo", repo)
                .param("version", version),
            )
            .await?;

        let mut rows = Vec::new();
        while let Some(row) = result.next().await? {
            let path: String = row.get("path").unwrap_or_default();
            let language: String = row.get("language").unwrap_or_default();
            let symbols_raw: Vec<serde_json::Value> = row
                .get::<serde_json::Value>("symbols")
                .ok()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();

            let symbols = symbols_raw
                .into_iter()
                .filter_map(|s| {
                    let name = s["name"].as_str()?.to_string();
                    let kind = s["kind"].as_str().unwrap_or("Unknown").to_string();
                    let signature = s["signature"].as_str().map(String::from);
                    Some(SymbolInfo { name, kind, signature })
                })
                .collect();

            rows.push(StructureRow { path, language, symbols });
        }
        Ok(rows)
    }

    async fn fetch_sources(&self, repo: &str, version: &str) -> Result<Vec<(String, String)>> {
        let mut result = self
            .graph
            .execute(
                query(
                    "MATCH (s {repo: $repo, version: $version}) \
                     WHERE s.source IS NOT NULL AND s.name IS NOT NULL \
                     RETURN s.name AS name, s.source AS source \
                     LIMIT 200",
                )
                .param("repo", repo)
                .param("version", version),
            )
            .await?;

        let mut sources = Vec::new();
        while let Some(row) = result.next().await? {
            let name: String = row.get("name").unwrap_or_default();
            let source: String = row.get("source").unwrap_or_default();
            if !name.is_empty() && !source.is_empty() {
                sources.push((name, source));
            }
        }
        Ok(sources)
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
