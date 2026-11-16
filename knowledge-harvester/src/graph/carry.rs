use anyhow::Result;
use harvest_db::Tx;
use serde_json::json;

const EMBEDDINGS_PRESENT: &str = "SELECT EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_schema = current_schema() AND table_name = 'symbols' AND column_name = 'embedding'
) AS present";

pub struct Carried {
    embeddings: bool,
}

impl Carried {
    pub async fn keep(tx: &Tx, version_id: i64) -> Result<Self> {
        tx.batch_execute(&keep_enrichments(version_id)).await?;
        let embeddings = embeddings_present(tx).await?;
        if embeddings {
            tx.batch_execute(&keep_embeddings(version_id)).await?;
        }
        Ok(Self { embeddings })
    }

    pub async fn restore(&self, tx: &Tx, version_id: i64) -> Result<()> {
        tx.batch_execute(&restore_enrichments(version_id)).await?;
        if self.embeddings {
            tx.batch_execute(&restore_embeddings(version_id)).await?;
        }
        Ok(())
    }
}

async fn embeddings_present(tx: &Tx) -> Result<bool> {
    let rows = tx.query_uncached(EMBEDDINGS_PRESENT, json!({})).await?;
    Ok(rows.first().and_then(|r| r["present"].as_bool()).unwrap_or(false))
}

fn keep_enrichments(version_id: i64) -> String {
    format!(
        "CREATE TEMP TABLE carried_enrichments ON COMMIT DROP AS
         SELECT DISTINCT ON (e.source_hash) e.source_hash, e.model, e.summary, e.questions, e.tags
         FROM symbol_enrichments e JOIN symbols s ON s.id = e.symbol_id
         WHERE s.version_id = {version_id} AND e.source_hash = md5(s.source)"
    )
}

fn restore_enrichments(version_id: i64) -> String {
    format!(
        "INSERT INTO symbol_enrichments (symbol_id, source_hash, model, summary, questions, tags)
         SELECT s.id, c.source_hash, c.model, c.summary, c.questions, c.tags
         FROM symbols s JOIN carried_enrichments c ON c.source_hash = md5(s.source)
         WHERE s.version_id = {version_id}
         ON CONFLICT (symbol_id) DO NOTHING"
    )
}

fn keep_embeddings(version_id: i64) -> String {
    format!(
        "CREATE TEMP TABLE carried_embeddings ON COMMIT DROP AS
         SELECT DISTINCT ON (md5(source), name) md5(source) AS source_hash, name,
                embedding, embedding_model, embedding_source_hash
         FROM symbols
         WHERE version_id = {version_id} AND embedding IS NOT NULL AND source IS NOT NULL"
    )
}

fn restore_embeddings(version_id: i64) -> String {
    format!(
        "UPDATE symbols s SET embedding = c.embedding, embedding_model = c.embedding_model,
             embedding_source_hash = c.embedding_source_hash
         FROM carried_embeddings c
         WHERE s.version_id = {version_id} AND s.name = c.name AND md5(s.source) = c.source_hash"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carried_rows_are_matched_by_source_hash_within_the_version() {
        for sql in [keep_enrichments(7), restore_enrichments(7), keep_embeddings(7), restore_embeddings(7)] {
            assert!(sql.contains("version_id = 7"));
            assert!(sql.contains("md5(s.source)") || sql.contains("md5(source)"));
        }
        assert!(keep_enrichments(7).contains("ON COMMIT DROP"));
        assert!(restore_enrichments(7).contains("ON CONFLICT (symbol_id) DO NOTHING"));
    }
}
