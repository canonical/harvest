use anyhow::{Context, Result};
use tokio_postgres::Client;

const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
];

/// Changes whenever a migration is added or edited.
pub fn fingerprint() -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for (version, sql) in MIGRATIONS {
        for byte in version.to_le_bytes().iter().chain(sql.as_bytes()) {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

const MIGRATION_LOCK_KEY: i64 = 0x4841_5256_4553_54; // "HARVEST"

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
