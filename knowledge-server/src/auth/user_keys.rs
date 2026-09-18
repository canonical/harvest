use anyhow::Result;
use serde_json::json;
use std::sync::Arc;

use crate::crypto::Crypto;
use crate::neo4j::Neo4jClient;

pub struct UserKeyStore {
    neo4j: Arc<Neo4jClient>,
    crypto: Arc<Crypto>,
}

impl UserKeyStore {
    pub fn new(neo4j: Arc<Neo4jClient>, crypto: Arc<Crypto>) -> Self {
        Self { neo4j, crypto }
    }

    pub async fn upsert(&self, user_id: &str, provider_id: &str, api_key: &str) -> Result<()> {
        let (ciphertext, nonce) = self.crypto.encrypt(api_key)?;
        let now = chrono::Utc::now().to_rfc3339();
        let key_id = format!("{user_id}:{provider_id}");
        self.neo4j.query_read(
            "MATCH (u:User {id: $uid})
             MERGE (u)-[:HAS_LLM_KEY]->(k:UserLlmKey {key_id: $key_id})
             SET k.provider_id = $pid, k.key_ciphertext = $ct, k.key_nonce = $nonce, k.updated_at = $now",
            json!({
                "uid": user_id,
                "key_id": key_id,
                "pid": provider_id,
                "ct": ciphertext,
                "nonce": nonce,
                "now": now,
            }),
        ).await?;
        Ok(())
    }

    pub async fn get(&self, user_id: &str, provider_id: &str) -> Result<Option<String>> {
        let rows = self.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:HAS_LLM_KEY]->(k:UserLlmKey {provider_id: $pid})
             RETURN k.key_ciphertext AS ct, k.key_nonce AS nonce",
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
        let rows = self.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:HAS_LLM_KEY]->(k:UserLlmKey)
             RETURN k.provider_id AS pid",
            json!({ "uid": user_id }),
        ).await?;
        Ok(rows.iter()
            .filter_map(|r| r["pid"].as_str().map(String::from))
            .collect())
    }

    pub async fn list_with_status(&self, user_id: &str) -> Result<Vec<UserKeyStatus>> {
        let rows = self.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:HAS_LLM_KEY]->(k:UserLlmKey)
             RETURN k.provider_id AS pid, k.updated_at AS updated_at",
            json!({ "uid": user_id }),
        ).await?;
        Ok(rows.iter().map(|r| UserKeyStatus {
            provider_id: r["pid"].as_str().unwrap_or("").to_string(),
            updated_at: r["updated_at"].as_str().map(String::from),
        }).collect())
    }

    pub async fn delete(&self, user_id: &str, provider_id: &str) -> Result<()> {
        self.neo4j.query_read(
            "MATCH (:User {id: $uid})-[r:HAS_LLM_KEY]->(k:UserLlmKey {provider_id: $pid})
             DELETE r, k",
            json!({ "uid": user_id, "pid": provider_id }),
        ).await?;
        Ok(())
    }

    pub async fn resolve_all(&self, user_id: &str) -> Result<std::collections::HashMap<String, String>> {
        let rows = self.neo4j.query_read(
            "MATCH (:User {id: $uid})-[:HAS_LLM_KEY]->(k:UserLlmKey)
             RETURN k.provider_id AS pid, k.key_ciphertext AS ct, k.key_nonce AS nonce",
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
