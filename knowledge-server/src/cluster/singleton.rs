use std::future::Future;

use anyhow::Result;
use harvest_db::Db;
use serde_json::json;

pub async fn run_exclusive<F, Fut>(db: &Db, key: &str, job: F) -> Result<bool>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = ()>,
{
    let tx = db.begin().await?;
    let rows = tx
        .query(
            "SELECT pg_try_advisory_xact_lock(hashtext($key)) AS locked",
            json!({ "key": format!("harvest-singleton:{key}") }),
        )
        .await?;
    if rows.first().and_then(|r| r["locked"].as_bool()) != Some(true) {
        tx.rollback().await?;
        return Ok(false);
    }
    job().await;
    tx.commit().await?;
    Ok(true)
}
