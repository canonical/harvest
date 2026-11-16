use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use harvest_db::Db;

use super::repos::{check_remote, RepoAccess, Visibility};
use crate::cluster::lease::{self, Leases};
use crate::credentials::CredentialStore;

const STALE_AFTER_HOURS: i64 = 24;
const BATCH: i64 = 50;
const EVERY: Duration = Duration::from_secs(60 * 60);
const LEASE_KEY: &str = "repository-access-recheck";

pub async fn recheck_stale(db: &Db, credentials: &CredentialStore) -> Result<usize> {
    let rows = db
        .query(
            "SELECT a.user_id, r.name, r.url FROM repository_access a
             JOIN repositories r ON r.id = a.repository_id
             WHERE r.visibility = 'private' AND r.url IS NOT NULL
               AND a.verified_at < now() - make_interval(hours => $hours::int)
             ORDER BY a.verified_at
             LIMIT $limit",
            json!({ "hours": STALE_AFTER_HOURS, "limit": BATCH }),
        )
        .await?;
    let access = RepoAccess::new(db);
    for row in &rows {
        let (user_id, repo, url) = (
            row["user_id"].as_str().unwrap_or_default(),
            row["name"].as_str().unwrap_or_default(),
            row["url"].as_str().unwrap_or_default(),
        );
        let creds = credentials.git_credentials(user_id, url).await?;
        match check_remote(url, creds).await {
            Ok(check) if check.visibility == Visibility::Private => {
                access.grant(repo, user_id).await?
            }
            Ok(_) => {}
            Err(reason) => {
                tracing::info!(repo, user_id, %reason, "revoking repository access");
                access.revoke(repo, user_id).await?;
            }
        }
    }
    Ok(rows.len())
}

pub fn spawn(leases: Arc<dyn Leases>, db: Arc<Db>, credentials: Arc<CredentialStore>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let result = lease::run_exclusive(leases.as_ref(), LEASE_KEY, || async {
                if let Err(e) = recheck_stale(&db, &credentials).await {
                    tracing::warn!(error = %e, "repository access recheck failed");
                }
            })
            .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "repository access recheck could not run");
            }
        }
    });
}
