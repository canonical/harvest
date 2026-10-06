use anyhow::{anyhow, bail, Result};
use std::sync::Arc;
use serde_json::{json, Value};

pub const SEMANTIC_DIMENSIONS: usize = 768;
pub const SEMANTIC_MODEL: &str = "text-embedding-004";
pub const BACKFILL_BATCH: usize = 64;
pub const SOURCE_EXCERPT_CHARS: usize = 600;
pub const DEFAULT_LEXICAL_WEIGHT: f64 = 0.6;
pub const DEFAULT_SEMANTIC_WEIGHT: f64 = 0.4;

#[async_trait::async_trait]
pub trait Embedder: Send + Sync {
    fn model(&self) -> &str;
    fn dimensions(&self) -> usize;
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>>;
}

pub fn embedding_text(name: &str, signature: Option<&str>, docstring: Option<&str>, source: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !name.trim().is_empty() {
        parts.push(name.trim().to_string());
    }
    if let Some(sig) = signature {
        if !sig.trim().is_empty() {
            parts.push(sig.trim().to_string());
        }
    }
    if let Some(doc) = docstring {
        if !doc.trim().is_empty() {
            parts.push(doc.trim().to_string());
        }
    }
    if parts.len() <= 1 {
        if let Some(src) = source {
            let excerpt: String = src.trim().chars().take(SOURCE_EXCERPT_CHARS).collect();
            if !excerpt.is_empty() {
                parts.push(excerpt);
            }
        }
    }
    parts.join("\n")
}

pub fn to_pgvector(values: &[f32]) -> Result<String> {
    if values.is_empty() {
        bail!("cannot build a vector literal from an empty slice");
    }
    if !values.iter().all(|v| v.is_finite()) {
        bail!("embedding contains a non-finite value");
    }
    let body: Vec<String> = values.iter().map(|v| format!("{v}")).collect();
    Ok(format!("[{}]", body.join(",")))
}

pub fn hybrid_weights(lexical_weight: f64, semantic_weight: f64) -> Result<(f64, f64)> {
    if !lexical_weight.is_finite() || !semantic_weight.is_finite() {
        bail!("hybrid weights must be finite");
    }
    if lexical_weight < 0.0 || semantic_weight < 0.0 {
        bail!("hybrid weights must not be negative");
    }
    let total = lexical_weight + semantic_weight;
    if total <= 0.0 {
        bail!("hybrid weights must not both be zero");
    }
    Ok((lexical_weight / total, semantic_weight / total))
}

