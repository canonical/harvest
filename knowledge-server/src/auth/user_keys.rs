use anyhow::Result;
use serde_json::json;
use std::sync::Arc;

use crate::crypto::Crypto;
use harvest_db::Db;

pub struct UserKeyStore {
    db: Arc<Db>,
    crypto: Arc<Crypto>,
}

impl UserKeyStore {
    pub fn new(db: Arc<Db>, crypto: Arc<Crypto>) -> Self {
        Self { db, crypto }
    }

    pub async fn upsert(&self, user_id: &str, provider_id: &str, api_key: &str) -> Result<()> {
        let (ciphertext, nonce) = self.crypto.encrypt(api_key)?;
        let now = harvest_db::now_rfc3339();
        self.db.query(
            "INSERT INTO user_llm_keys (user_id, provider_id, key_ciphertext, key_nonce, updated_at)
             VALUES ($uid, $pid, $ct, $nonce, $now)
             ON CONFLICT (user_id, provider_id) DO UPDATE SET
                 key_ciphertext = EXCLUDED.key_ciphertext,
                 key_nonce      = EXCLUDED.key_nonce,
                 updated_at     = EXCLUDED.updated_at",
            json!({
                "uid": user_id,
                "pid": provider_id,
                "ct": ciphertext,
                "nonce": nonce,
                "now": now,
            }),
        ).await?;
        Ok(())
    }

    pub async fn get(&self, user_id: &str, provider_id: &str) -> Result<Option<String>> {
        let rows = self.db.query(
            "SELECT key_ciphertext AS ct, key_nonce AS nonce
             FROM user_llm_keys WHERE user_id = $uid AND provider_id = $pid",
            json!({ "uid": user_id, "pid": provider_id }),
        ).await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let row = &rows[0];
        let ct = row["ct"].as_str().unwrap_or("");
        let nonce = row["nonce"].as_str().unwrap_or("");
        if ct.is_empty() || nonce.is_empty() {
            return Ok(None);
        }
        let plaintext = self.crypto.decrypt(ct, nonce)?;
        Ok(Some(plaintext))
    }

    pub async fn list_set_keys(&self, user_id: &str) -> Result<Vec<String>> {
        let rows = self.db.query(
            "SELECT provider_id AS pid FROM user_llm_keys WHERE user_id = $uid",
            json!({ "uid": user_id }),
        ).await?;
        Ok(rows.iter()
            .filter_map(|r| r["pid"].as_str().map(String::from))
            .collect())
    }

    pub async fn list_with_status(&self, user_id: &str) -> Result<Vec<UserKeyStatus>> {
        let rows = self.db.query(
            "SELECT provider_id AS pid, updated_at FROM user_llm_keys WHERE user_id = $uid",
            json!({ "uid": user_id }),
        ).await?;
        Ok(rows.iter().map(|r| UserKeyStatus {
            provider_id: r["pid"].as_str().unwrap_or("").to_string(),
            updated_at: r["updated_at"].as_str().map(String::from),
        }).collect())
    }

    pub async fn delete(&self, user_id: &str, provider_id: &str) -> Result<()> {
        self.db.query(
            "DELETE FROM user_llm_keys WHERE user_id = $uid AND provider_id = $pid",
            json!({ "uid": user_id, "pid": provider_id }),
        ).await?;
        Ok(())
    }

    pub async fn resolve_all(&self, user_id: &str) -> Result<std::collections::HashMap<String, String>> {
        let rows = self.db.query(
            "SELECT provider_id AS pid, key_ciphertext AS ct, key_nonce AS nonce
             FROM user_llm_keys WHERE user_id = $uid",
            json!({ "uid": user_id }),
        ).await?;
        let mut keys = std::collections::HashMap::new();
        for row in &rows {
            let pid = match row["pid"].as_str() {
                Some(p) => p.to_string(),
                None => continue,
            };
            let ct = row["ct"].as_str().unwrap_or("");
            let nonce = row["nonce"].as_str().unwrap_or("");
            if ct.is_empty() || nonce.is_empty() {
                continue;
            }
            match self.crypto.decrypt(ct, nonce) {
                Ok(plaintext) => { keys.insert(pid, plaintext); }
                Err(e) => tracing::warn!(error = %e, "failed to decrypt user key, skipping"),
            }
        }
        Ok(keys)
    }
}

#[derive(Debug, Clone)]
pub struct UserKeyStatus {
    pub provider_id: String,
    pub updated_at: Option<String>,
}
