use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
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
            description: "Return all known repositories and their fully-ingested versions. \
                          Versions listed under `stale_versions` were indexed by an older \
                          harvester and may lack inheritance and docstrings; say so when an \
                          answer depends on them."
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
                    (array_agg(v.tag ORDER BY v.timestamp DESC))[1:10] AS versions,
                    coalesce(array_agg(v.tag ORDER BY v.timestamp DESC)
                             FILTER (WHERE v.parser_version < $parser_version), '{}') AS stale_versions
             FROM repositories r JOIN versions v ON v.repository_id = r.id AND v.ingested
             GROUP BY r.name ORDER BY r.name",
            json!({ "parser_version": knowledge_harvester::parser::PARSER_VERSION }),
        ).await?;
        let rows: Vec<Value> = rows.into_iter().map(|mut row| {
            if row["stale_versions"].as_array().is_some_and(|a| a.is_empty()) {
                if let Some(obj) = row.as_object_mut() {
                    obj.remove("stale_versions");
                }
            }
            row
        }).collect();
        Ok(serde_json::to_string_pretty(&rows)?)
    }
}

pub const SEARCH_LIMIT_DEFAULT: usize = 25;
pub const SEARCH_LIMIT_MAX: usize = 50;
pub const SEARCH_PREVIEW_CHARS: usize = 400;
pub const READ_SOURCES_BUDGET_CHARS: usize = 12_000;
pub const READ_SOURCES_MAX_SYMBOLS: usize = 25;

pub const SEARCH_RANKING: &[(&str, i64)] = &[
    ("exact_name", 1000),
    ("exact_name_ci", 900),
    ("name", 600),
    ("capability", 550),
    ("name_fuzzy", 500),
    ("sig_sub", 320),
    ("signature", 300),
    ("path", 200),
    ("doc_sub", 170),
    ("docstring", 150),
];

pub const SEARCH_CHAIN: &[&str] = &[
    "exact_name",
    "exact_name_ci",
    "name",
    "capability",
    "name_fuzzy",
    "sig_sub",
    "signature",
    "path",
    "doc_sub",
    "docstring",
];

pub fn fallback_chain() -> &'static [&'static str] {
    SEARCH_CHAIN
}

pub fn search_symbols_definition() -> ToolDefinition {
    ToolDefinition {
        name: "search_symbols".into(),
        description: "Search classes and functions by identifier. Matching is tried across \
                      several fields in one call — symbol name, file path, signature, capability \
                      constants declared inside a class body, and definition text — so a single \
                      search is enough to find the declaration that gates a capability, not just \
                      the method that implements it. Each result carries a short preview so you \
                      can triage without fetching the full source. To list everything under a \
                      directory, pass `path_prefix` with an empty `query` and page with `offset`; \
                      a final `more_results` entry tells you the next offset."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query":   { "type": "string", "description": "Name fragment, path fragment, or term to search for. May be empty when path_prefix is set" },
                "path_prefix": { "type": "string", "description": "Only symbols whose file path starts with this, e.g. cinder/volume/drivers/ (optional)" },
                "offset":  { "type": "integer", "description": "Skip this many results, for paging (default 0)" },
                "repo":    { "type": "string", "description": "Filter to this repository (optional)" },
                "version": { "type": "string", "description": "Filter to this version tag (optional)" },
                "kind":    { "type": "string", "enum": ["function", "class", "any"],
                             "description": "Limit to functions, classes, or either" },
                "limit":   { "type": "integer",
                             "description": format!("Maximum results, {} to {} (default {})", 1, SEARCH_LIMIT_MAX, SEARCH_LIMIT_DEFAULT) }
            },
            "required": ["query"]
        }),
    }
}

pub fn clamp_limit(params: &Value) -> usize {
    clamp_limit_key(params, "limit")
}

pub fn clamp_limit_key(params: &Value, key: &str) -> usize {
    match params.get(key).and_then(Value::as_i64) {
        Some(n) if n > 0 => (n as usize).min(SEARCH_LIMIT_MAX),
        Some(_) => 1,
        None => SEARCH_LIMIT_DEFAULT,
    }
}

pub fn source_preview(source: &str, max: usize) -> String {
    if source.chars().count() <= max {
        return source.to_string();
    }
    let mut out: String = source.chars().take(max).collect();
    out.push('…');
    out
}

#[derive(Default)]
pub struct SourceTargets {
    pub files: Vec<String>,
    pub names: Vec<String>,
}

pub fn source_targets(params: &Value) -> SourceTargets {
    SourceTargets {
        files: strings_from_param(params, &["files", "file"]),
        names: strings_from_param(params, &["names", "name", "symbols"]),
    }
}

pub fn format_sources_for_budget(output: &str, budget: usize) -> String {
    if output.chars().count() <= budget {
        return output.to_string();
    }
    let omitted = output.chars().count() - budget;
    let mut capped: String = output.chars().take(budget).collect();
    capped.push_str(&format!("\n\n[truncated, {omitted} more characters omitted]"));
    capped
}

pub fn read_sources_definition() -> ToolDefinition {
    ToolDefinition {
        name: "read_sources".into(),
        description: "Fetch source for several files or symbols in a single call. Prefer this \
                      over one call per file: pass every file or symbol you need at once and read \
                      them together. Returns each symbol's name, file, line range, and source."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "repo":    { "type": "string" },
                "version": { "type": "string" },
                "files": {
                    "type": "array", "items": { "type": "string" },
                    "description": "File paths to read"
                },
                "names": {
                    "type": "array", "items": { "type": "string" },
                    "description": "Class or function names to read, without needing their file"
                }
            },
            "required": ["repo", "version"]
        }),
    }
}

pub struct ReadSourcesTool(pub Arc<Db>);

#[async_trait]
impl Tool for ReadSourcesTool {
    fn definition(&self) -> ToolDefinition {
        read_sources_definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let repo    = params["repo"].as_str().unwrap_or("").trim().to_string();
        let version = params["version"].as_str().unwrap_or("").trim().to_string();
        if repo.is_empty() || version.is_empty() {
            anyhow::bail!("read_sources requires repo and version");
        }
        let targets = source_targets(&params);
        if targets.files.is_empty() && targets.names.is_empty() {
            anyhow::bail!("read_sources requires at least one file path in `files` or symbol name in `names`");
        }

        let rows = self.0.query(
            "SELECT name, file, start_line, end_line, lower(label) AS kind, signature, source
             FROM code_symbols
             WHERE repo = $repo AND version = $version
               AND (file = ANY($files) OR name = ANY($names))
             ORDER BY file, start_line",
            json!({ "repo": repo, "version": version, "files": targets.files, "names": targets.names }),
        ).await?;

        if rows.is_empty() {
            let mut missing = Vec::new();
            for f in &targets.files { missing.push(format!("file {f}")); }
            for n in &targets.names { missing.push(format!("symbol {n}")); }
            return Ok(format!(
                "No symbols found for {} in {repo}:{version}. Use search_symbols to find the exact file path or name.",
                missing.join(", ")
            ));
        }

        let limit = clamp_limit(&params).min(READ_SOURCES_MAX_SYMBOLS);
        let results: Vec<Value> = rows.iter().take(limit).map(|row| {
            json!({
                "file": row["file"],
                "name": row["name"],
                "kind": row["kind"],
                "signature": row["signature"],
                "start_line": row["start_line"],
                "end_line": row["end_line"],
                "source": row["source"],
            })
        }).collect();

        let serialized = serde_json::to_string_pretty(&results)?;
        Ok(truncate_to(&serialized, READ_SOURCES_BUDGET_CHARS).0)
    }
}

const SEARCH_SQL: &str = r#"
WITH signals AS (
    SELECT cs.*,
           regexp_replace(lower($query), '([.^$*+?()\[\]{}|\\-])', '\\\1', 'g') AS q_literal,
           lower(cs.name) % lower($query) AS name_trgm,
           strpos(lower(cs.name), lower($query)) > 0 AS name_sub,
           lower(coalesce(cs.signature, '')) % lower($query) AS sig_trgm,
           strpos(lower(coalesce(cs.signature, '')), lower($query)) > 0 AS sig_sub,
           lower(cs.file) % lower($query) AS path_trgm,
           strpos(lower(cs.file), lower($query)) > 0 AS path_sub,
           lower(coalesce(cs.docstring, '')) % lower($query) AS doc_trgm,
           strpos(lower(coalesce(cs.docstring, '')), lower($query)) > 0 AS doc_sub
    FROM code_symbols cs
    WHERE ($repo = '' OR cs.repo = $repo)
      AND ($version = '' OR cs.version = $version)
      AND ($kind = 'any' OR lower(cs.label) = $kind)
      AND ($path_prefix = '' OR starts_with(cs.file, $path_prefix))
), matched AS (
    SELECT s.*,
           lower(s.label) = 'class'
           AND s.source ~* ('(^|[^A-Za-z0-9_])' || s.q_literal || '([^A-Za-z0-9_]|$)') AS cap_match
    FROM signals s
)
SELECT repo, version, file, name, start_line, lower(label) AS kind,
       signature, source, docstring,
       array_remove(ARRAY[
           CASE WHEN name = $query                 THEN 'exact_name' END,
           CASE WHEN name_sub                      THEN 'name' END,
           CASE WHEN cap_match                     THEN 'capability' END,
           CASE WHEN sig_sub OR sig_trgm           THEN 'signature' END,
           CASE WHEN path_trgm OR path_sub         THEN 'path' END,
           CASE WHEN doc_sub OR doc_trgm           THEN 'docstring' END
       ], NULL) AS matched_on,
       GREATEST(
           CASE WHEN name = $query THEN 1000 ELSE 0 END,
           CASE WHEN lower(name) = lower($query) THEN 900 ELSE 0 END,
           CASE WHEN name_sub THEN 600 ELSE 0 END,
           CASE WHEN cap_match THEN 550 ELSE 0 END,
           CASE WHEN name_trgm THEN 500 ELSE 0 END,
           CASE WHEN sig_sub THEN 320 ELSE 0 END,
           CASE WHEN sig_trgm THEN 300 ELSE 0 END,
           CASE WHEN path_trgm OR path_sub THEN 200 ELSE 0 END,
           CASE WHEN doc_sub THEN 170 ELSE 0 END,
           CASE WHEN doc_trgm THEN 150 ELSE 0 END
       )::float8 AS score
