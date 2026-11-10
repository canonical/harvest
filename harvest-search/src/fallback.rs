use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use harvest_db::Db;
use serde_json::{json, Value};

use crate::deadline::Deadline;
use crate::domain::{Evidence, EvidenceKey, Query, SearchError, SearchResult, Source, TierId};
use crate::engine::SearchEngine;
use crate::index::tokenizer::{identifiers, query_tokens};
use crate::trace::Trace;

const PER_TERM: i64 = 10;
const MAX_TERMS: usize = 5;
const CHARS_PER_TOKEN: usize = 4;

pub const SEARCH_SQL: &str = r#"
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
      AND ($decorator = '' OR EXISTS (
            SELECT 1 FROM unnest(cs.decorators) d
            WHERE d = $decorator OR right(d, length($decorator) + 1) = '.' || $decorator))
), cap_candidates AS (
    SELECT s.id
    FROM symbols s
    WHERE s.label = 'Class'
      AND lower(s.source) LIKE '%' || replace(replace(replace(lower($query), '\', '\\'), '%', '\%'), '_', '\_') || '%'
), matched AS (
    SELECT s.*,
           s.id IN (SELECT id FROM cap_candidates)
           AND s.source ~* ('(^|[^A-Za-z0-9_])' || s.q_literal || '([^A-Za-z0-9_]|$)') AS cap_match
    FROM signals s
)
SELECT repo, version, file, name, start_line, end_line, impl_type, lower(label) AS kind,
       signature, source, docstring, coalesce(decorators, '{}') AS decorators,
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

pub const SEMANTIC_CANDIDATES: i64 = 200;

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
      AND ($decorator = '' OR EXISTS (
            SELECT 1 FROM unnest(cs.decorators) d
            WHERE d = $decorator OR right(d, length($decorator) + 1) = '.' || $decorator))
), cap_candidates AS (
    SELECT s.id
    FROM symbols s
    WHERE s.label = 'Class'
      AND lower(s.source) LIKE '%' || replace(replace(replace(lower($query), '\', '\\'), '%', '\%'), '_', '\_') || '%'
), lexical_matched AS (
    SELECT l.*,
           l.id IN (SELECT id FROM cap_candidates)
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
    JOIN versions v     ON v.id = s.version_id
    JOIN repositories r ON r.id = v.repository_id
    WHERE s.embedding IS NOT NULL
      AND s.embedding_model = $qmodel
      AND ($repo = '' OR r.name = $repo)
      AND ($version = '' OR v.tag = $version)
    ORDER BY s.embedding <=> $qvec::vector
    LIMIT $semantic_k
)
SELECT ls.repo, ls.version, ls.file, ls.name, ls.start_line, lower(ls.label) AS kind,
       ls.signature, ls.source, ls.docstring, coalesce(ls.decorators, '{}') AS decorators,
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

pub struct PostgresEngine {
    db: Arc<Db>,
}

impl PostgresEngine {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }
}

pub fn fallback_terms(text: &str) -> Vec<String> {
    let mut terms = identifiers(text);
    for token in query_tokens(text) {
        if terms.len() >= MAX_TERMS {
            break;
        }
        if token.len() >= 4 && !terms.iter().any(|t| t.to_lowercase().contains(&token)) {
            terms.push(token);
        }
    }
    terms.truncate(MAX_TERMS);
    terms
}

fn row_evidence(row: &Value) -> Evidence {
    let name = row["name"].as_str().unwrap_or_default();
    let qualified = match row["impl_type"].as_str().filter(|p| !p.is_empty()) {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_string(),
    };
    let file = row["file"].as_str().unwrap_or_default().to_string();
    let start_line = row["start_line"].as_i64().unwrap_or(0).max(0) as u32;
    let end_line = row["end_line"].as_i64().unwrap_or(0).max(0) as u32;
    let kind = row["kind"].as_str().unwrap_or("function");
    Evidence {
        label: format!("{kind} {qualified} ({file})"),
        key: EvidenceKey::Symbol {
            file: file.clone(),
            name: qualified.clone(),
        },
        unit: None,
        start_line,
        end_line,
        text: format!(
            "{qualified} — {file}:{start_line}-{end_line}\n{}",
            row["source"].as_str().unwrap_or_default()
        ),
        score: row["score"].as_f64().unwrap_or(0.0) as f32,
        found_by: Source::Fallback,
        covers: Vec::new(),
    }
}

#[async_trait]
impl SearchEngine for PostgresEngine {
    async fn search(
        &self,
        query: &Query,
        _deadline: Deadline,
    ) -> Result<SearchResult, SearchError> {
        let started = Instant::now();
        let mut found: Vec<Evidence> = Vec::new();
        let mut seen: HashSet<EvidenceKey> = HashSet::new();
        for term in fallback_terms(&query.text) {
            let rows = self
                .db
                .query(
                    SEARCH_SQL,
                    json!({
                        "query": term, "repo": query.scope.repo, "version": query.scope.version,
                        "kind": "any", "path_prefix": "", "decorator": "",
                        "limit": PER_TERM, "offset": 0,
                    }),
                )
                .await?;
            for row in rows {
                let evidence = row_evidence(&row);
                if seen.insert(evidence.key.clone()) {
                    found.push(evidence);
                }
            }
        }
        found.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut remaining = query.budget.0 * CHARS_PER_TOKEN;
        found.retain(|e| {
            let fits = remaining >= e.text.len().min(remaining.max(1));
            remaining = remaining.saturating_sub(e.text.len());
            fits && remaining > 0
        });
        let mut trace = Trace::default();
        trace.record("fallback.search", started.elapsed());
        trace.total_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(SearchResult {
            evidence: found,
            gaps: Vec::new(),
            tier_reached: TierId::Fallback,
            degraded: None,
            generation: None,
            trace,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_prefer_identifiers_then_content_words() {
        assert_eq!(
            fallback_terms("Where is `retry_limit` read by the volume manager?"),
            vec!["retry_limit", "read", "volume", "manager"]
        );
    }

    #[test]
    fn rows_become_evidence_keyed_by_qualified_name() {
        let evidence = row_evidence(&json!({
            "name": "push", "impl_type": "Stack", "file": "a.rs", "start_line": 3, "end_line": 9,
            "kind": "function", "source": "fn push() {}", "score": 600.0
        }));
        assert_eq!(
            evidence.key,
            EvidenceKey::Symbol {
                file: "a.rs".into(),
                name: "Stack.push".into()
            }
        );
        assert_eq!(evidence.found_by, Source::Fallback);
    }
}
