use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use uuid::Uuid;

use harvest_db::Db;

use super::failure::BuildFailure;
use super::state::STALE_BUILD;
use crate::code::short_id;

const ABANDONED_LOG: &str = "The build stopped before it finished, for example because the server restarted.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildClaim {
    pub project_id: String,
    pub attempt_id: String,
    pub setup_hash: String,
}

fn builder_name(project_id: &str, attempt_id: &str) -> String {
    format!("hcb-{}-{}", short_id(project_id), short_id(attempt_id))
}

impl BuildClaim {
    pub fn builder_name(&self) -> String {
        builder_name(&self.project_id, &self.attempt_id)
    }

    pub fn image_alias(&self) -> String {
        format!("harvest-code-{}-{}", short_id(&self.project_id), short_id(&self.attempt_id))
    }
}

pub struct Completion<'a> {
    pub base_commit: &'a str,
    pub log: &'a str,
    pub setup_warning: Option<&'a str>,
}

pub async fn try_claim(db: &Db, project_id: &str, setup_hash: &str) -> Result<Option<BuildClaim>> {
    let attempt_id = Uuid::new_v4().simple().to_string();
    let rows = db
        .query(
            "INSERT INTO code_images (project_id, alias, status, setup_hash, log, built_at,
                                      attempt_id, attempt_status, attempt_hash, attempted_at)
             VALUES ($pid, '', 'none', '', '', now(), $attempt, 'building', $hash, now())
             ON CONFLICT (project_id) DO UPDATE SET
                 attempt_id = EXCLUDED.attempt_id,
                 attempt_status = 'building',
                 attempt_failure = NULL,
                 attempt_hash = EXCLUDED.attempt_hash,
                 attempt_log = '',
                 attempted_at = now()
             WHERE code_images.attempt_status IS DISTINCT FROM 'building'
                OR code_images.attempted_at < now() - make_interval(secs => $stale::float8)
             RETURNING attempt_id",
            json!({
                "pid": project_id,
                "attempt": attempt_id,
                "hash": setup_hash,
                "stale": STALE_BUILD.as_secs_f64(),
            }),
        )
        .await?;
    let won = rows.first().and_then(|row| row["attempt_id"].as_str()) == Some(attempt_id.as_str());
    Ok(won.then(|| BuildClaim {
        project_id: project_id.to_string(),
        attempt_id,
        setup_hash: setup_hash.to_string(),
    }))
}

pub async fn complete(db: &Db, claim: &BuildClaim, completion: Completion<'_>) -> Result<Option<String>> {
    let rows = db
        .query(
            "UPDATE code_images AS image SET
                 status = 'ready',
                 alias = $alias,
                 base_commit = $base,
                 setup_hash = $hash,
                 log = $log,
                 setup_warning = $warning,
                 built_at = now(),
                 attempt_status = NULL,
                 attempt_failure = NULL,
                 attempt_log = ''
             FROM (SELECT alias AS previous_alias FROM code_images WHERE project_id = $pid) AS before
             WHERE image.project_id = $pid AND image.attempt_id = $attempt
             RETURNING before.previous_alias",
            json!({
                "pid": claim.project_id,
                "attempt": claim.attempt_id,
                "alias": claim.image_alias(),
                "base": completion.base_commit,
                "hash": claim.setup_hash,
                "log": completion.log,
                "warning": completion.setup_warning,
            }),
        )
        .await?;
    let previous = rows.first().and_then(|row| row["previous_alias"].as_str()).unwrap_or_default();
    Ok(replaced_alias(previous, &claim.image_alias()))
}

pub async fn fail(db: &Db, claim: &BuildClaim, failure: BuildFailure, log: &str) -> Result<()> {
    db.execute(
        "UPDATE code_images SET attempt_status = 'failed', attempt_failure = $failure, attempt_log = $log
         WHERE project_id = $pid AND attempt_id = $attempt",
        json!({
            "pid": claim.project_id,
            "attempt": claim.attempt_id,
            "failure": failure.as_str(),
            "log": log,
        }),
    )
    .await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveAttempt {
    pub project_id: String,
    pub attempt_id: String,
    pub age: Duration,
}

impl ActiveAttempt {
    pub fn builder_name(&self) -> String {
        builder_name(&self.project_id, &self.attempt_id)
    }
}

pub async fn active_attempts(db: &Db) -> Result<Vec<ActiveAttempt>> {
    let rows = db
        .query(
            "SELECT project_id, attempt_id, extract(epoch FROM now() - attempted_at)::float8 AS age
             FROM code_images
             WHERE attempt_status = 'building'
               AND attempted_at >= now() - make_interval(secs => $stale::float8)",
            json!({ "stale": STALE_BUILD.as_secs_f64() }),
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| ActiveAttempt {
            project_id: row["project_id"].as_str().unwrap_or_default().to_string(),
            attempt_id: row["attempt_id"].as_str().unwrap_or_default().to_string(),
            age: Duration::from_secs_f64(row["age"].as_f64().unwrap_or_default().max(0.0)),
        })
        .collect())
}