FROM matched
WHERE name_trgm OR name_sub OR cap_match OR sig_trgm OR sig_sub
   OR path_trgm OR path_sub OR doc_trgm OR doc_sub
ORDER BY score DESC, file, name
LIMIT $limit OFFSET $offset
"#;

pub struct SearchSymbolsTool(pub Arc<Db>, pub Option<Arc<SemanticHandle>>);

impl SearchSymbolsTool {
    pub fn new(db: Arc<Db>) -> Self {
        Self(db, None)
    }

    pub fn with_semantic(db: Arc<Db>, semantic: Arc<SemanticHandle>) -> Self {
        Self(db, Some(semantic))
    }

    async fn embed_query(semantic: &SemanticHandle, query: &str) -> Option<Vec<f32>> {
        let embedder = semantic.embedder.as_ref()?;
        let vectors = match embedder.embed(&[query.to_string()]).await {
            Ok(v) => v,
            Err(err) => {
                tracing::warn!(error = %err, "semantic query embedding failed, falling back to lexical search");
                return None;
            }
        };
        match vectors.into_iter().next() {
            Some(v) if !v.is_empty() => Some(v),
            _ => None,
        }
    }

    async fn semantic_query_embedding(&self, query: &str) -> Option<Vec<f32>> {
        let semantic = self.1.as_ref()?;
        if !semantic.is_ready() {
            return None;
        }
        if !semantic.available(&self.0).await {
            return None;
        }
        Self::embed_query(semantic, query).await
    }
}

pub const HYBRID_SEARCH_SQL: &str = r#"
WITH lexical AS (
    SELECT cs.*,
           regexp_replace(lower($query), '([.^$*+?()\[\]{}|\\-])', '\\\1', 'g') AS q_literal,
           lower(cs.name) % lower($query) AS name_trgm,
           strpos(lower(cs.name), lower($query)) > 0 AS name_sub,
           lower(coalesce(cs.signature, '')) % lower($query) AS sig_trgm,
           strpos(lower(coalesce(cs.signature, '')), lower($query)) > 0 AS sig_sub,
           lower(cs.file) % lower($query) AS path_trgm,
           strpos(lower(cs.file), lower($query)) > 0 AS path_sub,
           lower(coalesce(cs.docstring, '')) % lower($query) AS doc_trgm,
           strpos(lower(coalesce(cs.docstring, '')), lower($query)) > 0 AS doc_sub
    FROM code_symbols cs
    WHERE ($repo = '' OR cs.repo = $repo)
      AND ($version = '' OR cs.version = $version)
      AND ($kind = 'any' OR lower(cs.label) = $kind)
      AND ($path_prefix = '' OR starts_with(cs.file, $path_prefix))
), lexical_matched AS (
    SELECT l.*,
           lower(l.label) = 'class'
           AND l.source ~* ('(^|[^A-Za-z0-9_])' || l.q_literal || '([^A-Za-z0-9_]|$)') AS cap_match
    FROM lexical l
), lexical_scored AS (
    SELECT m.*,
           GREATEST(
               CASE WHEN m.name = $query THEN 1000 ELSE 0 END,
               CASE WHEN lower(m.name) = lower($query) THEN 900 ELSE 0 END,
               CASE WHEN m.name_sub THEN 600 ELSE 0 END,
               CASE WHEN m.cap_match THEN 550 ELSE 0 END,
               CASE WHEN m.name_trgm THEN 500 ELSE 0 END,
               CASE WHEN m.sig_sub THEN 320 ELSE 0 END,
               CASE WHEN m.sig_trgm THEN 300 ELSE 0 END,
               CASE WHEN m.path_trgm OR m.path_sub THEN 200 ELSE 0 END,
               CASE WHEN m.doc_sub THEN 170 ELSE 0 END,
               CASE WHEN m.doc_trgm THEN 150 ELSE 0 END
           )::float8 AS lexical_score
    FROM lexical_matched m
), semantic_scored AS (
    SELECT s.id,
           (1 - (s.embedding <=> $qvec::vector))::float8 AS semantic_score
    FROM symbols s
    WHERE s.embedding IS NOT NULL
      AND s.embedding_model = $qmodel
      AND ($repo = '' OR s.repo = $repo)
      AND ($version = '' OR s.version = $version)
)
SELECT ls.repo, ls.version, ls.file, ls.name, ls.start_line, lower(ls.label) AS kind,
       ls.signature, ls.source, ls.docstring,
       array_remove(ARRAY[
           CASE WHEN ls.name = $query                 THEN 'exact_name' END,
           CASE WHEN ls.name_sub                      THEN 'name' END,
           CASE WHEN ls.cap_match                     THEN 'capability' END,
           CASE WHEN ls.sig_sub OR ls.sig_trgm         THEN 'signature' END,
           CASE WHEN ls.path_trgm OR ls.path_sub       THEN 'path' END,
           CASE WHEN ls.doc_sub OR ls.doc_trgm         THEN 'docstring' END,
           CASE WHEN ss.semantic_score IS NOT NULL      THEN 'semantic' END
       ], NULL) AS matched_on,
       round((
           $lexical::float8 * (coalesce(ls.lexical_score, 0) / 1000.0)
         + $semantic::float8 * coalesce(ss.semantic_score, 0)
       )::numeric, 6)::float8 AS score
FROM lexical_scored ls
LEFT JOIN semantic_scored ss ON ss.id = ls.id
WHERE ls.lexical_score > 0 OR ss.semantic_score IS NOT NULL
ORDER BY score DESC, ls.file, ls.name
LIMIT $limit OFFSET $offset
"#;

#[derive(Clone)]
pub struct SemanticHandle {
    pub embedder: Option<Arc<dyn super::semantic::Embedder>>,
    pub model: String,
    pub lexical_weight: f64,
    pub semantic_weight: f64,
    availability: Arc<tokio::sync::OnceCell<bool>>,
}

impl Default for SemanticHandle {
    fn default() -> Self {
        Self {
            embedder: None,
            model: String::new(),
            lexical_weight: super::semantic::DEFAULT_LEXICAL_WEIGHT,
            semantic_weight: super::semantic::DEFAULT_SEMANTIC_WEIGHT,
            availability: Arc::new(tokio::sync::OnceCell::new()),
        }
    }
}

impl SemanticHandle {
    pub fn configured(embedder: Arc<dyn super::semantic::Embedder>, cfg: &crate::config::SemanticConfig) -> Self {
        let (lexical_weight, semantic_weight) = cfg.effective_weights();
        Self {
            embedder: Some(embedder),
            model: cfg.model.clone(),
            lexical_weight,
            semantic_weight,
            availability: Arc::new(tokio::sync::OnceCell::new()),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.embedder.is_some() && !self.model.is_empty()
    }

    pub async fn available(&self, db: &Db) -> bool {
        *self
            .availability
            .get_or_init(|| async { super::semantic::semantic_available(db).await.unwrap_or(false) })
            .await
    }
}

#[async_trait]
impl Tool for SearchSymbolsTool {

