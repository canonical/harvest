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

    pub async fn reset_ingested(&self) -> Result<()> {
        self.graph
            .run(query("MATCH (v:Version) SET v.ingested = false"))
            .await?;
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

    pub async fn upsert_version(&self, repo: &str, tag: &str, timestamp: i64, ingested: bool) -> Result<()> {
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

    pub async fn write_version(&self, repo: &str, tag: &str, files: &[ParsedFile]) -> Result<()> {
        for file in files {
            self.write_file(repo, tag, file).await?;
        }
        // Call edges are written after all functions exist so cross-file callees resolve.
        self.write_call_edges(repo, tag, files).await?;
        self.link_inheritance_edges(repo, tag).await?;
        self.link_impl_edges(repo, tag).await?;
        self.link_embed_edges(repo, tag).await?;
        self.link_uses_edges(repo, tag).await?;
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
                        "name":       f.name,
                        "kind":       f.kind,
                        "signature":  f.signature,
                        "start_line": f.start_line,
                        "end_line":   f.end_line,
                        "source":     f.source,
                        "impl_type":  f.impl_type,
                    })
                })
                .collect();
            self.graph
                .run(
                    query(
                        "MATCH (file:File {repo: $repo, version: $tag, path: $path})
                         UNWIND $fns AS fn_data
                         MERGE (fn:Function {repo: $repo, version: $tag, file: $path, name: fn_data.name})
                         SET fn.kind       = fn_data.kind,
                             fn.signature  = fn_data.signature,
                             fn.start_line = fn_data.start_line,
                             fn.end_line   = fn_data.end_line,
                             fn.source     = fn_data.source,
                             fn.impl_type  = fn_data.impl_type
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
                        "name":       c.name,
                        "kind":       c.kind,
                        "start_line": c.start_line,
                        "end_line":   c.end_line,
                        "source":     c.source,
                        "bases":      c.bases,
                        "traits":     c.traits,
                        "embeds":     c.embeds,
                        "uses":       c.uses,
                    })
                })
                .collect();
            self.graph
                .run(
                    query(
                        "MATCH (file:File {repo: $repo, version: $tag, path: $path})
                         UNWIND $cls AS cls_data
                         MERGE (c:Class {repo: $repo, version: $tag, file: $path, name: cls_data.name})
                         SET c.kind       = cls_data.kind,
                             c.start_line = cls_data.start_line,
                             c.end_line   = cls_data.end_line,
                             c.source     = cls_data.source,
                             c.bases      = cls_data.bases,
                             c.traits     = cls_data.traits,
                             c.embeds     = cls_data.embeds,
                             c.uses       = cls_data.uses
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

    /// Python/C++: resolve `bases` list → INHERITS edges between Class nodes.
    /// Only creates an edge when the base name is unambiguous (exactly one Class
    /// with that name exists in the version); ambiguous names are skipped rather
    /// than creating spurious edges to wrong types.
    async fn link_inheritance_edges(&self, repo: &str, tag: &str) -> Result<()> {
        self.graph
            .run(
                query(
                    "MATCH (child:Class {repo: $repo, version: $tag})
                     WHERE child.bases IS NOT NULL AND size(child.bases) > 0
                     UNWIND child.bases AS base_name
                     MATCH (parent:Class {repo: $repo, version: $tag, name: base_name})
                     WITH child, base_name, collect(parent) AS parents
                     WHERE size(parents) = 1
                     WITH child, parents[0] AS parent
                     MERGE (child)-[:INHERITS]->(parent)",
                )
                .param("repo", repo)
                .param("tag", tag),
            )
            .await?;
        Ok(())
    }

    /// Rust: resolve `traits` list → IMPLEMENTS edges (struct/enum → trait).
    /// Only creates an edge when the trait name is unambiguous within the version.
    async fn link_impl_edges(&self, repo: &str, tag: &str) -> Result<()> {
        self.graph
            .run(
                query(
                    "MATCH (implementor:Class {repo: $repo, version: $tag})
                     WHERE implementor.traits IS NOT NULL AND size(implementor.traits) > 0
                     UNWIND implementor.traits AS trait_name
                     MATCH (t:Class {repo: $repo, version: $tag, name: trait_name})
                     WITH implementor, trait_name, collect(t) AS traits
                     WHERE size(traits) = 1
                     WITH implementor, traits[0] AS t
                     MERGE (implementor)-[:IMPLEMENTS]->(t)",
                )
                .param("repo", repo)
                .param("tag", tag),
            )
            .await?;
        Ok(())
    }

    /// Rust: resolve `uses` list → USES edges (struct/enum → referenced field type).
    /// Only creates an edge when the type name is unambiguous within the version.
    async fn link_uses_edges(&self, repo: &str, tag: &str) -> Result<()> {
        self.graph
            .run(
                query(
                    "MATCH (user:Class {repo: $repo, version: $tag})
                     WHERE user.uses IS NOT NULL AND size(user.uses) > 0
                     UNWIND user.uses AS used_name
                     MATCH (used:Class {repo: $repo, version: $tag, name: used_name})
                     WITH user, used_name, collect(used) AS candidates
                     WHERE size(candidates) = 1
                     WITH user, candidates[0] AS used
                     MERGE (user)-[:USES]->(used)",
                )
                .param("repo", repo)
                .param("tag", tag),
            )
            .await?;
        Ok(())
    }

    /// Go: resolve `embeds` list → EMBEDS edges (outer struct → embedded type).
    /// Only creates an edge when the embedded type name is unambiguous within the version.
    async fn link_embed_edges(&self, repo: &str, tag: &str) -> Result<()> {
        self.graph
            .run(
                query(
                    "MATCH (outer:Class {repo: $repo, version: $tag})
                     WHERE outer.embeds IS NOT NULL AND size(outer.embeds) > 0
                     UNWIND outer.embeds AS embed_name
                     MATCH (inner:Class {repo: $repo, version: $tag, name: embed_name})
                     WITH outer, embed_name, collect(inner) AS candidates
                     WHERE size(candidates) = 1
                     WITH outer, candidates[0] AS inner
                     MERGE (outer)-[:EMBEDS]->(inner)",
                )
                .param("repo", repo)
                .param("tag", tag),
            )
            .await?;
        Ok(())
    }

    /// Write CALLS edges from in-memory call data collected during parsing.
    /// Runs after all files are written so cross-file callees can be resolved.
    ///
    /// Resolution strategy (in priority order):
    ///   1. Same-file match — the callee is defined in the caller's own file.
    ///   2. Unique repo-wide match — the name resolves to exactly one function
    ///      across the whole version (unambiguous cross-file call).
    ///   3. Ambiguous — multiple candidates, no same-file match → edge skipped.
    ///
    /// Unresolved external calls (library functions, builtins) are silently dropped.
    async fn write_call_edges(&self, repo: &str, tag: &str, files: &[ParsedFile]) -> Result<()> {
        for file in files {
            // Collect only functions that actually have recorded calls.
            let fn_calls: Vec<serde_json::Value> = file
                .functions
                .iter()
                .filter(|f| !f.calls.is_empty())
                .map(|f| {
                    serde_json::json!({
                        "caller":      f.name,
                        "caller_file": f.file,
                        "calls": f.calls.iter().map(|c| serde_json::json!({
                            "callee": c.callee,
                            "line":   c.line,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();

            if fn_calls.is_empty() { continue; }

            self.graph
                .run(
                    query(
                        "UNWIND $fn_calls AS fc
                         MATCH (caller:Function {repo: $repo, version: $tag, file: fc.caller_file, name: fc.caller})
                         UNWIND fc.calls AS call
                         OPTIONAL MATCH (same_file:Function {repo: $repo, version: $tag, file: fc.caller_file, name: call.callee})
                         WITH caller, call, same_file
                         OPTIONAL MATCH (any_file:Function {repo: $repo, version: $tag, name: call.callee})
                         WITH caller, call, same_file, collect(any_file) AS all_matches
                         WITH caller, call,
                              CASE
                                WHEN same_file IS NOT NULL THEN same_file
                                WHEN size(all_matches) = 1 THEN all_matches[0]
                                ELSE null
                              END AS callee
                         WHERE callee IS NOT NULL AND callee <> caller
                         MERGE (caller)-[:CALLS {line: call.line}]->(callee)",
                    )
                    .param("repo", repo)
                    .param("tag", tag)
                    .param("fn_calls", to_bolt_list(fn_calls)),
                )
                .await?;
        }
        Ok(())
    }
}