pub fn source_hash(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn chunks<T: Clone>(items: &[T], size: usize) -> Vec<Vec<T>> {
    if size == 0 || items.is_empty() {
        return Vec::new();
    }
    items.chunks(size).map(|c| c.to_vec()).collect()
}

pub fn pending_count(rows: usize, already: usize) -> usize {
    rows.saturating_sub(already)
}

pub fn hybrid_score_sql(lexical_weight: f64, semantic_weight: f64) -> Result<String> {
    let (lex, sem) = hybrid_weights(lexical_weight, semantic_weight)?;
    Ok(format!("({lex}::float8 * $lexical + {sem}::float8 * $semantic)"))
}

pub fn semantic_params(query_embedding: &[f32], model: &str, lexical: f64, semantic: f64) -> Result<Value> {
    Ok(json!({
        "qvec": to_pgvector(query_embedding)?,
        "qmodel": model,
        "lexical": lexical,
        "semantic": semantic,
    }))
}

pub fn backfill_batch_limit(configured: Option<usize>) -> usize {
    match configured {
        Some(n) if n > 0 => n.min(BACKFILL_BATCH),
        _ => BACKFILL_BATCH,
    }
}

pub struct GeminiEmbedder {
    provider: std::sync::Arc<crate::llm::gemini::GeminiProvider>,
    model: String,
    dimensions: usize,
}

impl GeminiEmbedder {
    pub fn new(
        provider: std::sync::Arc<crate::llm::gemini::GeminiProvider>,
        model: impl Into<String>,
        dimensions: usize,
    ) -> Self {
        Self { provider, model: model.into(), dimensions }
    }

    pub fn from_config(provider: std::sync::Arc<crate::llm::gemini::GeminiProvider>, cfg: &crate::config::SemanticConfig) -> Result<Self> {
        if cfg.dimensions == 0 {
            bail!("semantic dimensions must be greater than zero");
        }
        Ok(Self::new(provider, cfg.model.clone(), cfg.dimensions))
    }
}

#[async_trait::async_trait]
impl Embedder for GeminiEmbedder {
    fn model(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        let vectors = self
            .provider
            .embed(inputs, &self.model, Some(self.dimensions))
            .await?;
        for v in &vectors {
            if v.len() != self.dimensions {
                bail!("expected {} embedding dimensions, received {}", self.dimensions, v.len());
            }
        }
        Ok(vectors)
    }
}

pub struct FnEmbedder {
    model: String,
    dimensions: usize,
    #[allow(clippy::type_complexity)]
    embed: Box<dyn Fn(&[String]) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Vec<f32>>>> + Send + Sync>> + Send + Sync>,
}

impl FnEmbedder {
    pub fn new<F>(model: impl Into<String>, dimensions: usize, f: F) -> Self
    where
        F: Fn(&[String]) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Vec<f32>>>> + Send + Sync>> + Send + Sync + 'static,
    {
        Self { model: model.into(), dimensions, embed: Box::new(f) }
    }

    pub fn failing(model: impl Into<String>, dimensions: usize) -> Self {
        Self::new(model, dimensions, |_| Box::pin(async { bail!("embedder unavailable") }))
    }

    pub fn constant(model: impl Into<String>, dimensions: usize, value: f32) -> Self {
        Self::new(model, dimensions, move |inputs| {
            let vectors: Vec<Vec<f32>> = inputs.iter().map(|_| vec![value; dimensions]).collect();
            Box::pin(async move { Ok(vectors) })
        })
    }
}

#[async_trait::async_trait]
impl Embedder for FnEmbedder {
    fn model(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        (self.embed)(inputs).await
    }
}

pub const SEMANTIC_AVAILABILITY_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1
    FROM information_schema.columns
    WHERE table_schema = current_schema()
      AND table_name = 'symbols'
      AND column_name = 'embedding'
) AS present
"#;

pub const BACKFILL_CANDIDATE_SQL: &str = r#"
SELECT id, name, signature, docstring, file, source, embedding_model, embedding_source_hash
FROM symbols
WHERE ($repo = '' OR repo = $repo)
  AND ($version = '' OR version = $version)
ORDER BY id
LIMIT $limit
"#;

pub const BACKFILL_UPDATE_SQL: &str = r#"
UPDATE symbols
SET embedding = $embedding::vector,
    embedding_model = $model,
    embedding_source_hash = $hash,
    embedded_at = now()
WHERE id = $id
"#;

pub const PENDING_COUNT_SQL: &str = r#"
SELECT count(*)::bigint AS pending
FROM symbols
WHERE ($repo = '' OR repo = $repo)
  AND ($version = '' OR version = $version)
  AND (embedding IS NULL OR embedding_model IS DISTINCT FROM $model)
"#;

pub const HNSW_INDEX_EXISTS_SQL: &str = r#"
SELECT EXISTS (
    SELECT 1
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relname = 'idx_symbols_embedding_hnsw'
      AND n.nspname = current_schema()
) AS present
"#;

pub const HNSW_INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_symbols_embedding_hnsw
ON symbols
USING hnsw (embedding vector_cosine_ops)
"#;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BackfillStats {
    pub scanned: usize,
    pub embedded: usize,
    pub skipped: usize,
    pub batches: usize,
}