    fn definition(&self) -> ToolDefinition {
        search_symbols_definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let q = params["query"].as_str().unwrap_or("").trim().to_string();
        let path_prefix = params["path_prefix"].as_str().unwrap_or("").trim().to_string();
        if q.is_empty() && path_prefix.is_empty() {
            anyhow::bail!("search_symbols requires a non-empty query or a path_prefix");
        }
        let repo    = params["repo"].as_str().unwrap_or("").trim().to_string();
        let version = params["version"].as_str().unwrap_or("").trim().to_string();
        let kind    = params["kind"].as_str().unwrap_or("any").trim().to_string();
        let kind    = if kind.is_empty() { "any".to_string() } else { kind };
        let limit   = clamp_limit(&params);
        let offset  = params["offset"].as_i64().unwrap_or(0).max(0);

        // One extra row reveals whether another page exists.
        let mut params = json!({
            "query": q,
            "repo": repo,
            "version": version,
            "kind": kind,
            "path_prefix": path_prefix,
            "limit": limit as i64 + 1,
            "offset": offset,
        });

        let embed = if q.is_empty() { None } else { self.semantic_query_embedding(&q).await };
        let hybrid = match embed {
            Some(v) => {
                params["qvec"] = json!(v);
                params["qmodel"] = json!(self.1.as_ref().unwrap().model);
                params["lexical"] = json!(self.1.as_ref().unwrap().lexical_weight);
                params["semantic"] = json!(self.1.as_ref().unwrap().semantic_weight);
                Some(HYBRID_SEARCH_SQL)
            }
            None => None,
        };

        let mut rows = self.0.query(hybrid.unwrap_or(SEARCH_SQL), params).await?;

        if rows.is_empty() {
            let hint = if repo.is_empty() || version.is_empty() {
                " Pass repo and version to search a specific codebase."
            } else if offset > 0 {
                " There are no more results past this offset."
            } else {
                " Try a shorter fragment, a file path fragment, or a term from a class body."
            };
            let scope = if path_prefix.is_empty() { String::new() } else { format!(" under {path_prefix:?}") };
            return Ok(format!("No symbols found for {q:?}{scope}.{hint}"));
        }
        let has_more = rows.len() > limit;
        rows.truncate(limit);

        let mut results: Vec<Value> = Vec::with_capacity(rows.len() + 1);
        for row in &rows {
            let signature = row["signature"].as_str().unwrap_or_default();
            let source = row["source"].as_str().unwrap_or_default();
            let basis = if !signature.is_empty() { signature } else { source };
            results.push(json!({
                "repo": row["repo"],
                "version": row["version"],
                "file": row["file"],
                "name": row["name"],
                "start_line": row["start_line"],
                "kind": row["kind"],
                "matched_on": row["matched_on"],
                "score": row["score"],
                "preview": source_preview(basis, SEARCH_PREVIEW_CHARS),
            }));
        }
        if has_more {
            results.push(json!({
                "more_results": true,
                "next_offset": offset + limit as i64,
            }));
        }

        Ok(serde_json::to_string_pretty(&results)?)
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

static RUN_SQL_AVAILABLE: AtomicBool = AtomicBool::new(true);

/// Whether `run_sql` is offered to the model. Set once at startup from [`probe_run_sql`];
/// a tool that can only fail wastes a model round trip every time it is called.
pub fn run_sql_available() -> bool {
    RUN_SQL_AVAILABLE.load(Ordering::Relaxed)
}

pub fn set_run_sql_available(available: bool) {
    RUN_SQL_AVAILABLE.store(available, Ordering::Relaxed);
}

/// Checks that the database user can switch to the read-only role `run_sql` runs as. When it
/// cannot, logs the statements an administrator must run to enable it.
pub async fn probe_run_sql(db: &Db) -> bool {
    let probe = async {
        let tx = db.begin().await?;
        tx.batch_execute(&format!("SET LOCAL ROLE {GRAPH_READER_ROLE}")).await?;
        tx.rollback().await?;
        anyhow::Ok(())
    };
    match probe.await {
        Ok(()) => true,
        Err(e) => {
            let user = db.query("SELECT current_user AS u", json!({})).await.ok()
                .and_then(|rows| rows.first().and_then(|r| r["u"].as_str().map(String::from)))
                .unwrap_or_else(|| "<harvest database user>".to_string());
            tracing::error!(
                error = %e,
                "run_sql is disabled: the database user cannot switch to the read-only role {GRAPH_READER_ROLE}. \
                 To enable it, run as a PostgreSQL superuser and restart the server:\n\
                 CREATE ROLE {GRAPH_READER_ROLE} NOLOGIN;\n\
                 GRANT SELECT ON code_repositories, code_versions, code_files, code_symbols, code_imports, code_edges TO {GRAPH_READER_ROLE};\n\
                 GRANT {GRAPH_READER_ROLE} TO \"{user}\";"
            );
            false
        }
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

#[derive(Clone, Debug, PartialEq)]
pub struct AncestorVisit {
    pub root:  String,
    pub name:  String,
    pub file:  String,
    pub depth: usize,
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveCapability {
    pub class:      String,
    pub value:      Option<String>,
    pub defined_on: Option<String>,
    pub defined_in: Option<String>,
    pub depth:      Option<usize>,
    pub inherited:  bool,
    pub ambiguous:  bool,
}

pub fn validate_capability_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid_first = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_');
    let valid_rest = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid_first || !valid_rest {
        anyhow::bail!("{name:?} is not a valid capability name; pass a plain identifier such as SUPPORTS_ACTIVE_ACTIVE");
    }
    Ok(())
}

pub fn extract_capability_value(source: &str, capability: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix(capability) else { continue };
        let rest = rest.trim_start_matches([' ', '\t']);
        let assigned = match rest.strip_prefix(':') {
            Some(after_annotation) => match after_annotation.find('=') {
                Some(eq) => &after_annotation[eq + 1..],
                None => continue,
            },
            None => match rest.strip_prefix('=') {
                Some(assigned) => assigned,
                None => continue,
            },
        };
        let value = assigned.split('#').next().unwrap_or("").trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

pub fn resolve_effective_capabilities(visits: &[AncestorVisit]) -> Vec<EffectiveCapability> {
    let mut order: Vec<String> = Vec::new();
    for v in visits {
        if !order.contains(&v.root) {
            order.push(v.root.clone());
        }
    }
    order.iter().map(|root| {
        let mut candidates: Vec<&AncestorVisit> = visits.iter()
            .filter(|v| &v.root == root && v.value.is_some())
            .collect();
        candidates.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.name.cmp(&b.name)));
        let nearest_depth = candidates.first().map(|c| c.depth);
        let same_depth: Vec<&&AncestorVisit> = candidates.iter()
            .filter(|c| Some(c.depth) == nearest_depth)
            .collect();
        let ambiguous = same_depth.len() > 1;
        let chosen = if ambiguous { None } else { candidates.first().copied() };
        let (value, defined_on, defined_in, depth) = match chosen {
            Some(c) => (
                c.value.clone(),
                Some(c.name.clone()),
                Some(c.file.clone()),
                Some(c.depth),
            ),
            None => (None, None, None, None),
        };
        let inherited = depth.map(|d| d > 0).unwrap_or(false);
        EffectiveCapability {
            class: root.clone(),
            value,
            defined_on,
            defined_in,
            depth,
            inherited,
            ambiguous,
        }
    }).collect()
}

pub fn capability_names_from_params(params: &Value) -> Vec<String> {
    strings_from_param(params, &["capability", "capabilities"])
}

pub fn class_names_from_params(params: &Value) -> Vec<String> {
    strings_from_param(params, &["classes", "class"])
}

fn strings_from_param(params: &Value, keys: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in keys {
        let candidates: Vec<String> = match params.get(*key) {
            Some(Value::String(s)) => s.split(',').map(|p| p.trim().to_string()).collect(),
            Some(Value::Array(items)) => items.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .collect(),
            _ => Vec::new(),
        };
        for candidate in candidates {
            if !candidate.is_empty() && !out.contains(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

pub fn capability_matrix_definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_capability_matrix".into(),
        description: "Resolve the effective value of a capability across several classes, \
                      following inheritance so a class that inherits its value is reported with \
                      the ancestor that defines it. Returns one row per class per capability, \
                      including whether the value was inherited, which class declared it, and \
                      whether the declaring ancestor was ambiguous. Use this to answer whether \
                      each variant supports a capability instead of reading every class body."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "repo":       { "type": "string", "description": "Repository name" },
                "version":    { "type": "string", "description": "Version tag" },
                "classes": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Class names to resolve the capability for"
                },
                "capability": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ],
                    "description": "Capability identifier, e.g. SUPPORTS_ACTIVE_ACTIVE. One or more."
                }
            },
            "required": ["repo", "version", "classes", "capability"]
        }),
    }
}

/// One class reached by the capability walk, with what is needed to spot inheritance the
/// walk could not follow.
#[derive(Clone, Debug, PartialEq)]
pub struct WalkedClass {
    pub root:           String,
    pub name:           String,
    pub file:           String,
    pub depth:          usize,
    pub recorded_bases: Vec<String>,
    pub header:         String,
}

/// A break in the inheritance walk started from `root`.
#[derive(Clone, Debug, PartialEq)]
pub enum InheritanceGap {
    /// `class` declares parents in its source but the index recorded none.
    BasesNotRecorded { root: String, class: String },
    /// A recorded parent has no class definition in the indexed version.
    ParentNotIndexed { root: String, parent: String },
}

impl InheritanceGap {
    pub fn root(&self) -> &str {
        match self {
            Self::BasesNotRecorded { root, .. } | Self::ParentNotIndexed { root, .. } => root,
        }
    }
}

pub fn inheritance_gaps(walked: &[WalkedClass]) -> Vec<InheritanceGap> {
    let mut gaps = Vec::new();
    for class in walked {
        if class.recorded_bases.is_empty() {
            if class.file.ends_with(".py") && !python_declared_bases(&class.header).is_empty() {
                gaps.push(InheritanceGap::BasesNotRecorded { root: class.root.clone(), class: class.name.clone() });
            }
            continue;
        }
        if class.depth as i32 >= CAPABILITY_MAX_DEPTH {
            continue;
        }
        for base in &class.recorded_bases {
            if IMPLICIT_PYTHON_BASES.contains(&base.as_str()) {
                continue;
            }
            // Any visit under the same root counts: a cyclic or diamond hierarchy reaches a
            // parent once, possibly at a shallower depth.
            let reached = walked.iter().any(|w| w.root == class.root && &w.name == base);
            if !reached {
                gaps.push(InheritanceGap::ParentNotIndexed { root: class.root.clone(), parent: base.clone() });
            }
        }
    }
    gaps
}

