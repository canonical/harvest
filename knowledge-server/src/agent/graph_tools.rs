use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::llm::types::ToolDefinition;
use harvest_db::{Db, GRAPH_READER_ROLE};
use super::tool::Tool;

pub struct ListRepositoriesTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT r.name AS repo, count(*) AS version_count,
                    (array_agg(v.tag ORDER BY v.timestamp DESC))[1:10] AS versions
             FROM repositories r JOIN versions v ON v.repository_id = r.id AND v.ingested
             GROUP BY r.name ORDER BY r.name",
            json!({}),
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct SearchSymbolsTool(pub Arc<Db>);

#[async_trait]
impl Tool for SearchSymbolsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_symbols".into(),
            description: "Full-text search for functions or classes by name fragment. \
                           Returns up to 10 matches ranked by relevance.".into(),
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
        let kind    = params["kind"].as_str().unwrap_or("any").to_string();

        let rows = self.0.query(
            "SELECT repo, version, file, name, start_line, lower(label) AS kind,
                    CASE WHEN name = $query THEN 1000
                         WHEN lower(name) = lower($query) THEN 900
                         ELSE similarity(lower(name), lower($query)) END::float8 AS score
             FROM code_symbols
             WHERE (lower(name) % lower($query)
                    OR strpos(lower(name), lower($query)) > 0)
               AND ($repo = '' OR repo = $repo)
               AND ($version = '' OR version = $version)
               AND ($kind = 'any' OR lower(label) = $kind)
             ORDER BY score DESC, name
             LIMIT 10",
            json!({ "query": q, "repo": repo, "version": version, "kind": kind }),
        ).await?;

        if rows.is_empty() {
            Ok("No symbols found. Try a different name fragment or broader search term.".into())
        } else {
            Ok(serde_json::to_string_pretty(&rows)?)
        }
    }
}

pub struct GetSymbolSourceTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT name, start_line, end_line, signature, source
             FROM code_symbols
             WHERE repo = $repo AND version = $version AND file = $file AND name = $name
             LIMIT 1",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }

    fn preview(&self, result: &str) -> String {
        result.to_string()
    }
}

pub struct GetFileSymbolsTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT label AS kind, name, start_line, end_line, signature
             FROM code_symbols
             WHERE repo = $repo AND version = $version AND file = $file
             ORDER BY start_line",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct FindCallersTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT src_file AS file, src_name AS caller, line AS call_site_line
                  FROM code_edges
                  WHERE relation = 'CALLS' AND repo = $repo AND version = $version
                    AND dst_name = $function_name AND dst_label = 'Function'",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct FindCalleesTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT dst_name AS callee, dst_file AS defined_in, line AS call_site_line
                                     FROM code_edges
                                     WHERE relation = 'CALLS' AND repo = $repo AND version = $version
                                       AND src_file = $file AND src_name = $function_name",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct GetImportsTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT target, line FROM code_imports
             WHERE repo = $repo AND version = $version AND file = $file
             ORDER BY line",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub struct CompareSymbolAcrossVersionsTool(pub Arc<Db>);

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
        let rows = self.0.query(
            "SELECT version, start_line, end_line, source
             FROM code_symbols
             WHERE repo = $repo AND file = $file AND name = $name
               AND version IN ($version_a, $version_b)
             ORDER BY version",
            params,
        ).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }

    fn preview(&self, result: &str) -> String {
        result.to_string()
    }
}

pub struct RunSqlTool(pub Arc<Db>);

#[async_trait]
impl Tool for RunSqlTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_sql".into(),
            description: "Execute a custom read-only PostgreSQL SELECT against the code knowledge graph \
                          (views code_repositories, code_versions, code_files, code_symbols, code_imports, \
                          code_edges). Use this when the other tools cannot express the query you need. \
                          Writes are rejected.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query":  { "type": "string", "description": "A single SELECT (or WITH … SELECT) statement; named parameters are written $name" },
                    "params": { "type": "object", "description": "Named query parameters (optional)" }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let sql = params["query"].as_str().unwrap_or("").to_string();
        let query_params = params.get("params").cloned().unwrap_or(json!({}));
        let rows = run_read_only_sql(&self.0, &sql, query_params).await?;
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub fn reject_non_select(sql: &str) -> Result<()> {
    let first_word: String = sql.trim_start()
        .trim_start_matches('(')
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_ascii_uppercase();
    if !matches!(first_word.as_str(), "SELECT" | "WITH" | "VALUES" | "TABLE") {
        anyhow::bail!("run_sql only accepts a single SELECT statement");
    }
    Ok(())
}

/// Runs model-written SQL in a read-only transaction as the `harvest_graph_reader` role,
/// which can only see the code_* views, and always rolls back.
pub async fn run_read_only_sql(db: &Db, sql: &str, params: Value) -> Result<Vec<Value>> {
    reject_non_select(sql)?;
    let tx = db.begin().await?;
    tx.batch_execute("SET TRANSACTION READ ONLY; SET LOCAL statement_timeout = '10s'").await?;
    if let Err(e) = tx.batch_execute(&format!("SET LOCAL ROLE {GRAPH_READER_ROLE}")).await {
        tracing::error!(error = %e, "run_sql: cannot switch to the read-only role");
        anyhow::bail!(
            "run_sql is unavailable: the database role {GRAPH_READER_ROLE} is missing or not granted \
             to the Harvest database user"
        );
    }
    let result = tx.query_uncached(sql, params).await;
    tx.rollback().await?;
    result
}

pub fn all_tools(db: Arc<Db>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ListRepositoriesTool(Arc::clone(&db))),
        Box::new(SearchSymbolsTool(Arc::clone(&db))),
        Box::new(GetSymbolSourceTool(Arc::clone(&db))),
        Box::new(GetFileSymbolsTool(Arc::clone(&db))),
        Box::new(FindCallersTool(Arc::clone(&db))),
        Box::new(FindCalleesTool(Arc::clone(&db))),
        Box::new(GetImportsTool(Arc::clone(&db))),
        Box::new(CompareSymbolAcrossVersionsTool(Arc::clone(&db))),
        Box::new(RunSqlTool(db)),
    ]
}

#[cfg(test)]
mod tests {
    use super::reject_non_select;

    #[test]
    fn accepts_select_statements() {
        for q in [
            "SELECT name FROM code_symbols",
            "  select path from code_files where path like '%test%'",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "(SELECT 1) UNION (SELECT 2)",
        ] {
            assert!(reject_non_select(q).is_ok(), "should allow: {q}");
        }
    }

    #[test]
    fn rejects_other_statements() {
        for q in [
            "DELETE FROM users",
            "UPDATE users SET role = 'admin'",
            "INSERT INTO users (id) VALUES ('x')",
            "DROP TABLE users",
            "SET ROLE harvest",
            "RESET ROLE",
            "COMMIT",
            "COPY users TO STDOUT",
            "",
        ] {
            assert!(reject_non_select(q).is_err(), "should reject: {q}");
        }
    }
}
