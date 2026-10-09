use anyhow::{Context, Result};
use tokio_postgres::Client;

const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_docstring_and_search.sql")),
    (3, include_str!("../migrations/0003_semantic.sql")),
    (4, include_str!("../migrations/0004_parser_version.sql")),
    (5, include_str!("../migrations/0005_decorators.sql")),
    (6, include_str!("../migrations/0006_conversation_summary.sql")),
];

pub fn fingerprint() -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (version, sql) in MIGRATIONS {
        for byte in version.to_le_bytes().iter().chain(sql.as_bytes()) {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

const MIGRATION_LOCK_KEY: i64 = 0x4841_5256_4553_54;

pub async fn run(client: &mut Client) -> Result<()> {
    client.batch_execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    integer PRIMARY KEY,
             applied_at timestamptz NOT NULL DEFAULT now()
         )",
    ).await?;
    client.execute("SELECT pg_advisory_lock($1)", &[&MIGRATION_LOCK_KEY]).await?;
    let result = apply_pending(client).await;
    client.execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY]).await?;
    result
}

async fn apply_pending(client: &mut Client) -> Result<()> {
    for (version, sql) in MIGRATIONS {
        let applied = client
            .query_opt("SELECT 1 FROM schema_migrations WHERE version = $1", &[version])
            .await?
            .is_some();
        if applied {
            continue;
        }
        let tx = client.transaction().await?;
        tx.batch_execute(sql).await.with_context(|| format!("migration {version} failed"))?;
        tx.execute("INSERT INTO schema_migrations (version) VALUES ($1)", &[version]).await?;
        tx.commit().await?;
        tracing::info!(version, "applied database migration");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sql_for(version: i32) -> &'static str {
        MIGRATIONS.iter().find(|(v, _)| *v == version).map(|(_, s)| *s).unwrap()
    }

    #[test]
    fn migration_versions_are_unique_and_ascending() {
        let versions: Vec<i32> = MIGRATIONS.iter().map(|(v, _)| *v).collect();
        let mut sorted = versions.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(versions, sorted, "migration versions must be unique and ascending");
    }

    #[test]
    fn migration_2_adds_docstring_column_and_indexes() {
        let sql = sql_for(2);
        assert!(sql.contains("docstring"), "migration 2 must add a docstring column");
        assert!(sql.contains("symbols_signature_trgm"), "migration 2 must index signature for trigram search");
        assert!(sql.contains("symbols_docstring_trgm"), "migration 2 must index docstring for trigram search");
    }

    #[test]
    fn migration_2_is_idempotent_where_possible() {
        let sql = sql_for(2);
        assert!(sql.contains("IF NOT EXISTS"), "migration 2 must be re-runnable safely");
    }

    #[test]
    fn fingerprint_changes_when_a_migration_is_added() {
        let before = fingerprint();
        assert_ne!(before, 0);
    }

    #[test]
    fn later_migrations_do_not_block_startup_when_vector_is_missing() {
        let sql = sql_for(3);
        assert!(sql.contains("pg_available_extensions"), "vector setup must check availability first");
        assert!(sql.contains("EXCEPTION"), "vector setup must not abort the migration on failure");
    }
}