/// The parent classes named in a Python class statement, e.g. `["driver.VolumeDriver"]` for
/// `class LVMVolumeDriver(driver.VolumeDriver):`. Keyword arguments such as `metaclass=` and
/// the implicit `object` base are left out.
pub fn python_declared_bases(source: &str) -> Vec<String> {
    let Some(class_at) = source.find("class ") else { return Vec::new() };
    let statement = &source[class_at..];
    let Some(colon) = statement.find(':') else { return Vec::new() };
    let head = &statement[..colon];
    let (Some(open), Some(close)) = (head.find('('), head.rfind(')')) else { return Vec::new() };
    if close <= open {
        return Vec::new();
    }
    let mut bases = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in head[open + 1..close].chars() {
        match c {
            '(' | '[' => { depth += 1; current.push(c); }
            ')' | ']' => { depth = depth.saturating_sub(1); current.push(c); }
            ',' if depth == 0 => { bases.push(std::mem::take(&mut current)); }
            _ => current.push(c),
        }
    }
    bases.push(current);
    bases.into_iter()
        .map(|b| b.split_whitespace().collect::<String>())
        .filter(|b| !b.is_empty() && !b.contains('=') && b != "object")
        .collect()
}

/// Bases that never carry a project's capability flags, so their absence from the index is
/// not worth a warning.
const IMPLICIT_PYTHON_BASES: &[&str] = &["object", "ABC", "Generic", "Protocol", "Exception", "BaseException"];

const CAPABILITY_HEADER_CHARS: i32 = 400;
const CAPABILITY_MAX_DEPTH: i32 = 12;
const CAPABILITY_WINDOW_CHARS: i32 = 600;
const CAPABILITY_WINDOW_LEAD: i32 = 300;

pub struct GetCapabilityMatrixTool(pub Arc<Db>);

#[async_trait]
impl Tool for GetCapabilityMatrixTool {
    fn definition(&self) -> ToolDefinition {
        capability_matrix_definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let repo    = params["repo"].as_str().unwrap_or("").trim().to_string();
        let version = params["version"].as_str().unwrap_or("").trim().to_string();
        let classes = class_names_from_params(&params);
        let capabilities = capability_names_from_params(&params);

        if repo.is_empty() || version.is_empty() {
            anyhow::bail!("get_capability_matrix requires repo and version");
        }
        if classes.is_empty() {
            anyhow::bail!("get_capability_matrix requires at least one class name in `classes`");
        }
        if capabilities.is_empty() {
            anyhow::bail!("get_capability_matrix requires at least one capability name in `capability`");
        }
        for name in &capabilities {
            validate_capability_name(name)?;
        }

        let mut report: Vec<Value> = Vec::new();
        let mut missing: Vec<String> = Vec::new();
        let mut bases_not_recorded: Vec<String> = Vec::new();
        let mut parents_not_indexed: Vec<String> = Vec::new();
        for capability in &capabilities {
            let rows = self.0.query(
                CAPABILITY_WALK_SQL,
                json!({
                    "repo": repo,
                    "version": version,
                    "classes": classes,
                    "capability": capability,
                    "max_depth": CAPABILITY_MAX_DEPTH,
                    "window": CAPABILITY_WINDOW_CHARS,
                    "lead": CAPABILITY_WINDOW_LEAD,
                    "header": CAPABILITY_HEADER_CHARS,
                }),
            ).await?;

            let mut visits: Vec<AncestorVisit> = Vec::new();
            let mut found: Vec<String> = Vec::new();
            let mut walked: Vec<WalkedClass> = Vec::new();
            for row in &rows {
                let root = row["root"].as_str().unwrap_or_default().to_string();
                if !found.contains(&root) {
                    found.push(root.clone());
                }
                let name = row["name"].as_str().unwrap_or_default().to_string();
                let file = row["file"].as_str().unwrap_or_default().to_string();
                let depth = row["depth"].as_i64().unwrap_or(0).max(0) as usize;
                let window = row["source_window"].as_str().unwrap_or_default();
                let value = extract_capability_value(window, capability);
                walked.push(WalkedClass {
                    root: root.clone(),
                    name: name.clone(),
                    file: file.clone(),
                    depth,
                    recorded_bases: row["bases"].as_array()
                        .map(|a| a.iter().filter_map(|b| b.as_str().map(String::from)).collect())
                        .unwrap_or_default(),
                    header: row["header"].as_str().unwrap_or_default().to_string(),
                });
                visits.push(AncestorVisit { root, name, file, depth, value });
            }
            let effective = resolve_effective_capabilities(&visits);
            // A gap only matters for a class whose value it leaves unresolved: a value found
            // on the class or a nearer ancestor wins regardless of what lies beyond the gap.
            let unresolved: Vec<&str> = effective.iter()
                .filter(|e| e.value.is_none())
                .map(|e| e.class.as_str())
                .collect();
            for gap in inheritance_gaps(&walked) {
                if !unresolved.contains(&gap.root()) {
                    continue;
                }
                match gap {
                    InheritanceGap::BasesNotRecorded { class, .. } => {
                        if !bases_not_recorded.contains(&class) { bases_not_recorded.push(class); }
                    }
                    InheritanceGap::ParentNotIndexed { parent, .. } => {
                        if !parents_not_indexed.contains(&parent) { parents_not_indexed.push(parent); }
                    }
                }
            }

            for class in &classes {
                if !found.contains(class) && !missing.contains(class) {
                    missing.push(class.clone());
                }
            }

            for effective in effective {
                report.push(json!({
                    "capability": capability,
                    "class": effective.class,
                    "value": effective.value,
                    "declared_by": effective.defined_on,
                    "declared_in": effective.defined_in,
                    "inherited": effective.inherited,
                    "inherited_from_depth": effective.depth,
                    "ambiguous": effective.ambiguous,
                }));
            }
        }

        if report.is_empty() {
            return Ok(format!(
                "No classes matched {classes:?} in {repo}:{version}. Check the class names with search_symbols."
            ));
        }
        if !missing.is_empty() {
            report.push(json!({
                "warning": "classes_not_found",
                "classes": missing,
                "detail": "these classes were not present in this repository version, so no value is reported for them",
            }));
        }
        if !bases_not_recorded.is_empty() {
            report.push(json!({
                "warning": "bases_not_recorded",
                "classes": bases_not_recorded,
                "detail": "these classes declare parent classes in their source, but the index recorded none, \
                           so inherited values could not be followed and a null value here means unknown, \
                           not unsupported. The repository version was probably indexed by an older \
                           harvester and needs re-ingesting; tell the user instead of guessing",
            }));
        }
        if !parents_not_indexed.is_empty() {
            report.push(json!({
                "warning": "parents_not_indexed",
                "parents": parents_not_indexed,
                "detail": "these parent classes are not defined in this repository version (for example an \
                           external library), so values they might declare are unknown",
            }));
        }
        Ok(serde_json::to_string_pretty(&report)?)
    }
}

const CAPABILITY_WALK_SQL: &str = r#"
WITH RECURSIVE walk AS (
    SELECT s.name AS root, s.id, s.name, s.file, 0 AS depth,
           ARRAY[s.id] AS visited, s.source, s.bases
    FROM code_symbols s
    WHERE s.repo = $repo AND s.version = $version
      AND s.label = 'Class'
      AND s.name = ANY($classes)
  UNION ALL
    SELECT w.root, p.id, p.name, p.file, w.depth + 1,
           w.visited || p.id, p.source, p.bases
    FROM walk w
    CROSS JOIN LATERAL unnest(
        (SELECT c.bases FROM code_symbols c WHERE c.id = w.id)
    ) AS b(base_name)
    JOIN code_symbols p
      ON p.repo = $repo
     AND p.version = $version
     AND p.label = 'Class'
     AND p.name = b.base_name
    WHERE w.depth < $max_depth
      AND NOT (p.id = ANY(w.visited))
)
SELECT root, name, file, depth, coalesce(bases, '{}') AS bases,
       left(source, $header::int) AS header,
       CASE
         WHEN strpos(source, $capability) > 0
         THEN substring(source FROM greatest(strpos(source, $capability) - $lead::int, 1) FOR $window::int)
         ELSE ''
       END AS source_window
FROM walk
ORDER BY root, depth, name
"#;

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

pub const MAX_SINGLE_TOOL_RESULT_CHARS: usize = 15_000;
pub const EVIDENCE_PACK_BUDGET_CHARS: usize = 13_500;
pub const EVIDENCE_MATRIX_MAX_CHARS: usize = 6_000;
pub const EVIDENCE_SYMBOL_MAX_CHARS: usize = 5_000;
pub const EVIDENCE_MAX_SYMBOLS: usize = 12;

pub fn evidence_pack_definition() -> ToolDefinition {
    ToolDefinition {
        name: "get_evidence_pack".into(),
        description: "Gather everything needed to answer a behavioural question in one call: \
                      the effective value of each capability across the given classes (resolved \
                      through inheritance), plus the source of the named symbols. Prefer this \
                      over reading class bodies one at a time. Every claim it returns is anchored \
                      to a class, a file and a line, so the answer can cite evidence directly."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "repo":       { "type": "string", "description": "Repository name" },
                "version":    { "type": "string", "description": "Version tag" },
                "classes": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ],
                    "description": "Classes whose capabilities should be resolved"
                },
                "capability": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ],
                    "description": "Capability identifiers, e.g. SUPPORTS_ACTIVE_ACTIVE. One or more."
                },
                "symbols": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ],
                    "description": "Symbol names to include as source evidence"
                },
                "max_symbols": { "type": "integer", "description": "Cap on symbols returned" }
            },
            "required": ["repo", "version"]
        }),
    }
}

fn truncate_to(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    let kept: String = text.chars().take(max).collect();
    (format!("{kept}\n[truncated]"), true)
}

