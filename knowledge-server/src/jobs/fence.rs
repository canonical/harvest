use std::sync::Arc;

use harvest_db::Tx;
use serde_json::json;

use crate::cluster::lease::Lease;
use crate::metrics::FENCED_WRITES_REJECTED;

#[derive(Debug, thiserror::Error)]
pub enum Fenced {
    #[error("the lease for {0} was lost")]
    LeaseLost(String),
    #[error("a newer holder of {0} already wrote")]
    Superseded(String),
    #[error(transparent)]
    Store(#[from] anyhow::Error),
}

const ADVANCE_SQL: &str = r#"
INSERT INTO fencing_tokens (key, token) VALUES ($key, $token)
ON CONFLICT (key) DO UPDATE SET token = EXCLUDED.token
 WHERE fencing_tokens.token <= EXCLUDED.token
RETURNING token
"#;

#[derive(Clone)]
pub struct Fence {
    lease: Arc<Lease>,
}

impl Fence {
    pub fn new(lease: Arc<Lease>) -> Self {
        Self { lease }
    }

    pub fn token(&self) -> u64 {
        self.lease.token()
    }

    pub fn key(&self) -> &str {
        self.lease.key()
    }

    pub fn check(&self) -> Result<(), Fenced> {
        if self.lease.is_lost() {
            return Err(Fenced::LeaseLost(self.key().to_string()));
        }
        Ok(())
    }

    pub async fn commit(&self, tx: Tx) -> Result<(), Fenced> {
        self.check()?;
        let rows = tx
            .query(
                ADVANCE_SQL,
                json!({ "key": self.key(), "token": self.token() as i64 }),
            )
            .await?;
        if rows.is_empty() {
            tx.rollback().await?;
            FENCED_WRITES_REJECTED.inc(&[("key", self.key())]);
            return Err(Fenced::Superseded(self.key().to_string()));
        }
        tx.commit().await?;
        Ok(())
    }
}