pub async fn abandon(db: &Db, attempt: &ActiveAttempt) -> Result<()> {
    db.execute(
        "UPDATE code_images SET attempt_status = 'failed', attempt_failure = $failure, attempt_log = $log
         WHERE project_id = $pid AND attempt_id = $attempt AND attempt_status = 'building'",
        json!({
            "pid": attempt.project_id,
            "attempt": attempt.attempt_id,
            "failure": BuildFailure::Infrastructure.as_str(),
            "log": ABANDONED_LOG,
        }),
    )
    .await?;
    Ok(())
}

fn replaced_alias(previous: &str, current: &str) -> Option<String> {
    (!previous.is_empty() && previous != current).then(|| previous.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim() -> BuildClaim {
        BuildClaim {
            project_id: "154f6dcf-b6e3-4994-b627-3bc6183f8ffe".into(),
            attempt_id: "0a1b2c3d4e5f60718293a4b5c6d7e8f9".into(),
            setup_hash: "h".into(),
        }
    }

    #[test]
    fn each_attempt_builds_in_its_own_container_and_publishes_its_own_image() {
        assert_eq!(claim().builder_name(), "hcb-154f6dcf-0a1b2c3d");
        assert_eq!(claim().image_alias(), "harvest-code-154f6dcf-0a1b2c3d");
    }

    async fn project(db: &Db) -> String {
        db.execute("INSERT INTO repositories (name, url) VALUES ('claimed', 'https://x/claimed')", json!({}))
            .await
            .unwrap();
        db.execute(
            "INSERT INTO code_projects (id, repository_id) SELECT 'p-claimed', id FROM repositories WHERE name = 'claimed'",
            json!({}),
        )
        .await
        .unwrap();
        "p-claimed".into()
    }

    fn completion() -> Completion<'static> {
        Completion { base_commit: "abc", log: "ok", setup_warning: None }
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn concurrent_claims_produce_one_build_and_completion_keeps_the_previous_alias_for_removal() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = &test_db.db;
        let project_id = project(db).await;

        let claims = futures::future::join_all((0..5).map(|_| try_claim(db, &project_id, "h"))).await;
        let winners: Vec<BuildClaim> = claims.into_iter().filter_map(|claim| claim.unwrap()).collect();
        assert_eq!(winners.len(), 1);
        assert_eq!(complete(db, &winners[0], completion()).await.unwrap(), None);

        let rebuild = try_claim(db, &project_id, "h2").await.unwrap().unwrap();
        assert_eq!(complete(db, &rebuild, completion()).await.unwrap(), Some(winners[0].image_alias()));
    }

    #[tokio::test]
    #[ignore = "needs HARVEST_TEST_DATABASE_URL"]
    async fn a_failed_attempt_frees_the_claim_and_a_superseded_attempt_changes_nothing() {
        let test_db = harvest_db::test_support::TestDb::new().await;
        let db = &test_db.db;
        let project_id = project(db).await;

        let first = try_claim(db, &project_id, "h").await.unwrap().unwrap();
        fail(db, &first, BuildFailure::LxdUnavailable, "refused").await.unwrap();
        let second = try_claim(db, &project_id, "h").await.unwrap().unwrap();
        assert_eq!(complete(db, &first, completion()).await.unwrap(), None);
        let row = &db.query("SELECT status, attempt_status FROM code_images", json!({})).await.unwrap()[0];
        assert_eq!(row["status"], "none");
        assert_eq!(row["attempt_status"], "building");
        assert_ne!(first.attempt_id, second.attempt_id);
    }

    #[test]
    fn only_a_different_earlier_image_is_replaced() {
        assert_eq!(replaced_alias("harvest-code-154f6dcf", "harvest-code-154f6dcf-0a1b2c3d"), Some("harvest-code-154f6dcf".into()));
        assert_eq!(replaced_alias("", "harvest-code-x"), None);
        assert_eq!(replaced_alias("harvest-code-x", "harvest-code-x"), None);
    }
}