fn render_evidence_pack(matrix: &Value, symbols: &[(&str, &str, &str)]) -> String {
    let mut sections: Vec<String> = Vec::new();

    let matrix_text = serde_json::to_string_pretty(matrix).unwrap_or_else(|_| "[]".into());
    let (matrix_text, matrix_truncated) = truncate_to(&matrix_text, EVIDENCE_MATRIX_MAX_CHARS);
    sections.push(format!("## capability_matrix\n{matrix_text}"));

    let mut symbol_lines: Vec<String> = Vec::new();
    let mut remaining = EVIDENCE_SYMBOL_MAX_CHARS;
    let mut symbols_truncated = false;
    for (name, file, source) in symbols {
        if source.is_empty() {
            symbol_lines.push(format!("### {name} ({file})\nnot found"));
            continue;
        }
        let (text, cut) = truncate_to(source, remaining);
        symbols_truncated |= cut;
        remaining = remaining.saturating_sub(text.chars().count());
        symbol_lines.push(format!("### {name} ({file})\n{text}"));
        if remaining == 0 {
            symbols_truncated = true;
            break;
        }
    }
    if !symbol_lines.is_empty() {
        sections.push(format!("## symbols\n{}", symbol_lines.join("\n\n")));
    }

    let mut out = sections.join("\n\n");
    if matrix_truncated || symbols_truncated {
        out.push_str("\n\n[truncated]");
    }
    if out.chars().count() > EVIDENCE_PACK_BUDGET_CHARS {
        out = out.chars().take(EVIDENCE_PACK_BUDGET_CHARS).collect();
        out.push_str("\n[truncated]");
    }
    out
}

pub struct GetEvidencePackTool(pub Arc<Db>);

#[async_trait]
impl Tool for GetEvidencePackTool {
    fn definition(&self) -> ToolDefinition {
        evidence_pack_definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let repo    = params["repo"].as_str().unwrap_or("").trim().to_string();
        let version = params["version"].as_str().unwrap_or("").trim().to_string();
        if repo.is_empty() || version.is_empty() {
            anyhow::bail!("get_evidence_pack requires repo and version");
        }

        let classes = class_names_from_params(&params);
        let capabilities = capability_names_from_params(&params);
        for name in &capabilities {
            validate_capability_name(name)?;
        }

        let mut matrix_rows: Vec<Value> = Vec::new();
        if !classes.is_empty() && !capabilities.is_empty() {
            let matrix = GetCapabilityMatrixTool(Arc::clone(&self.0));
            let rendered = matrix.execute(json!({
                "repo": repo,
                "version": version,
                "classes": classes,
                "capability": capabilities,
            })).await?;
            matrix_rows = serde_json::from_str::<Vec<Value>>(&rendered)
                .unwrap_or_else(|_| vec![json!({ "note": truncate_to(&rendered, EVIDENCE_MATRIX_MAX_CHARS).0 })]);
        }

        let mut symbols = SourceTargets::default();
        let wanted = strings_from_param(&params, &["symbols", "symbol", "names"]);
        if !wanted.is_empty() {
            symbols.names = wanted;
        }
        let max_symbols = clamp_limit_key(&params, "max_symbols").min(EVIDENCE_MAX_SYMBOLS);
        let sources = ReadSourcesTool(Arc::clone(&self.0));
        let mut fetched: Vec<(String, String, String)> = Vec::new();
        if max_symbols > 0 && !(symbols.files.is_empty() && symbols.names.is_empty()) {
            let rendered = sources.execute(json!({
                "repo": repo,
                "version": version,
                "files": symbols.files,
                "names": symbols.names,
                "limit": max_symbols,
            })).await?;
            if let Ok(parsed) = serde_json::from_str::<Vec<Value>>(&rendered) {
                for row in parsed {
                    let name = row["name"].as_str().unwrap_or_default().to_string();
                    let file = row["file"].as_str().unwrap_or_default().to_string();
                    let src  = row["source"].as_str().unwrap_or_default().to_string();
                    fetched.push((name, file, src));
                }
            }
        }

        let requested = if symbols.names.is_empty() { symbols.files.clone() } else { symbols.names.clone() };
        let refs: Vec<(&str, &str, &str)> = if fetched.is_empty() {
            requested.iter().map(|n| (n.as_str(), "", "")).collect()
        } else {
            fetched.iter().map(|(n, f, s)| (n.as_str(), f.as_str(), s.as_str())).collect()
        };

        Ok(render_evidence_pack(&Value::Array(matrix_rows), &refs))
    }
}

const SUBCLASSES_MAX_DEPTH: i32 = 12;
const SUBCLASSES_MAX_ROWS: usize = 400;

const SUBCLASSES_SQL: &str = r#"
WITH RECURSIVE sub AS (
    SELECT c.id, c.name, $class::text AS parent, 1 AS depth, ARRAY[c.id] AS visited
    FROM code_symbols c
    WHERE c.repo = $repo AND c.version = $version AND c.label = 'Class'
      AND $class = ANY(c.bases)
  UNION ALL
    SELECT c.id, c.name, s.name, s.depth + 1, s.visited || c.id
    FROM sub s
    JOIN code_symbols c
      ON c.repo = $repo AND c.version = $version AND c.label = 'Class'
     AND s.name = ANY(c.bases)
    WHERE s.depth < $max_depth
      AND NOT (c.id = ANY(s.visited))
), nearest AS (
    SELECT DISTINCT ON (id) id, parent, depth FROM sub ORDER BY id, depth
)
SELECT cs.name, cs.file, cs.start_line, n.parent, n.depth
FROM nearest n JOIN code_symbols cs ON cs.id = n.id
WHERE $path_prefix = '' OR starts_with(cs.file, $path_prefix)
ORDER BY cs.file, cs.start_line
"#;

pub fn find_subclasses_definition() -> ToolDefinition {
    ToolDefinition {
        name: "find_subclasses".into(),
        description: "List every class that inherits from a class, directly or through \
                      intermediate classes, in one call. Use it to enumerate the implementations \
                      of a plugin or driver interface (for example every subclass of a base driver \
                      class), optionally restricted to a directory with `path_prefix`. Returns \
                      each subclass's file, line, the parent it inherits through, and its depth."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "repo":        { "type": "string", "description": "Repository name" },
                "version":     { "type": "string", "description": "Version tag" },
                "class":       { "type": "string", "description": "Name of the base class, without module prefix" },
                "path_prefix": { "type": "string", "description": "Only subclasses whose file path starts with this (optional)" }
            },
            "required": ["repo", "version", "class"]
        }),
    }
}

pub struct FindSubclassesTool(pub Arc<Db>);

#[async_trait]
impl Tool for FindSubclassesTool {
    fn definition(&self) -> ToolDefinition {
        find_subclasses_definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        let repo    = params["repo"].as_str().unwrap_or("").trim().to_string();
        let version = params["version"].as_str().unwrap_or("").trim().to_string();
        let class   = params["class"].as_str().unwrap_or("").trim();
        // Bases are stored without their module prefix, so `driver.VolumeDriver` is `VolumeDriver`.
        let class   = class.rsplit('.').next().unwrap_or(class).to_string();
        let path_prefix = params["path_prefix"].as_str().unwrap_or("").trim().to_string();
        if repo.is_empty() || version.is_empty() || class.is_empty() {
            anyhow::bail!("find_subclasses requires repo, version and class");
        }

        let rows = self.0.query(SUBCLASSES_SQL, json!({
            "repo": repo,
            "version": version,
            "class": class,
            "path_prefix": path_prefix,
            "max_depth": SUBCLASSES_MAX_DEPTH,
        })).await?;

        if rows.is_empty() {
            return Ok(format!(
                "No subclasses of {class:?} found in {repo}:{version}{}. Check the class name with \
                 search_symbols; if the version is listed under stale_versions by list_repositories, \
                 its inheritance data is incomplete.",
                if path_prefix.is_empty() { String::new() } else { format!(" under {path_prefix:?}") },
            ));
        }

        let total = rows.len();
        let mut results: Vec<Value> = rows.iter().take(SUBCLASSES_MAX_ROWS).map(|row| json!({
            "name": row["name"],
            "file": row["file"],
            "line": row["start_line"],
            "parent": row["parent"],
            "depth": row["depth"],
        })).collect();
        if total > SUBCLASSES_MAX_ROWS {
            results.push(json!({
                "truncated": true,
                "total": total,
                "detail": "narrow the search with path_prefix to see the rest",
            }));
        }
        Ok(serde_json::to_string(&results)?)
    }
}

pub fn all_tools(db: Arc<Db>) -> Vec<Box<dyn Tool>> {
    all_tools_with_semantic(db, None)
}

