pub mod handlers;
pub mod keys;
pub mod platform;
pub mod remote;

use std::sync::Arc;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use harvest_db::Db;
use knowledge_harvester::git::GitCredentials;

use crate::crypto::Crypto;
use platform::Platform;
use remote::{RemoteUrl, Transport};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SshKey {
    pub public_key: String,
    pub fingerprint: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlatformAccount {
    pub platform: String,
    pub host: String,
    pub username: String,
    pub verified_at: Option<String>,
}

pub struct NewAccount<'a> {
    pub platform: Platform,
    pub host: &'a str,
    pub username: &'a str,
    pub secret: &'a str,
}

pub struct CredentialStore {
    db: Arc<Db>,
    crypto: Arc<Crypto>,
}

impl CredentialStore {
    pub fn new(db: Arc<Db>, crypto: Arc<Crypto>) -> Self {
        Self { db, crypto }
    }

    pub async fn ssh_key(&self, user_id: &str) -> Result<Option<SshKey>> {
        let rows = self
            .db
            .query(
                "SELECT public_key, fingerprint, created_at FROM ssh_keys WHERE user_id = $uid",
                json!({ "uid": user_id }),
            )
            .await?;
        Ok(rows.first().map(|row| SshKey {
            public_key: text(row, "public_key"),
            fingerprint: text(row, "fingerprint"),
            created_at: text(row, "created_at"),
        }))
    }

    pub async fn ssh_private_key(&self, user_id: &str) -> Result<Option<String>> {
        let rows = self
            .db
            .query(
                "SELECT private_key_ciphertext AS ct, private_key_nonce AS nonce
                 FROM ssh_keys WHERE user_id = $uid",
                json!({ "uid": user_id }),
            )
            .await?;
        rows.first()
            .map(|row| self.crypto.decrypt(&text(row, "ct"), &text(row, "nonce")))
            .transpose()
    }

    pub async fn generate_ssh_key(&self, user_id: &str) -> Result<SshKey> {
        let pair = keys::generate().await?;
        self.store_ssh_key(user_id, pair).await
    }

    pub async fn import_ssh_key(&self, user_id: &str, private_key: &str) -> Result<SshKey> {
        let pair = keys::inspect(private_key).await?;
        self.store_ssh_key(user_id, pair).await
    }

