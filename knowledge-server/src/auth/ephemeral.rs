use std::time::Duration;

use anyhow::Result;
use harvest_db::Db;
use serde_json::{json, Value};

pub const OAUTH_KIND: &str = "oauth";
pub const TUI_KIND: &str = "tui";

pub async fn put(db: &Db, kind: &str, key: &str, data: &Value, ttl: Duration) -> Result<()> {
    db.execute(
        "INSERT INTO auth_ephemeral (key, kind, data, expires_at)
         VALUES ($key, $kind, $data, now() + make_interval(secs => $ttl::float8))
         ON CONFLICT (key) DO UPDATE SET kind = EXCLUDED.kind, data = EXCLUDED.data, expires_at = EXCLUDED.expires_at",
        json!({ "key": format!("{kind}:{key}"), "kind": kind, "data": data, "ttl": ttl.as_secs_f64() }),
    ).await?;
    Ok(())
}

pub async fn get(db: &Db, kind: &str, key: &str) -> Result<Option<Value>> {
    let rows = db.query(
        "SELECT data FROM auth_ephemeral WHERE key = $key AND expires_at > now()",
        json!({ "key": format!("{kind}:{key}") }),
    ).await?;
    Ok(rows.into_iter().next().map(|r| r["data"].clone()))
}

pub async fn take(db: &Db, kind: &str, key: &str) -> Result<Option<Value>> {
    let rows = db.query(
        "DELETE FROM auth_ephemeral WHERE key = $key RETURNING data, expires_at > now() AS live",
        json!({ "key": format!("{kind}:{key}") }),
    ).await?;
    Ok(rows.into_iter().next().filter(|r| r["live"].as_bool() == Some(true)).map(|r| r["data"].clone()))
}

pub async fn replace_if<F>(db: &Db, kind: &str, key: &str, update: F) -> Result<Option<Value>>
where
    F: FnOnce(&Value) -> Option<Value>,
{
    let tx = db.begin().await?;
    let rows = tx.query(
        "SELECT data FROM auth_ephemeral WHERE key = $key AND expires_at > now() FOR UPDATE",
        json!({ "key": format!("{kind}:{key}") }),
    ).await?;
    let Some(current) = rows.into_iter().next().map(|r| r["data"].clone()) else {
        tx.rollback().await?;
        return Ok(None);
    };
    let Some(next) = update(&current) else {
        tx.rollback().await?;
        return Ok(None);
    };
    tx.execute(
        "UPDATE auth_ephemeral SET data = $data WHERE key = $key",
        json!({ "key": format!("{kind}:{key}"), "data": next }),
    ).await?;
    tx.commit().await?;
    Ok(Some(next))
}

pub async fn purge_expired(db: &Db) -> Result<u64> {
    db.execute("DELETE FROM auth_ephemeral WHERE expires_at <= now()", json!({})).await
}