pub fn all_tools_with_semantic(db: Arc<Db>, semantic: Option<Arc<SemanticHandle>>) -> Vec<Box<dyn Tool>> {
    let search = match semantic {
        Some(handle) => SearchSymbolsTool::with_semantic(Arc::clone(&db), handle),
        None => SearchSymbolsTool::new(Arc::clone(&db)),
    };
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ListRepositoriesTool(Arc::clone(&db))),
        Box::new(search),
        Box::new(GetSymbolSourceTool(Arc::clone(&db))),
        Box::new(GetFileSymbolsTool(Arc::clone(&db))),
        Box::new(FindCallersTool(Arc::clone(&db))),
        Box::new(FindCalleesTool(Arc::clone(&db))),
        Box::new(GetImportsTool(Arc::clone(&db))),
        Box::new(CompareSymbolAcrossVersionsTool(Arc::clone(&db))),
        Box::new(GetCapabilityMatrixTool(Arc::clone(&db))),
        Box::new(ReadSourcesTool(Arc::clone(&db))),
        Box::new(GetEvidencePackTool(Arc::clone(&db))),
        Box::new(FindSubclassesTool(Arc::clone(&db))),
    ];
    if run_sql_available() {
        tools.push(Box::new(RunSqlTool(db)));
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::reject_non_select;
    use super::*;

    #[test]
    fn python_declared_bases_reads_dotted_and_multiple_parents() {
        assert_eq!(python_declared_bases("class LVMVolumeDriver(driver.VolumeDriver):\n    pass"), vec!["driver.VolumeDriver"]);
        assert_eq!(
            python_declared_bases("class TatlinFCVolumeDriver(tatlin_common.TatlinCommonVolumeDriver,\n                           driver.FibreChannelDriver):\n"),
            vec!["tatlin_common.TatlinCommonVolumeDriver", "driver.FibreChannelDriver"],
        );
        assert_eq!(python_declared_bases("class BaseVD(object, metaclass=abc.ABCMeta):\n"), Vec::<String>::new());
        assert_eq!(python_declared_bases("class Plain:\n    x = 1"), Vec::<String>::new());
        assert_eq!(python_declared_bases("class G(Generic[T, U], Base):\n"), vec!["Generic[T,U]", "Base"]);
    }

    fn walked(root: &str, name: &str, depth: usize, file: &str, bases: &[&str], header: &str) -> WalkedClass {
        WalkedClass {
            root: root.into(), name: name.into(), file: file.into(), depth,
            recorded_bases: bases.iter().map(|b| b.to_string()).collect(), header: header.into(),
        }
    }

    #[test]
    fn inheritance_gaps_flag_unrecorded_bases_only_for_python_classes_with_parents() {
        let classes = vec![
            walked("LVM", "LVM", 0, "drivers/lvm.py", &[], "class LVM(driver.VolumeDriver):"),
            walked("Root", "Root", 0, "root.py", &[], "class Root(object):"),
            walked("Rs", "Rs", 0, "src/lib.rs", &[], "struct Rs(u8);"),
        ];
        assert_eq!(inheritance_gaps(&classes), vec![InheritanceGap::BasesNotRecorded { root: "LVM".into(), class: "LVM".into() }]);
    }

    #[test]
    fn inheritance_gaps_flag_parents_missing_from_the_walk() {
        let classes = vec![
            walked("A", "A", 0, "a.py", &["B", "External", "object"], "class A(B, External, object):"),
            walked("A", "B", 1, "b.py", &[], "class B:"),
        ];
        assert_eq!(inheritance_gaps(&classes), vec![InheritanceGap::ParentNotIndexed { root: "A".into(), parent: "External".into() }]);
    }

    #[test]
    fn accepts_select_statements() {
        for q in [
            "SELECT name FROM code_symbols",
            "  select path from code_files where path like '%test%'",
            "WITH x AS (SELECT 1) SELECT * FROM x",
            "(SELECT 1) UNION (SELECT 1)",
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

    fn visit(root: &str, name: &str, depth: usize, value: Option<&str>) -> AncestorVisit {
        AncestorVisit {
            root: root.into(),
            name: name.into(),
            file: format!("pkg/{}.py", name.to_lowercase()),
            depth,
            value: value.map(|v| v.to_string()),
        }
    }

    #[test]
    fn extract_capability_value_finds_class_level_assignment() {
        let src = "class NfsDriver(BaseDriver):\n    SUPPORTS_ACTIVE_ACTIVE = True\n    VERSION = '1.0'\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_ACTIVE_ACTIVE"), Some("True".into()));
    }

    #[test]
    fn extract_capability_value_handles_type_annotation() {
        let src = "class A:\n    SUPPORTS_X: bool = False\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_X"), Some("False".into()));
    }

    #[test]
    fn extract_capability_value_returns_none_when_absent() {
        let src = "class A:\n    VERSION = '1.0'\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_X"), None);
    }

    #[test]
    fn extract_capability_value_ignores_instance_attribute_assignment() {
        let src = "class A:\n    def __init__(self):\n        self.SUPPORTS_X = True\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_X"), None);
    }

    #[test]
    fn extract_capability_value_requires_full_name_match() {
        let src = "class A:\n    SUPPORTS_X_EXTRA = True\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_X"), None);
    }

    #[test]
    fn extract_capability_value_ignores_commented_out_assignment() {
        let src = "class A:\n    # SUPPORTS_X = True\n    OTHER = 1\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_X"), None);
    }

    #[test]
    fn extract_capability_value_strips_trailing_comment() {
        let src = "class A:\n    SUPPORTS_X = True  # only for one protocol\n";
        assert_eq!(extract_capability_value(src, "SUPPORTS_X"), Some("True".into()));
    }

    #[test]
    fn extract_capability_value_finds_collection_values() {
        let src = "class A:\n    PROTOCOLS = ['iscsi', 'fc']\n";
        assert_eq!(extract_capability_value(src, "PROTOCOLS"), Some("['iscsi', 'fc']".into()));
    }

    #[test]
    fn extract_capability_value_works_on_bounded_window() {
        let src = "class A:\n    SUPPORTS_X = True\n";
        let window: String = src.chars().skip(10).take(20).collect();
        assert_eq!(extract_capability_value(&window, "SUPPORTS_X"), Some("True".into()));
    }

    #[test]
    fn resolve_prefers_own_value_over_ancestor() {
        let visits = vec![
            visit("Leaf", "Leaf", 0, Some("True")),
            visit("Leaf", "Base", 1, Some("False")),
        ];
        let resolved = resolve_effective_capabilities(&visits);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].value.as_deref(), Some("True"));
        assert_eq!(resolved[0].defined_on.as_deref(), Some("Leaf"));
        assert_eq!(resolved[0].depth, Some(0));
        assert!(!resolved[0].inherited);
        assert!(!resolved[0].ambiguous);
    }

    #[test]
    fn resolve_falls_back_to_nearest_defining_ancestor() {
        let visits = vec![
            visit("Leaf", "Leaf", 0, None),
            visit("Leaf", "Middle", 1, Some("True")),
            visit("Leaf", "Base", 2, Some("False")),
        ];
        let resolved = resolve_effective_capabilities(&visits);
        assert_eq!(resolved[0].value.as_deref(), Some("True"));
        assert_eq!(resolved[0].defined_on.as_deref(), Some("Middle"));
        assert_eq!(resolved[0].depth, Some(1));
        assert!(resolved[0].inherited);
    }

    #[test]
    fn resolve_reports_undefined_when_no_ancestor_declares_it() {
        let visits = vec![
            visit("Leaf", "Leaf", 0, None),
            visit("Leaf", "Base", 1, None),
        ];
        let resolved = resolve_effective_capabilities(&visits);
        assert_eq!(resolved[0].value, None);
        assert_eq!(resolved[0].defined_on, None);
        assert!(!resolved[0].ambiguous);
    }

    #[test]
    fn resolve_returns_empty_for_no_visits() {
        assert!(resolve_effective_capabilities(&[]).is_empty());
    }

    #[test]
    fn resolve_flags_ambiguous_when_two_ancestors_at_same_depth_define_it() {
        let visits = vec![
            visit("Leaf", "Leaf", 0, None),
            visit("Leaf", "BaseA", 1, Some("True")),
            visit("Leaf", "BaseB", 1, Some("False")),
        ];
        let resolved = resolve_effective_capabilities(&visits);
        assert!(resolved[0].ambiguous, "two same-depth definitions must be flagged");
    }

    #[test]
    fn resolve_keeps_roots_in_first_seen_order() {
        let visits = vec![
            visit("B", "B", 0, Some("1")),
            visit("A", "A", 0, Some("2")),
        ];
        let resolved = resolve_effective_capabilities(&visits);
        assert_eq!(resolved[0].class, "B");
        assert_eq!(resolved[1].class, "A");
    }

    #[test]
    fn resolve_handles_matrix_of_several_roots() {
        let visits = vec![
            visit("Nfs", "Nfs", 0, Some("True")),
            visit("Iscsi", "Iscsi", 0, None),
            visit("Iscsi", "Base", 1, Some("False")),
        ];
        let resolved = resolve_effective_capabilities(&visits);
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].class, "Nfs");
        assert_eq!(resolved[0].value.as_deref(), Some("True"));
        assert_eq!(resolved[1].class, "Iscsi");
        assert_eq!(resolved[1].value.as_deref(), Some("False"));
        assert!(resolved[1].inherited);
    }

    #[test]
    fn capability_name_validation_accepts_identifiers() {
        assert!(validate_capability_name("SUPPORTS_ACTIVE_ACTIVE").is_ok());
        assert!(validate_capability_name("_private_flag").is_ok());
    }

    #[test]
    fn capability_name_validation_rejects_non_identifiers() {
        for bad in ["", "has space", "1LEADING", "drop; table", "a-b", "(x)"] {
            assert!(validate_capability_name(bad).is_err(), "should reject: {bad}");
        }
    }

    #[test]
    fn capability_matrix_definition_is_well_formed() {
        let def = capability_matrix_definition();
        assert_eq!(def.name, "get_capability_matrix");
        let params = &def.parameters;
        assert_eq!(params["type"], "object");
        for required in ["repo", "version", "classes", "capability"] {
            assert!(
                params["required"].as_array().unwrap().iter().any(|v| v == required),
                "{required} should be required"
            );
            assert!(params["properties"].get(required).is_some(), "{required} should have a schema");
        }
    }

    #[test]
    fn capability_matrix_definition_documents_inheritance() {
        let desc = capability_matrix_definition().description.to_lowercase();
        assert!(desc.contains("inherit"), "description should mention inheritance: {desc}");
        assert!(desc.contains("effective"), "description should mention effective values: {desc}");
    }

    #[test]
    fn capability_names_from_params_accepts_string_and_array() {
        let p = json!({ "capability": "FLAG_A" });
        assert_eq!(capability_names_from_params(&p), vec!["FLAG_A".to_string()]);

        let p = json!({ "capability": ["FLAG_A", "FLAG_B"] });
        assert_eq!(capability_names_from_params(&p), vec!["FLAG_A".to_string(), "FLAG_B".to_string()]);

        let p = json!({ "capabilities": ["FLAG_C"] });
        assert_eq!(capability_names_from_params(&p), vec!["FLAG_C".to_string()]);

        let p = json!({});
        assert!(capability_names_from_params(&p).is_empty());
    }

    #[test]
    fn class_names_from_params_accepts_string_and_array() {
        let p = json!({ "classes": "OnlyOne" });
        assert_eq!(class_names_from_params(&p), vec!["OnlyOne".to_string()]);

        let p = json!({ "class": "Alias" });
        assert_eq!(class_names_from_params(&p), vec!["Alias".to_string()]);

        let p = json!({ "classes": ["A", "B"] });
        assert_eq!(class_names_from_params(&p), vec!["A".to_string(), "B".to_string()]);

        let p = json!({});
        assert!(class_names_from_params(&p).is_empty());
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    #[test]
    fn search_definition_requests_limit_and_kind() {
        let def = search_symbols_definition();
        assert_eq!(def.name, "search_symbols");
        let props = &def.parameters["properties"];
        assert!(props.get("query").is_some());
        assert!(props.get("limit").is_some());
        assert_eq!(props["kind"]["enum"], json!(["function", "class", "any"]));
    }

    #[test]
    fn search_sql_matches_docstrings() {
        assert!(SEARCH_SQL.contains("docstring"), "search should match prose documentation");
    }

    #[test]
    fn search_sql_escapes_regex_metacharacters_in_the_query() {
        assert!(
            SEARCH_SQL.contains("regexp_replace"),
            "the capability matcher must not interpolate the raw query as a regex"
        );
    }

    #[test]
    fn search_sql_keeps_repo_version_kind_and_limit_filters() {
        for needle in ["$repo", "$version", "$kind", "LIMIT $limit"] {
            assert!(SEARCH_SQL.contains(needle), "missing filter: {needle}");
        }
        assert!(
            !SEARCH_SQL.contains("ELSE 100 * similarity(lower(name)"),
            "non-name matches must not be scored by name similarity alone"
        );
    }

    #[test]
    fn search_definition_mentions_capability_constants() {
        let desc = search_symbols_definition().description.to_lowercase();
        assert!(desc.contains("capabilit"), "should mention capability constants: {desc}");
    }

    #[test]
    fn search_preview_truncates_long_source_and_keeps_line_count() {
        let long = "x".repeat(5_000);
        let preview = source_preview(&long, 100);
        assert!(preview.chars().count() <= 140, "preview too long: {}", preview.chars().count());
        assert!(preview.contains('…'));
    }

    #[test]
    fn search_preview_leaves_short_source_untouched() {
        let short = "def f():\n    return 1\n";
        assert_eq!(source_preview(short, 400), short);
    }

    #[test]
    fn search_clamps_limit_into_range() {
        assert_eq!(clamp_limit(&json!({ "limit": 1 })), 1);
        assert_eq!(clamp_limit(&json!({ "limit": 25 })), 25);
        assert_eq!(clamp_limit(&json!({ "limit": 10_000 })), SEARCH_LIMIT_MAX);
        assert_eq!(clamp_limit(&json!({ "limit": 0 })), 1);
        assert_eq!(clamp_limit(&json!({})) , SEARCH_LIMIT_DEFAULT);
    }

    #[test]
    fn read_sources_definition_accepts_files_and_names() {
        let def = read_sources_definition();
        assert_eq!(def.name, "read_sources");
        let props = &def.parameters["properties"];
        assert!(props.get("files").is_some());
        assert!(props.get("names").is_some());
        assert_eq!(def.parameters["required"], json!(["repo", "version"]));
    }

    #[test]
    fn read_sources_definition_advertises_batching() {
        let desc = read_sources_definition().description.to_lowercase();
        assert!(desc.contains("single call"), "should encourage batching: {desc}");
    }

    #[test]
    fn source_targets_collects_files_and_symbol_names() {
        let params = json!({
            "files": ["a.py", "b.py"],
            "names": ["Foo", "Bar"],
            "symbols": ["Baz"],
        });
        let targets = source_targets(&params);
        assert_eq!(targets.files, vec!["a.py".to_string(), "b.py".to_string()]);
        assert_eq!(targets.names, vec!["Foo".to_string(), "Bar".to_string(), "Baz".to_string()]);
    }

    #[test]
    fn source_targets_tolerates_missing_keys() {
        let targets = source_targets(&json!({}));
        assert!(targets.files.is_empty());
        assert!(targets.names.is_empty());
    }

    #[test]
    fn source_targets_splits_comma_separated_values() {
        let targets = source_targets(&json!({ "files": "a.py, b.py" }));
        assert_eq!(targets.files, vec!["a.py".to_string(), "b.py".to_string()]);
    }

    #[test]
    fn source_budget_truncates_output_with_marker() {
        let out = format_sources_for_budget(&"z".repeat(5_000), 1_000);
        assert!(out.chars().count() < 5_000);
        assert!(out.contains("truncated"));
    }

    #[test]
    fn source_budget_leaves_small_output_untouched() {
        assert_eq!(format_sources_for_budget("small", 1_000), "small");
    }

    fn stub_handle() -> Arc<SemanticHandle> {
        Arc::new(SemanticHandle::configured(
            Arc::new(super::super::semantic::FnEmbedder::constant("text-embedding-004", 4, 0.1)),
            &crate::config::SemanticConfig::default(),
        ))
    }

    #[test]
    fn semantic_handle_is_not_ready_without_an_embedder() {
        assert!(!SemanticHandle::default().is_ready());
    }

    #[test]
    fn semantic_handle_is_ready_with_an_embedder_and_model() {
        assert!(stub_handle().is_ready());
    }

    #[test]
    fn semantic_handle_is_not_ready_when_the_model_is_blank() {
        let mut h = SemanticHandle::configured(Arc::new(super::super::semantic::FnEmbedder::constant("m", 4, 0.1)), &crate::config::SemanticConfig::default());
        h.model = String::new();
        assert!(!h.is_ready());
    }

    #[test]
    fn semantic_handle_uses_the_normalised_config_weights() {
        let cfg = crate::config::SemanticConfig { lexical_weight: 1.0, semantic_weight: 3.0, ..Default::default() };
        let h = SemanticHandle::configured(Arc::new(super::super::semantic::FnEmbedder::constant("m", 4, 0.1)), &cfg);
        assert!((h.lexical_weight - 0.25).abs() < 1e-9);
        assert!((h.semantic_weight - 0.75).abs() < 1e-9);
        assert_eq!(h.model, "text-embedding-004");
    }

    #[tokio::test]
    async fn search_stays_lexical_when_no_embedder_is_configured() {
        let tool = SearchSymbolsTool::new(Arc::new(Db::connect_without_migrating("postgres://127.0.0.1:1/none").unwrap()));
        assert!(tool.semantic_query_embedding("hello").await.is_none());
    }

    #[test]
    fn hybrid_sql_scores_cosine_similarity_against_the_query_vector() {
        assert!(HYBRID_SEARCH_SQL.contains("embedding <=> $qvec::vector"));
        assert!(
            HYBRID_SEARCH_SQL.contains(") AS cap_match"),
            "capability matching must be computed in its own CTE"
        );
        assert!(
            !HYBRID_SEARCH_SQL.contains("l.cap_match"),
            "a select-list alias cannot be reused in the same select list"
        );
        assert!(
            HYBRID_SEARCH_SQL.contains("CASE WHEN m.cap_match THEN 550 ELSE 0 END"),
            "the capability score must read the CTE column"
        );
        assert!(HYBRID_SEARCH_SQL.contains("1 - (s.embedding <=> $qvec::vector)"));
    }

    #[test]
    fn hybrid_sql_only_matches_vectors_from_the_requested_model() {
        assert!(HYBRID_SEARCH_SQL.contains("s.embedding_model = $qmodel"));
    }

    #[test]
    fn hybrid_sql_blends_both_weights() {
        assert!(HYBRID_SEARCH_SQL.contains("$lexical::float8"));
        assert!(HYBRID_SEARCH_SQL.contains("$semantic::float8"));
        assert!(HYBRID_SEARCH_SQL.contains("coalesce(ss.semantic_score, 0)"));
    }

    #[test]
    fn hybrid_sql_keeps_lexical_only_matches() {
        assert!(HYBRID_SEARCH_SQL.contains("WHERE ls.lexical_score > 0 OR ss.semantic_score IS NOT NULL"));
    }

    #[test]
    fn hybrid_sql_reports_the_semantic_signal() {
        assert!(HYBRID_SEARCH_SQL.contains("'semantic'"));
    }

    #[test]
    fn hybrid_sql_applies_the_same_filters_as_the_lexical_query() {
        for needle in ["$repo = ''", "$version = ''", "$kind = 'any'", "LIMIT $limit", "ORDER BY score DESC"] {
            assert!(HYBRID_SEARCH_SQL.contains(needle), "hybrid search is missing {needle}");
        }
    }

    #[test]
    fn hybrid_sql_shares_the_lexical_signal_definition() {
        for needle in ["q_literal", "name_trgm", "sig_sub", "doc_trgm", "cap_match"] {
            assert!(HYBRID_SEARCH_SQL.contains(needle), "hybrid search is missing {needle}");
        }
    }

    #[test]
    fn hybrid_sql_does_not_reference_the_undocumented_similarity_fallback() {
        assert!(!HYBRID_SEARCH_SQL.contains("ELSE 100 * similarity(lower(name)"));
    }

    #[test]
    fn availability_sql_asks_for_the_embedding_column_in_the_current_schema() {
        let sql = super::super::semantic::SEMANTIC_AVAILABILITY_SQL;
        assert!(sql.contains("table_name = 'symbols'"));
        assert!(sql.contains("column_name = 'embedding'"));
        assert!(sql.contains("current_schema()"));
    }

    #[tokio::test]
    async fn search_falls_back_to_lexical_when_the_embedder_fails() {
        let db = Arc::new(Db::connect_without_migrating("postgres://127.0.0.1:1/none").unwrap());
        let handle = Arc::new(SemanticHandle::configured(
            Arc::new(super::super::semantic::FnEmbedder::failing("text-embedding-004", 4)),
            &crate::config::SemanticConfig::default(),
        ));
        let tool = SearchSymbolsTool::with_semantic(db, handle);
        assert!(tool.semantic_query_embedding("hello").await.is_none());
    }

    #[tokio::test]
    async fn a_configured_embedder_produces_a_query_embedding() {
        let handle = SemanticHandle::configured(
            Arc::new(super::super::semantic::FnEmbedder::constant("text-embedding-004", 4, 0.25)),
            &crate::config::SemanticConfig::default(),
        );
        assert_eq!(SearchSymbolsTool::embed_query(&handle, "hello").await.unwrap(), vec![0.25; 4]);
    }

    #[tokio::test]
    async fn a_failing_embedder_produces_no_query_embedding() {
        let handle = SemanticHandle::configured(
            Arc::new(super::super::semantic::FnEmbedder::failing("text-embedding-004", 4)),
            &crate::config::SemanticConfig::default(),
        );
        assert!(SearchSymbolsTool::embed_query(&handle, "hello").await.is_none());
    }

    #[test]
    fn a_constant_embedder_returns_one_vector_per_input() {
        let e = super::super::semantic::FnEmbedder::constant("m", 3, 1.0);
        let out = futures::executor::block_on(super::super::semantic::Embedder::embed(&e, &["a".to_string(), "b".to_string()])).unwrap();
        assert_eq!(out, vec![vec![1.0; 3], vec![1.0; 3]]);
        assert_eq!(super::super::semantic::Embedder::dimensions(&e), 3);
        assert_eq!(super::super::semantic::Embedder::model(&e), "m");
    }

    #[test]
    fn a_failing_embedder_surfaces_its_error() {
        let e = super::super::semantic::FnEmbedder::failing("m", 3);
        let err = futures::executor::block_on(super::super::semantic::Embedder::embed(&e, &["a".to_string()])).unwrap_err().to_string();
        assert!(err.contains("unavailable"), "unexpected error: {err}");
    }

    #[test]
    fn fallback_chain_is_ordered_by_descending_score() {
        let chain = fallback_chain();
        assert_eq!(
            chain,
            vec![
                "exact_name",
                "exact_name_ci",
                "name",
                "capability",
                "name_fuzzy",
                "sig_sub",
                "signature",
                "path",
                "doc_sub",
                "docstring",
            ]
        );
    }

    #[test]
    fn fallback_chain_is_strictly_descending() {
        let chain = fallback_chain();
        let scores: Vec<i64> = chain
            .iter()
            .map(|signal| {
                SEARCH_RANKING
                    .iter()
                    .find(|(n, _)| n == signal)
                    .map(|(_, s)| *s)
                    .unwrap_or_else(|| panic!("{signal} is missing from SEARCH_RANKING"))
            })
            .collect();
        for pair in scores.windows(2) {
            assert!(pair[0] > pair[1], "the chain must be strictly descending, got {pair:?}");
        }
    }

    #[test]
    fn fallback_chain_covers_every_ranking_signal() {
        let chain: Vec<&str> = fallback_chain().to_vec();
        let ranking: Vec<&str> = SEARCH_RANKING.iter().map(|(n, _)| *n).collect();
        assert_eq!(chain, ranking, "the chain and the ranking must stay in sync");
    }

    #[test]
    fn search_ranking_scores_match_the_sql_weights() {
        for sql in [SEARCH_SQL, HYBRID_SEARCH_SQL] {
            for (signal, score) in SEARCH_RANKING {
                assert!(
                    sql.contains(&format!("{score} ELSE 0 END")) || sql.contains(&format!("{score},")),
                    "ranking weight {score} for {signal} must appear in the SQL"
                );
            }
        }
    }
}

