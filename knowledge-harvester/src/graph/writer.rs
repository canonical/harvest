use std::convert::TryInto;

use anyhow::Result;
use neo4rs::{query, BoltType, Graph};

use super::model::ParsedFile;

fn to_bolt_list(values: Vec<serde_json::Value>) -> BoltType {
    serde_json::Value::Array(values)
        .try_into()
        .expect("well-formed JSON array")
}

pub struct GraphWriter {
    graph: Graph,
}

impl GraphWriter {
    pub async fn new(uri: &str, user: &str, password: &str) -> Result<Self> {
        let graph = Graph::new(uri, user, password).await?;
        Ok(Self { graph })
    }

    pub async fn ensure_indexes(&self) -> Result<()> {
        let stmts = [
            "CREATE INDEX repo_name IF NOT EXISTS FOR (r:Repository) ON (r.name)",
            "CREATE INDEX version_key IF NOT EXISTS FOR (v:Version) ON (v.repo, v.tag)",
            "CREATE INDEX file_path IF NOT EXISTS FOR (f:File) ON (f.repo, f.version, f.path)",
            "CREATE INDEX fn_key IF NOT EXISTS FOR (f:Function) ON (f.repo, f.version, f.name)",
            "CREATE INDEX cls_key IF NOT EXISTS FOR (c:Class) ON (c.repo, c.version, c.name)",
            "CREATE FULLTEXT INDEX symbol_names IF NOT EXISTS FOR (n:Function|Class) ON EACH [n.name]",
            "CREATE FULLTEXT INDEX file_paths IF NOT EXISTS FOR (f:File) ON EACH [f.path]",
        ];
        for stmt in stmts {
            self.graph.run(query(stmt)).await?;
        }
        Ok(())
    }

    pub async fn is_ingested(&self, repo: &str, tag: &str) -> Result<bool> {
        let mut result = self
            .graph
            .execute(
                query("MATCH (v:Version {repo: $repo, tag: $tag, ingested: true}) RETURN v LIMIT 1")
                    .param("repo", repo)
                    .param("tag", tag),
            )
            .await?;
        Ok(result.next().await?.is_some())
    }

    pub async fn ingested_versions(&self, repo: &str) -> Result<Vec<String>> {
        let mut result = self
            .graph
            .execute(
                query("MATCH (v:Version {repo: $repo, ingested: true}) RETURN v.tag AS tag ORDER BY v.timestamp")
                    .param("repo", repo),
            )
            .await?;
        let mut tags = Vec::new();
        while let Some(row) = result.next().await? {
            tags.push(row.get::<String>("tag")?);
        }
        Ok(tags)
    }

    pub async fn upsert_repository(&self, name: &str, url: &str) -> Result<()> {
        self.graph
            .run(
                query("MERGE (r:Repository {name: $name}) SET r.url = $url")
                    .param("name", name)
                    .param("url", url),
            )
            .await?;
        Ok(())
    }

    pub async fn upsert_version(
        &self,
        repo: &str,
        tag: &str,
        timestamp: i64,
        ingested: bool,
    ) -> Result<()> {
        self.graph
            .run(
                query(
                    "MERGE (r:Repository {name: $repo})
                     MERGE (v:Version {repo: $repo, tag: $tag})
                     SET v.timestamp = $timestamp, v.ingested = $ingested
                     MERGE (r)-[:HAS_VERSION]->(v)",
                )
                .param("repo", repo)
                .param("tag", tag)
                .param("timestamp", timestamp)
                .param("ingested", ingested),
            )
            .await?;
        Ok(())
    }

    pub async fn write_version(
        &self,
        repo: &str,
        tag: &str,
        files: &[ParsedFile],
    ) -> Result<()> {
        for file in files {
            self.write_file(repo, tag, file).await?;
        }
        self.link_call_edges(repo, tag).await?;
        self.graph
            .run(
                query("MATCH (v:Version {repo: $repo, tag: $tag}) SET v.ingested = true")
                    .param("repo", repo)
                    .param("tag", tag),
            )
            .await?;
        Ok(())
    }

