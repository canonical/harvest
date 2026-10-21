use anyhow::Result;
use axum::{extract::Request, http::StatusCode, middleware::Next, response::IntoResponse, Json};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use harvest_db::Db;

#[derive(Debug, Clone)]
pub struct ApiKeyPrincipal {
    pub key_id: String,
    pub user_id: String,
    pub key_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ApiKeyMeta {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
}

pub struct ApiKeyStore {
    db: Arc<Db>,
}

pub struct CreatedApiKey {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub secret: String,
    pub created_at: String,
}

impl ApiKeyStore {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db }
    }

    pub async fn create(&self, user_id: &str, name: &str) -> Result<CreatedApiKey> {
        let id = Uuid::new_v4().to_string();
        let now = harvest_db::now_rfc3339();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let key_prefix = token.chars().take(12).collect::<String>();
        let secret = format!("hk_live_{}_{}", key_prefix, token);
        let key_hash = hash_key(&secret);
        self.db.query(
            "INSERT INTO user_api_keys (id, user_id, name, key_prefix, key_hash, created_at, updated_at)
             VALUES ($id, $uid, $name, $prefix, $hash, $now, $now)",
            json!({
                "id": id,
                "uid": user_id,
                "name": name,
                "prefix": key_prefix,
                "hash": key_hash,
                "now": now,
            }),
        ).await?;
        Ok(CreatedApiKey {
            id,
            name: name.to_string(),
            key_prefix,
            secret,
            created_at: now,
        })
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<ApiKeyMeta>> {
        let rows = self
            .db
            .query(
                "SELECT id, name, key_prefix, created_at, updated_at, last_used_at, revoked_at
             FROM user_api_keys WHERE user_id = $uid ORDER BY created_at DESC",
                json!({ "uid": user_id }),
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| ApiKeyMeta {
                id: row["id"].as_str().unwrap_or_default().to_string(),
                name: row["name"].as_str().unwrap_or_default().to_string(),
                key_prefix: row["key_prefix"].as_str().unwrap_or_default().to_string(),
                created_at: row["created_at"].as_str().unwrap_or_default().to_string(),
                updated_at: row["updated_at"].as_str().unwrap_or_default().to_string(),
                last_used_at: row["last_used_at"].as_str().map(String::from),
                revoked_at: row["revoked_at"].as_str().map(String::from),
            })
            .collect())
    }

    pub async fn rename(&self, user_id: &str, key_id: &str, name: &str) -> Result<bool> {
        let rows = self.db.query(
            "UPDATE user_api_keys SET name = $name, updated_at = $now
             WHERE id = $id AND user_id = $uid
             RETURNING id",
            json!({ "id": key_id, "uid": user_id, "name": name, "now": harvest_db::now_rfc3339() }),
        ).await?;
        Ok(!rows.is_empty())
    }

    pub async fn revoke(&self, user_id: &str, key_id: &str) -> Result<bool> {
        let rows = self
            .db
            .query(
                "UPDATE user_api_keys
             SET revoked_at = COALESCE(revoked_at, $now), updated_at = $now
             WHERE id = $id AND user_id = $uid
             RETURNING id",
                json!({ "id": key_id, "uid": user_id, "now": harvest_db::now_rfc3339() }),
            )
            .await?;
        Ok(!rows.is_empty())
    }

    pub async fn authenticate(&self, token: &str) -> Result<Option<ApiKeyPrincipal>> {
        let Some(prefix) = key_prefix(token) else {
            return Ok(None);
        };
        let rows = self
            .db
            .query(
                "SELECT id, user_id, name, key_hash
             FROM user_api_keys
             WHERE key_prefix = $prefix AND revoked_at IS NULL",
                json!({ "prefix": prefix }),
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let stored_hash = row["key_hash"].as_str().unwrap_or_default();
        let provided_hash = hash_key(token);
        if stored_hash != provided_hash {
            return Ok(None);
        }
        let key_id = row["id"].as_str().unwrap_or_default().to_string();
        self.db
            .execute(
                "UPDATE user_api_keys SET last_used_at = $now, updated_at = $now WHERE id = $id",
                json!({ "id": key_id, "now": harvest_db::now_rfc3339() }),
            )
            .await?;
        Ok(Some(ApiKeyPrincipal {
            key_id,
            user_id: row["user_id"].as_str().unwrap_or_default().to_string(),
            key_name: row["name"].as_str().unwrap_or_default().to_string(),
        }))
    }
}

fn hash_key(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

fn key_prefix(token: &str) -> Option<&str> {
    let body = token.strip_prefix("hk_live_")?;
    let (prefix, _) = body.split_once('_')?;
    if prefix.is_empty() {
        return None;
    }
    Some(prefix)
}

fn bearer_token(req: &Request) -> Option<String> {
    let auth = req.headers().get(axum::http::header::AUTHORIZATION)?;
    let value = auth.to_str().ok()?;
    value.strip_prefix("Bearer ").map(ToString::to_string)
}

pub async fn require_api_key(
    axum::extract::State(store): axum::extract::State<Arc<ApiKeyStore>>,
    mut req: Request,
    next: Next,
) -> impl IntoResponse {
    let token = bearer_token(&req);
    let Some(token) = token else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response();
    };
    match store.authenticate(&token).await {
        Ok(Some(principal)) => {
            req.extensions_mut().insert(principal);
            next.run(req).await.into_response()
        }
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "server error" })),
        )
            .into_response(),
    }
}