impl BackfillStats {
    pub fn merged(&mut self, other: BackfillStats) {
        self.scanned += other.scanned;
        self.embedded += other.embedded;
        self.skipped += other.skipped;
        self.batches += other.batches;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRow {
    pub id: i64,
    pub text: String,
    pub stored_model: Option<String>,
    pub stored_hash: Option<String>,
}

pub fn backfill_text(row_name: &str, signature: Option<&str>, docstring: Option<&str>, source: Option<&str>) -> String {
    embedding_text(row_name, signature, docstring, source)
}

pub fn needs_embedding(row: &BackfillRow, model: &str) -> bool {
    if row.text.trim().is_empty() {
        return false;
    }
    if row.stored_model.as_deref() != Some(model) {
        return true;
    }
    row.stored_hash.as_deref() != Some(source_hash(&row.text).as_str())
}

pub fn plan_batch(rows: &[BackfillRow], model: &str) -> (Vec<BackfillRow>, usize) {
    let mut pending = Vec::new();
    let mut skipped = 0;
    for row in rows {
        if needs_embedding(row, model) {
            pending.push(row.clone());
        } else {
            skipped += 1;
        }
    }
    (pending, skipped)
}

pub fn should_build_hnsw(pending: usize, embedded: usize, index_already_exists: bool) -> bool {
    embedded > 0 && pending == 0 && !index_already_exists
}

pub async fn pending_rows(db: &harvest_db::Db, model: &str, repo: &str, version: &str) -> Result<usize> {
    let rows = db
        .query(
            PENDING_COUNT_SQL,
            serde_json::json!({ "model": model, "repo": repo, "version": version }),
        )
        .await?;
    Ok(rows
        .first()
        .and_then(|r| r["pending"].as_i64())
        .unwrap_or_default()
        .max(0) as usize)
}

pub async fn hnsw_index_exists(db: &harvest_db::Db) -> Result<bool> {
    let rows = db.query(HNSW_INDEX_EXISTS_SQL, serde_json::json!({})).await?;
    Ok(rows
        .first()
        .and_then(|r| r["present"].as_bool())
        .unwrap_or(false))
}

#[derive(Debug, Default, Clone)]
pub struct BackfillOptions {
    pub model: String,
    pub batch_size: usize,
    pub repo: String,
    pub version: String,
    pub max_rows: Option<usize>,
    pub build_index_when_complete: bool,
}

impl BackfillOptions {
    pub fn from_config(cfg: &crate::config::SemanticConfig) -> Self {
        Self {
            model: cfg.model.clone(),
            batch_size: cfg.effective_batch_size(),
            repo: String::new(),
            version: String::new(),
            max_rows: None,
            build_index_when_complete: true,
        }
    }
}

pub async fn semantic_available(db: &harvest_db::Db) -> Result<bool> {
    let rows = db
        .query(SEMANTIC_AVAILABILITY_SQL, serde_json::json!({}))
        .await
        .map_err(|e| anyhow!("could not determine whether the embedding column exists: {e}"))?;
    Ok(rows
        .first()
        .and_then(|r| r["present"].as_bool())
        .unwrap_or(false))
}

pub async fn backfill_embeddings(
    db: &harvest_db::Db,
    embedder: &dyn Embedder,
    opts: &BackfillOptions,
) -> Result<BackfillStats> {
    if opts.model.trim().is_empty() {
        bail!("backfill requires an embedding model");
    }
    if !semantic_available(db).await? {
        bail!("embedding storage is not available; the pgvector migration has not been applied");
    }
    let batch_size = backfill_batch_limit(Some(opts.batch_size));
    let mut total = BackfillStats::default();
    let mut scanned_rows = 0usize;

    loop {
        let remaining = match opts.max_rows {
            Some(max) => max.saturating_sub(scanned_rows),
            None => batch_size,
        };
        if remaining == 0 {
            break;
        }
        let page = db
            .query(
                BACKFILL_CANDIDATE_SQL,
                serde_json::json!({
                    "repo": opts.repo,
                    "version": opts.version,
                    "limit": batch_size.min(remaining) as i64,
                }),
            )
            .await?;
        if page.is_empty() {
            break;
        }
        scanned_rows += page.len();

        let rows: Vec<BackfillRow> = page
            .iter()
            .map(|r| BackfillRow {
                id: r["id"].as_i64().unwrap_or_default(),
                text: backfill_text(
                    r["name"].as_str().unwrap_or_default(),
                    r["signature"].as_str(),
                    r["docstring"].as_str(),
                    r["source"].as_str(),
                ),
                stored_model: r["embedding_model"].as_str().map(str::to_string),
                stored_hash: r["embedding_source_hash"].as_str().map(str::to_string),
            })
            .collect();

        let (pending, skipped) = plan_batch(&rows, &opts.model);
        let mut stats = BackfillStats { scanned: rows.len(), skipped, ..Default::default() };

        for chunk in chunks(&pending, batch_size) {
            let inputs: Vec<String> = chunk.iter().map(|r| r.text.clone()).collect();
            let vectors = embedder.embed(&inputs).await?;
            if vectors.len() != chunk.len() {
                bail!("embedder returned {} vectors for {} inputs", vectors.len(), chunk.len());
            }
            for (row, vector) in chunk.iter().zip(vectors.iter()) {
                db.execute(
                    BACKFILL_UPDATE_SQL,
                    serde_json::json!({
                        "id": row.id,
                        "embedding": to_pgvector(vector)?,
                        "model": opts.model,
                        "hash": source_hash(&row.text),
                    }),
                )
                .await?;
                stats.embedded += 1;
            }
            stats.batches += 1;
        }

        let page_fully_handled = stats.embedded + stats.skipped == stats.scanned;
        total.merged(stats);
        if !page_fully_handled || page.len() < batch_size.min(remaining) {
            break;
        }
    }

    Ok(total)
}

pub async fn build_hnsw_index(db: &harvest_db::Db) -> Result<()> {
    db.execute(HNSW_INDEX_SQL, serde_json::json!({}))
        .await
        .map(|_| ())
        .map_err(|e| anyhow!("could not create the hnsw index: {e}"))
}

pub fn build_handle(
    cfg: &crate::config::SemanticConfig,
    providers: &[crate::config::LlmProviderConfig],
) -> Result<Option<Arc<crate::agent::graph_tools::SemanticHandle>>> {
    if !cfg.enabled {
        return Ok(None);
    }
    let provider = providers
        .iter()
        .find(|p| matches!(p, crate::config::LlmProviderConfig::Gemini { .. }))
        .ok_or_else(|| anyhow!("semantic search is enabled but no gemini provider is configured for embeddings"))?;
    let (api_key, timeout_secs, max_retries) = match provider {
        crate::config::LlmProviderConfig::Gemini { api_key, timeout_secs, max_retries, .. } => {
            (api_key.clone(), *timeout_secs, *max_retries)
        }
        _ => unreachable!(),
    };
    if api_key.trim().is_empty() {
        bail!("semantic search is enabled but the gemini provider has no api key");
    }
    let provider = Arc::new(
        crate::llm::gemini::GeminiProvider::new(
            cfg.model.clone(),
            api_key,
            timeout_secs,
            max_retries,
            crate::llm::types::ProviderMeta::new("semantic-embeddings"),
        ),
    );
    let embedder: Arc<dyn Embedder> = Arc::new(GeminiEmbedder::from_config(provider, cfg)?);
    Ok(Some(Arc::new(crate::agent::graph_tools::SemanticHandle::configured(embedder, cfg))))
}

pub async fn run_backfill_if_configured(
    db: &harvest_db::Db,
    embedder: &dyn Embedder,
    cfg: &crate::config::SemanticConfig,
) -> Result<BackfillStats> {
    let opts = BackfillOptions::from_config(cfg);
    let stats = backfill_embeddings(db, embedder, &opts).await?;
    if cfg.backfill_on_start && opts.build_index_when_complete {
        let pending = pending_rows(db, &opts.model, &opts.repo, &opts.version).await?;
        if should_build_hnsw(pending, stats.embedded, hnsw_index_exists(db).await?) {
            build_hnsw_index(db).await?;
        }
    }
    Ok(stats)
}

pub fn has_no_dimensionality(body: &str) -> bool {
    !body.contains("outputDimensionality")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_text_combines_name_signature_and_docstring() {
        let t = embedding_text("NfsDriver", Some("class NfsDriver(Base)"), Some("NFS driver."), Some("class NfsDriver(Base):\n    pass"));
        assert!(t.contains("NfsDriver"));
        assert!(t.contains("class NfsDriver(Base)"));
        assert!(t.contains("NFS driver."));
    }

    #[test]
    fn embedding_text_appends_a_source_excerpt_when_there_is_no_prose() {
        let t = embedding_text("foo", None, None, Some("   fn foo() {}   "));
        assert_eq!(t, "foo\nfn foo() {}");
    }

    #[test]
    fn embedding_text_does_not_append_source_when_prose_exists() {
        let t = embedding_text("foo", Some("fn foo()"), Some("Does a thing."), Some("fn foo() { body }"));
        assert!(!t.contains("body"), "source should be skipped when a docstring exists: {t}");
    }

    #[test]
    fn embedding_text_caps_the_source_excerpt() {
        let long: String = "q".repeat(SOURCE_EXCERPT_CHARS * 3);
        let t = embedding_text("foo", None, None, Some(&long));
        assert!(t.chars().count() <= SOURCE_EXCERPT_CHARS + 10, "excerpt must be capped: {}", t.chars().count());
    }

    #[test]
    fn embedding_text_is_never_empty_for_a_named_symbol() {
        assert_eq!(embedding_text("foo", None, None, None), "foo");
    }

    #[test]
    fn to_pgvector_formats_a_literal() {
        assert_eq!(to_pgvector(&[1.0, -2.5, 0.0]).unwrap(), "[1,-2.5,0]");
    }

    #[test]
    fn to_pgvector_rejects_empty_and_non_finite_vectors() {
        assert!(to_pgvector(&[]).is_err());
        assert!(to_pgvector(&[f32::NAN]).is_err());
        assert!(to_pgvector(&[f32::INFINITY]).is_err());
    }

    #[test]
    fn hybrid_weights_normalise_to_one() {
        let (l, s) = hybrid_weights(0.6, 0.4).unwrap();
        assert!((l - 0.6).abs() < 1e-9);
        assert!((s - 0.4).abs() < 1e-9);
        let (l2, s2) = hybrid_weights(3.0, 1.0).unwrap();
        assert!((l2 + s2 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn hybrid_weights_reject_invalid_input() {
        assert!(hybrid_weights(-1.0, 0.5).is_err());
        assert!(hybrid_weights(0.0, 0.0).is_err());
        assert!(hybrid_weights(f64::NAN, 0.5).is_err());
    }

    #[test]
    fn source_hash_is_stable_and_content_sensitive() {
        assert_eq!(source_hash("abc"), source_hash("abc"));
        assert_ne!(source_hash("abc"), source_hash("abd"));
    }

    #[test]
    fn chunks_splits_evenly_and_drops_empty_input() {
        assert_eq!(chunks(&[1, 2, 3, 4, 5], 2), vec![vec![1, 2], vec![3, 4], vec![5]]);
        assert!(chunks::<u8>(&[], 2).is_empty());
        assert!(chunks(&[1, 2], 0).is_empty());
    }

    #[test]
    fn pending_count_never_underflows() {
        assert_eq!(pending_count(10, 3), 7);
        assert_eq!(pending_count(3, 10), 0);
    }

    #[test]
    fn semantic_params_carry_vector_model_and_weights() {
        let p = semantic_params(&[0.1, 0.2], SEMANTIC_MODEL, 0.6, 0.4).unwrap();
        assert_eq!(p["qvec"], "[0.1,0.2]");
        assert_eq!(p["qmodel"], SEMANTIC_MODEL);
        assert_eq!(p["lexical"], 0.6);
    }

    #[test]
    fn hybrid_score_sql_normalises_the_weights() {
        let sql = hybrid_score_sql(3.0, 1.0).unwrap();
        assert!(sql.contains("0.75::float8"), "unexpected sql: {sql}");
        assert!(sql.contains("0.25::float8"), "unexpected sql: {sql}");
    }

    #[test]
    fn backfill_batch_limit_is_capped() {
        assert_eq!(backfill_batch_limit(None), BACKFILL_BATCH);
        assert_eq!(backfill_batch_limit(Some(10)), 10);
        assert_eq!(backfill_batch_limit(Some(10_000)), BACKFILL_BATCH);
        assert_eq!(backfill_batch_limit(Some(0)), BACKFILL_BATCH);
    }

    fn row(id: i64, name: &str, model: Option<&str>, hash: Option<&str>) -> BackfillRow {
        BackfillRow {
            id,
            text: name.to_string(),
            stored_model: model.map(str::to_string),
            stored_hash: hash.map(str::to_string),
        }
    }

    #[test]
    fn backfill_skips_rows_whose_hash_already_matches() {
        let text = "NfsDriver";
        let r = BackfillRow {
            id: 1,
            text: text.to_string(),
            stored_model: Some(SEMANTIC_MODEL.to_string()),
            stored_hash: Some(source_hash(text)),
        };
        assert!(!needs_embedding(&r, SEMANTIC_MODEL));
    }

    #[test]
    fn backfill_reembeds_when_the_source_text_changed() {
        let r = row(1, "NfsDriver", Some(SEMANTIC_MODEL), Some("stale-hash"));
        assert!(needs_embedding(&r, SEMANTIC_MODEL));
    }

    #[test]
    fn backfill_reembeds_when_the_model_changed() {
        let text = "NfsDriver";
        let r = BackfillRow {
            id: 1,
            text: text.to_string(),
            stored_model: Some("some-other-model".to_string()),
            stored_hash: Some(source_hash(text)),
        };
        assert!(needs_embedding(&r, SEMANTIC_MODEL));
    }

    #[test]
    fn backfill_embeds_rows_that_have_no_vector_yet() {
        let r = row(1, "NfsDriver", None, None);
        assert!(needs_embedding(&r, SEMANTIC_MODEL));
    }

    #[test]
    fn backfill_never_embeds_an_empty_text() {
        let r = row(1, "   ", None, None);
        assert!(!needs_embedding(&r, SEMANTIC_MODEL));
    }

    #[test]
    fn plan_batch_separates_pending_from_skipped() {
        let text = "A";
        let rows = vec![
            row(1, "A", None, None),
            BackfillRow { id: 2, text: text.into(), stored_model: Some(SEMANTIC_MODEL.into()), stored_hash: Some(source_hash(text)) },
            row(3, "B", Some("other"), None),
        ];
        let (pending, skipped) = plan_batch(&rows, SEMANTIC_MODEL);
        assert_eq!(pending.len(), 2);
        assert_eq!(skipped, 1);
        assert_eq!(pending.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn backfill_stats_merge_additively() {
        let mut a = BackfillStats { scanned: 5, embedded: 3, skipped: 2, batches: 1 };
        a.merged(BackfillStats { scanned: 4, embedded: 4, skipped: 0, batches: 1 });
        assert_eq!(a, BackfillStats { scanned: 9, embedded: 7, skipped: 2, batches: 2 });
    }

    #[test]
    fn hnsw_is_only_built_once_everything_is_embedded() {
        assert!(should_build_hnsw(0, 10, false));
    }

    #[test]
    fn hnsw_is_not_built_while_rows_remain() {
        assert!(!should_build_hnsw(3, 7, false));
    }

    #[test]
    fn hnsw_is_built_when_every_row_is_current() {
        assert!(should_build_hnsw(0, 10, false));
    }

    #[test]
    fn already_current_rows_do_not_block_the_index() {
        assert!(
            should_build_hnsw(0, 1, false),
            "rows that are already embedded are not pending work"
        );
    }

    #[test]
    fn a_partial_backfill_does_not_build_the_index() {
        assert!(
            !should_build_hnsw(90, 10, false),
            "a max_rows-limited backfill leaves pending rows behind"
        );
    }

    #[test]
    fn hnsw_is_not_built_when_there_was_nothing_to_do() {
        assert!(!should_build_hnsw(0, 0, false));
    }

    #[test]
    fn hnsw_is_not_built_twice() {
        assert!(!should_build_hnsw(0, 10, true));
    }

    #[test]
    fn pending_count_treats_a_missing_or_foreign_embedding_as_work() {
        assert!(PENDING_COUNT_SQL.contains("embedding IS NULL"));
        assert!(PENDING_COUNT_SQL.contains("embedding_model IS DISTINCT FROM $model"));
    }

    #[test]
    fn pending_count_respects_the_repository_and_version_filters() {
        assert!(PENDING_COUNT_SQL.contains("$repo = '' OR repo = $repo"));
        assert!(PENDING_COUNT_SQL.contains("$version = '' OR version = $version"));
    }

    #[test]
    fn hnsw_existence_probe_targets_the_named_index() {
        assert!(HNSW_INDEX_EXISTS_SQL.contains("idx_symbols_embedding_hnsw"));
        assert!(HNSW_INDEX_EXISTS_SQL.contains("current_schema()"));
    }

    #[test]
    fn hnsw_index_uses_cosine_ops_and_is_idempotent() {
        assert!(HNSW_INDEX_SQL.contains("USING hnsw (embedding vector_cosine_ops)"));
        assert!(HNSW_INDEX_SQL.contains("IF NOT EXISTS"));
    }

    #[test]
    fn backfill_update_writes_the_model_hash_and_timestamp() {
        assert!(BACKFILL_UPDATE_SQL.contains("embedding_model = $model"));
        assert!(BACKFILL_UPDATE_SQL.contains("embedding_source_hash = $hash"));
        assert!(BACKFILL_UPDATE_SQL.contains("embedded_at = now()"));
        assert!(BACKFILL_UPDATE_SQL.contains("WHERE id = $id"));
    }

    #[test]
    fn backfill_candidates_are_ordered_and_paged() {
        assert!(BACKFILL_CANDIDATE_SQL.contains("ORDER BY id"));
        assert!(BACKFILL_CANDIDATE_SQL.contains("LIMIT $limit"));
    }

    #[test]
    fn backfill_options_respect_the_configured_batch_size() {
        let cfg = crate::config::SemanticConfig { model: "m".into(), batch_size: 5_000, ..Default::default() };
        let opts = BackfillOptions::from_config(&cfg);
        assert_eq!(opts.batch_size, BACKFILL_BATCH);
        assert_eq!(opts.model, "m");
        assert!(opts.build_index_when_complete);
    }

    #[test]
    fn semantic_available_sql_asks_for_the_embedding_column() {
        assert!(SEMANTIC_AVAILABILITY_SQL.contains("table_name = 'symbols'"));
        assert!(SEMANTIC_AVAILABILITY_SQL.contains("column_name = 'embedding'"));
    }

    #[test]
    fn backfill_rejects_a_blank_model() {
        let opts = BackfillOptions { model: "  ".into(), ..Default::default() };
        let db = harvest_db::Db::connect_without_migrating("postgres://127.0.0.1:1/none").unwrap();
        let embedder = NoopEmbedder;
        let err = futures::executor::block_on(backfill_embeddings(&db, &embedder, &opts)).unwrap_err().to_string();
        assert!(err.contains("requires an embedding model"), "unexpected error: {err}");
    }

    struct NoopEmbedder;

    #[async_trait::async_trait]
    impl Embedder for NoopEmbedder {
        fn model(&self) -> &str { SEMANTIC_MODEL }
        fn dimensions(&self) -> usize { SEMANTIC_DIMENSIONS }
        async fn embed(&self, _inputs: &[String]) -> Result<Vec<Vec<f32>>> { Ok(Vec::new()) }
    }

    fn gemini_providers() -> Vec<crate::config::LlmProviderConfig> {
        use crate::config::LlmProviderConfig;
        vec![LlmProviderConfig::Gemini {
            model: "gemini-2.5-flash".into(),
            api_key: "key1".into(),
            id: String::new(),
            priority: 0,
            timeout_secs: 30,
            max_retries: 2,
            expose_to_ui: true,
            name: None,
            models: None,
            user_provided_key: false,
            pricing: None,
        }]
    }

    fn enabled_cfg() -> crate::config::SemanticConfig {
        crate::config::SemanticConfig { enabled: true, ..Default::default() }
    }

    fn build_handle_err(cfg: &crate::config::SemanticConfig, providers: &[crate::config::LlmProviderConfig]) -> String {
        match build_handle(cfg, providers) {
            Ok(_) => panic!("expected an error"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn build_handle_returns_nothing_when_semantic_is_disabled() {
        assert!(build_handle(&crate::config::SemanticConfig::default(), &gemini_providers()).unwrap().is_none());
    }

    #[test]
    fn build_handle_creates_a_ready_handle_when_enabled() {
        let handle = build_handle(&enabled_cfg(), &gemini_providers()).unwrap().expect("handle");
        assert!(handle.is_ready());
        assert_eq!(handle.model, "text-embedding-004");
    }

    #[test]
    fn build_handle_requires_a_gemini_provider() {
        let err = build_handle_err(&enabled_cfg(), &[]);
        assert!(err.contains("no gemini provider"), "unexpected error: {err}");
    }

    #[test]
    fn build_handle_requires_an_api_key() {
        use crate::config::LlmProviderConfig;
        let mut providers = gemini_providers();
        if let LlmProviderConfig::Gemini { api_key, .. } = &mut providers[0] {
            *api_key = "  ".to_string();
        }
        let err = build_handle_err(&enabled_cfg(), &providers);
        assert!(err.contains("no api key"), "unexpected error: {err}");
    }

    #[test]
    fn build_handle_rejects_zero_dimensions() {
        let cfg = crate::config::SemanticConfig { enabled: true, dimensions: 0, ..Default::default() };
        let err = build_handle_err(&cfg, &gemini_providers());
        assert!(err.contains("greater than zero"), "unexpected error: {err}");
    }

    #[test]
    fn build_handle_uses_the_configured_model_and_weights() {
        let cfg = crate::config::SemanticConfig {
            enabled: true,
            model: "custom-embed".into(),
            lexical_weight: 1.0,
            semantic_weight: 1.0,
            ..Default::default()
        };
        let handle = build_handle(&cfg, &gemini_providers()).unwrap().unwrap();
        assert_eq!(handle.model, "custom-embed");
        assert!((handle.lexical_weight - 0.5).abs() < 1e-9);
        assert!((handle.semantic_weight - 0.5).abs() < 1e-9);
    }

    #[test]
    fn backfill_options_default_to_no_repo_scope() {
        let opts = BackfillOptions::default();
        assert!(opts.repo.is_empty());
        assert!(opts.version.is_empty());
        assert!(opts.max_rows.is_none());
    }

    #[test]
    fn defaults_match_the_agreed_model_and_dimensions() {
        assert_eq!(SEMANTIC_MODEL, "text-embedding-004");
        assert_eq!(SEMANTIC_DIMENSIONS, 768);
    }
}
