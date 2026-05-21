use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use crate::neo4j::Neo4jClient;
use super::tool::Tool;

pub struct ListRepositoriesTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for ListRepositoriesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_repositories".into(),
            description: "Return all known repositories and their fully-ingested versions."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn execute(&self, _params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (r:Repository)-[:HAS_VERSION]->(v:Version {ingested: true})
             RETURN r.name AS repo, collect(v.tag) AS versions
             ORDER BY r.name",
            json!({}),
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct SearchSymbolsTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for SearchSymbolsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_symbols".into(),
            description: "Full-text search for functions or classes by name fragment. \
                          Returns up to 20 matches ranked by relevance.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query":   { "type": "string", "description": "Name fragment to search for" },
                    "repo":    { "type": "string", "description": "Filter to this repository (optional)" },
                    "version": { "type": "string", "description": "Filter to this version tag (optional)" },
                    "kind":    { "type": "string", "enum": ["function", "class", "any"],
                                 "description": "Limit to functions, classes, or either" }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let q = params["query"].as_str().unwrap_or("").to_string();
        let repo    = params["repo"].as_str().unwrap_or("").to_string();
        let version = params["version"].as_str().unwrap_or("").to_string();

        let rows = self.0.query_read(
            "CALL db.index.fulltext.queryNodes('symbol_names', $query) YIELD node, score
             WHERE ($repo    = '' OR node.repo    = $repo)
               AND ($version = '' OR node.version = $version)
             RETURN node.repo AS repo, node.version AS version, node.file AS file,
                    node.name AS name, node.start_line AS start_line, score
             ORDER BY score DESC LIMIT 20",
            json!({ "query": q, "repo": repo, "version": version }),
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct GetSymbolSourceTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for GetSymbolSourceTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_symbol_source".into(),
            description: "Return the full source text of a specific function or class.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo":    { "type": "string" },
                    "version": { "type": "string" },
                    "file":    { "type": "string" },
                    "name":    { "type": "string" }
                },
                "required": ["repo", "version", "file", "name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (n {repo: $repo, version: $version, file: $file, name: $name})
             WHERE n:Function OR n:Class
             RETURN n.name AS name, n.start_line AS start_line, n.end_line AS end_line,
                    n.signature AS signature, n.source AS source
             LIMIT 1",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct GetFileSymbolsTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for GetFileSymbolsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_file_symbols".into(),
            description: "List all functions and classes defined in a file (without source text)."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo":    { "type": "string" },
                    "version": { "type": "string" },
                    "file":    { "type": "string" }
                },
                "required": ["repo", "version", "file"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (f:File {repo: $repo, version: $version, path: $file})-[:DEFINES]->(n)
             RETURN labels(n)[0] AS kind, n.name AS name,
                    n.start_line AS start_line, n.end_line AS end_line,
                    n.signature AS signature
             ORDER BY n.start_line",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct FindCallersTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for FindCallersTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find_callers".into(),
            description: "Find all functions that call the given function within a version.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo":          { "type": "string" },
                    "version":       { "type": "string" },
                    "function_name": { "type": "string" }
                },
                "required": ["repo", "version", "function_name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (caller:Function {repo: $repo, version: $version})
                  -[c:CALLS]->
                  (callee:Function {repo: $repo, version: $version, name: $function_name})
             RETURN caller.file AS file, caller.name AS caller,
                    c.line AS call_site_line",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct FindCalleesTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for FindCalleesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find_callees".into(),
            description: "Find all functions called by the given function.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo":          { "type": "string" },
                    "version":       { "type": "string" },
                    "file":          { "type": "string" },
                    "function_name": { "type": "string" }
                },
                "required": ["repo", "version", "file", "function_name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (caller:Function {repo: $repo, version: $version,
                                     file: $file, name: $function_name})
                  -[c:CALLS]->(callee:Function)
             RETURN callee.name AS callee, callee.file AS defined_in,
                    c.line AS call_site_line",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct GetImportsTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for GetImportsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "get_imports".into(),
            description: "Return all import declarations for a file.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo":    { "type": "string" },
                    "version": { "type": "string" },
                    "file":    { "type": "string" }
                },
                "required": ["repo", "version", "file"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (:File {repo: $repo, version: $version, path: $file})-[:IMPORTS]->(i:Import)
             RETURN i.target AS target, i.line AS line
             ORDER BY i.line",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct CompareSymbolAcrossVersionsTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for CompareSymbolAcrossVersionsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "compare_symbol_across_versions".into(),
            description: "Return the source text of a named symbol in two versions side-by-side, \
                          useful for answering 'what changed between vA and vB'.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo":      { "type": "string" },
                    "version_a": { "type": "string" },
                    "version_b": { "type": "string" },
                    "file":      { "type": "string" },
                    "name":      { "type": "string" }
                },
                "required": ["repo", "version_a", "version_b", "file", "name"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let rows = self.0.query_read(
            "MATCH (n {repo: $repo, file: $file, name: $name})
             WHERE n:Function OR n:Class
               AND n.version IN [$version_a, $version_b]
             RETURN n.version AS version, n.start_line AS start_line,
                    n.end_line AS end_line, n.source AS source
             ORDER BY n.version",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct RunCypherTool(pub Arc<Neo4jClient>);

#[async_trait]
impl Tool for RunCypherTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_cypher".into(),
            description: "Execute a custom read-only Cypher query against the knowledge graph. \
                          Use this when the other tools cannot express the traversal you need. \
                          Only SELECT-equivalent queries are permitted — writes are rejected.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query":  { "type": "string", "description": "Cypher query string" },
                    "params": { "type": "object", "description": "Named query parameters (optional)" }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let cypher = params["query"].as_str().unwrap_or("").to_string();
        let query_params = params.get("params").cloned().unwrap_or(json!({}));
        let rows = self.0.query_read(&cypher, query_params).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub fn all_tools(neo4j: Arc<Neo4jClient>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ListRepositoriesTool(Arc::clone(&neo4j))),
        Box::new(SearchSymbolsTool(Arc::clone(&neo4j))),
        Box::new(GetSymbolSourceTool(Arc::clone(&neo4j))),
        Box::new(GetFileSymbolsTool(Arc::clone(&neo4j))),
        Box::new(FindCallersTool(Arc::clone(&neo4j))),
        Box::new(FindCalleesTool(Arc::clone(&neo4j))),
        Box::new(GetImportsTool(Arc::clone(&neo4j))),
        Box::new(CompareSymbolAcrossVersionsTool(Arc::clone(&neo4j))),
        Box::new(RunCypherTool(neo4j)),
    ]
}