    async fn write_file(&self, repo: &str, tag: &str, file: &ParsedFile) -> Result<()> {
        self.graph
            .run(
                query(
                    "MERGE (v:Version {repo: $repo, tag: $tag})
                     MERGE (f:File {repo: $repo, version: $tag, path: $path})
                     SET f.language = $lang
                     MERGE (v)-[:HAS_FILE]->(f)",
                )
                .param("repo", repo)
                .param("tag", tag)
                .param("path", file.path.as_str())
                .param("lang", file.language.as_str()),
            )
            .await?;

        if !file.functions.is_empty() {
            let fns: Vec<_> = file
                .functions
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "name": f.name, "signature": f.signature,
                        "start_line": f.start_line, "end_line": f.end_line,
                        "source": f.source,
                    })
                })
                .collect();
            self.graph
                .run(
                    query(
                        "MATCH (file:File {repo: $repo, version: $tag, path: $path})
                         UNWIND $fns AS fn_data
                         MERGE (fn:Function {repo: $repo, version: $tag, file: $path, name: fn_data.name})
                         SET fn.signature  = fn_data.signature,
                             fn.start_line = fn_data.start_line,
                             fn.end_line   = fn_data.end_line,
                             fn.source     = fn_data.source
                         MERGE (file)-[:DEFINES]->(fn)",
                    )
                    .param("repo", repo)
                    .param("tag", tag)
                    .param("path", file.path.as_str())
                    .param("fns", to_bolt_list(fns)),
                )
                .await?;
        }

        if !file.classes.is_empty() {
            let cls: Vec<_> = file
                .classes
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "start_line": c.start_line, "end_line": c.end_line,
                        "source": c.source,
                    })
                })
                .collect();
            self.graph
                .run(
                    query(
                        "MATCH (file:File {repo: $repo, version: $tag, path: $path})
                         UNWIND $cls AS cls_data
                         MERGE (c:Class {repo: $repo, version: $tag, file: $path, name: cls_data.name})
                         SET c.start_line = cls_data.start_line,
                             c.end_line   = cls_data.end_line,
                             c.source     = cls_data.source
                         MERGE (file)-[:DEFINES]->(c)",
                    )
                    .param("repo", repo)
                    .param("tag", tag)
                    .param("path", file.path.as_str())
                    .param("cls", to_bolt_list(cls)),
                )
                .await?;
        }

        if !file.imports.is_empty() {
            let imps: Vec<_> = file
                .imports
                .iter()
                .map(|i| serde_json::json!({ "target": i.target, "line": i.line }))
                .collect();
            self.graph
                .run(
                    query(
                        "MATCH (src:File {repo: $repo, version: $tag, path: $path})
                         UNWIND $imps AS imp
                         MERGE (imp_node:Import {repo: $repo, version: $tag, file: $path, target: imp.target})
                         SET imp_node.line = imp.line
                         MERGE (src)-[:IMPORTS]->(imp_node)",
                    )
                    .param("repo", repo)
                    .param("tag", tag)
                    .param("path", file.path.as_str())
                    .param("imps", to_bolt_list(imps)),
                )
                .await?;
        }

        Ok(())
    }

    async fn link_call_edges(&self, repo: &str, tag: &str) -> Result<()> {
        self.graph
            .run(
                query(
                    "MATCH (caller:Function {repo: $repo, version: $tag})
                     WHERE caller.calls IS NOT NULL
                     UNWIND caller.calls AS call
                     OPTIONAL MATCH (callee:Function {repo: $repo, version: $tag, name: call.callee})
                     WITH caller, call, coalesce(callee, null) AS resolved
                     CALL {
                         WITH caller, call, resolved
                         WITH caller, call,
                              CASE WHEN resolved IS NULL
                                   THEN '?' + call.callee
                                   ELSE call.callee
                              END AS callee_name
                         MERGE (fn:Function {repo: $repo, version: $tag, name: callee_name})
                         MERGE (caller)-[:CALLS {line: call.line}]->(fn)
                     }",
                )
                .param("repo", repo)
                .param("tag", tag),
            )
            .await?;
        Ok(())
    }
}