    async fn store_ssh_key(&self, user_id: &str, pair: keys::KeyPair) -> Result<SshKey> {
        let (ciphertext, nonce) = self.crypto.encrypt(&pair.private_key)?;
        self.db
            .execute(
                "INSERT INTO ssh_keys (user_id, private_key_ciphertext, private_key_nonce,
                                       public_key, fingerprint, created_at)
                 VALUES ($uid, $ct, $nonce, $public, $fingerprint, now())
                 ON CONFLICT (user_id) DO UPDATE SET
                     private_key_ciphertext = EXCLUDED.private_key_ciphertext,
                     private_key_nonce      = EXCLUDED.private_key_nonce,
                     public_key             = EXCLUDED.public_key,
                     fingerprint            = EXCLUDED.fingerprint,
                     created_at             = EXCLUDED.created_at",
                json!({
                    "uid": user_id, "ct": ciphertext, "nonce": nonce,
                    "public": pair.public_key, "fingerprint": pair.fingerprint,
                }),
            )
            .await?;
        self.ssh_key(user_id)
            .await?
            .context("the SSH key was not stored")
    }

    pub async fn delete_ssh_key(&self, user_id: &str) -> Result<()> {
        self.db
            .execute(
                "DELETE FROM ssh_keys WHERE user_id = $uid",
                json!({ "uid": user_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn accounts(&self, user_id: &str) -> Result<Vec<PlatformAccount>> {
        let rows = self
            .db
            .query(
                "SELECT platform, host, username, verified_at FROM platform_accounts
                 WHERE user_id = $uid ORDER BY platform, host",
                json!({ "uid": user_id }),
            )
            .await?;
        Ok(rows.iter().map(account_from_row).collect())
    }

    pub async fn account_for_host(
        &self,
        user_id: &str,
        host: &str,
    ) -> Result<Option<(PlatformAccount, String)>> {
        let rows = self
            .db
            .query(
                "SELECT platform, host, username, verified_at,
                        secret_ciphertext AS ct, secret_nonce AS nonce
                 FROM platform_accounts WHERE user_id = $uid AND host = $host",
                json!({ "uid": user_id, "host": host.to_ascii_lowercase() }),
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let secret = self.crypto.decrypt(&text(row, "ct"), &text(row, "nonce"))?;
        Ok(Some((account_from_row(row), secret)))
    }

    pub async fn save_account(&self, user_id: &str, account: NewAccount<'_>) -> Result<()> {
        let (ciphertext, nonce) = self.crypto.encrypt(account.secret)?;
        self.db
            .execute(
                "INSERT INTO platform_accounts (user_id, platform, host, username,
                                                secret_ciphertext, secret_nonce)
                 VALUES ($uid, $platform, $host, $username, $ct, $nonce)
                 ON CONFLICT (user_id, host) DO UPDATE SET
                     platform          = EXCLUDED.platform,
                     username          = EXCLUDED.username,
                     secret_ciphertext = EXCLUDED.secret_ciphertext,
                     secret_nonce      = EXCLUDED.secret_nonce,
                     verified_at       = NULL",
                json!({
                    "uid": user_id,
                    "platform": account.platform.as_str(),
                    "host": account.host.to_ascii_lowercase(),
                    "username": account.username,
                    "ct": ciphertext,
                    "nonce": nonce,
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn mark_verified(&self, user_id: &str, host: &str) -> Result<()> {
        self.db
            .execute(
                "UPDATE platform_accounts SET verified_at = now()
                 WHERE user_id = $uid AND host = $host",
                json!({ "uid": user_id, "host": host.to_ascii_lowercase() }),
            )
            .await?;
        Ok(())
    }

    pub async fn delete_account(&self, user_id: &str, host: &str) -> Result<()> {
        self.db
            .execute(
                "DELETE FROM platform_accounts WHERE user_id = $uid AND host = $host",
                json!({ "uid": user_id, "host": host.to_ascii_lowercase() }),
            )
            .await?;
        Ok(())
    }

    pub async fn git_credentials(&self, user_id: &str, url: &str) -> Result<GitCredentials> {
        let Some(remote) = RemoteUrl::parse(url) else {
            return Ok(GitCredentials::Anonymous);
        };
        match remote.transport {
            Transport::Ssh => Ok(self
                .ssh_private_key(user_id)
                .await?
                .map(|private_key| GitCredentials::SshKey { private_key })
                .unwrap_or_default()),
            Transport::Https => Ok(self
                .account_for_host(user_id, &remote.host)
                .await?
                .and_then(|(account, secret)| basic_credentials(&account, secret))
                .unwrap_or_default()),
        }
    }
}

fn basic_credentials(account: &PlatformAccount, secret: String) -> Option<GitCredentials> {
    let platform: Platform = account.platform.parse().ok()?;
    let username = platform.https_username(&account.username)?;
    Some(GitCredentials::Basic {
        username,
        password: secret,
    })
}

fn account_from_row(row: &Value) -> PlatformAccount {
    PlatformAccount {
        platform: text(row, "platform"),
        host: text(row, "host"),
        username: text(row, "username"),
        verified_at: row["verified_at"].as_str().map(String::from),
    }
}

fn text(row: &Value, key: &str) -> String {
    row[key].as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(platform: &str, username: &str) -> PlatformAccount {
        PlatformAccount {
            platform: platform.into(),
            host: "h".into(),
            username: username.into(),
            verified_at: None,
        }
    }

    #[test]
    fn https_tokens_use_the_username_each_platform_expects() {
        assert_eq!(
            basic_credentials(&account("github", "ignored"), "t".into()),
            Some(GitCredentials::Basic {
                username: "x-access-token".into(),
                password: "t".into()
            })
        );
        assert_eq!(
            basic_credentials(&account("gerrit", "alice"), "p".into()),
            Some(GitCredentials::Basic {
                username: "alice".into(),
                password: "p".into()
            })
        );
        assert_eq!(
            basic_credentials(&account("launchpad", "alice"), "x".into()),
            None
        );
    }
}