#[cfg(test)]
mod evidence_pack_tests {
    use super::*;

    #[test]
    fn evidence_pack_definition_advertises_single_call_use() {
        let d = evidence_pack_definition();
        assert_eq!(d.name, "get_evidence_pack");
        let text = d.description.to_lowercase();
        assert!(text.contains("one call"), "should promise a single call: {}", d.description);
        assert!(text.contains("capabilit"), "should mention capability resolution");
        assert!(text.contains("evidence"), "should mention evidence/citations");
    }

    #[test]
    fn evidence_pack_requires_repo_and_version() {
        let d = evidence_pack_definition();
        let required = d.parameters["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "repo"));
        assert!(required.iter().any(|v| v == "version"));
        assert!(!required.iter().any(|v| v == "capability"),
            "capability should be optional");
    }

    #[test]
    fn evidence_pack_declares_capability_and_symbol_params() {
        let d = evidence_pack_definition();
        let props = &d.parameters["properties"];
        assert!(props.get("capability").is_some(), "capability param missing");
        assert!(props.get("symbols").is_some(), "symbols param missing");
        assert!(props.get("max_symbols").is_some(), "max_symbols param missing");
    }

    #[test]
    fn evidence_pack_budget_is_bounded() {
        let budget = std::hint::black_box(EVIDENCE_PACK_BUDGET_CHARS);
        let cap = std::hint::black_box(MAX_SINGLE_TOOL_RESULT_CHARS);
        assert!(budget > 0);
        assert!(budget <= cap, "an evidence pack must fit inside a single tool result");
    }

    #[test]
    fn evidence_pack_sections_are_bounded_individually() {
        let matrix = std::hint::black_box(EVIDENCE_MATRIX_MAX_CHARS);
        let symbols = std::hint::black_box(EVIDENCE_SYMBOL_MAX_CHARS);
        let budget = std::hint::black_box(EVIDENCE_PACK_BUDGET_CHARS);
        assert!(matrix > 0);
        assert!(symbols > 0);
        assert!(matrix + symbols <= budget, "sections must fit in the pack budget");
    }

    #[test]
    fn evidence_pack_render_truncates_and_keeps_all_sections() {
        let big = "x".repeat(50_000);
        let out = render_evidence_pack(
            &serde_json::json!([{ "class": "A", "capability": "C", "value": big }]),
            &[("helper", "src/a.py", &big)],
        );
        assert!(out.contains("capability_matrix"), "matrix section missing");
        assert!(out.contains("helper"), "symbol section missing");
        assert!(out.len() <= EVIDENCE_PACK_BUDGET_CHARS + 400, "pack too large: {}", out.len());
        assert!(out.contains("truncated"), "truncation must be signalled");
    }

    #[test]
    fn evidence_pack_render_keeps_short_output_verbatim() {
        let out = render_evidence_pack(
            &serde_json::json!([{ "class": "A", "value": true }]),
            &[("helper", "src/a.py", "def helper():\n    return 1")],
        );
        assert!(out.contains("def helper():\n    return 1"));
        assert!(!out.contains("truncated"), "small packs must not be marked truncated");
    }

    #[test]
    fn evidence_pack_marks_missing_symbols() {
        let out = render_evidence_pack(
            &serde_json::json!([]),
            &[("ghost", "src/nope.py", "")],
        );
        assert!(out.contains("ghost"), "requested symbol should still be listed");
        assert!(out.to_lowercase().contains("not found"), "missing symbol must be reported");
    }

    #[test]
    fn evidence_pack_handles_no_symbols_requested() {
        let out = render_evidence_pack(&serde_json::json!([{ "class": "A" }]), &[]);
        assert!(out.contains("capability_matrix"));
    }
}

#[cfg(test)]
mod clamp_key_tests {
    use super::*;

    #[test]
    fn clamp_limit_key_reads_an_alternate_key() {
        let p = json!({ "max_symbols": 3 });
        assert_eq!(clamp_limit_key(&p, "max_symbols"), 3);
        assert_eq!(clamp_limit_key(&p, "limit"), SEARCH_LIMIT_DEFAULT);
    }

    #[test]
    fn clamp_limit_reads_a_bare_number_without_double_wrapping() {
        let p = json!({ "limit": 2 });
        assert_eq!(clamp_limit(&p), 2);
        assert_eq!(clamp_limit(&json!({ "limit": 0 })), 1);
        assert_eq!(clamp_limit(&json!({ "limit": -5 })), 1);
        assert_eq!(clamp_limit(&json!({})), SEARCH_LIMIT_DEFAULT);
    }

    #[test]
    fn clamp_limit_never_exceeds_the_maximum() {
        assert_eq!(clamp_limit(&json!({ "limit": 1_000_000 })), SEARCH_LIMIT_MAX);
    }
}
