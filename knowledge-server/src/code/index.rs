use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use harvest_db::Db;

use super::projects::{self, CodeProject};
use super::{repo_git, sessions, CodeEnv};
use crate::api::repositories::{start_ingestion, IngestionRefused, IngestionSource};
use crate::api::GraphState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IndexedVersion {
    pub repo: String,
    pub version: String,
    pub commit: Option<String>,
    pub ingested: bool,
    pub error: Option<String>,
}

impl IndexedVersion {
    pub fn describes(&self, repo: &str, version: Option<&str>) -> bool {
        self.repo == repo && version.map_or(true, |v| v == self.version)
    }
}

pub async fn of_project(db: &Db, project: &CodeProject) -> Result<Option<IndexedVersion>> {
    let Some(branch) = &project.default_branch else {
        return Ok(None);
    };
    let rows = db
        .query(
            "SELECT v.commit_sha, v.ingested, p.index_error
             FROM code_projects p
             JOIN repositories r ON r.id = p.repository_id
             LEFT JOIN versions v ON v.repository_id = r.id AND v.tag = $tag
             WHERE p.id = $pid",
            json!({ "pid": project.id, "tag": branch }),
        )
        .await?;
    let row = rows.first();
    let text = |key: &str| row.and_then(|r: &Value| r[key].as_str()).map(String::from);
    Ok(Some(IndexedVersion {
        repo: project.name.clone(),
        version: branch.clone(),
        commit: text("commit_sha"),
        ingested: row.and_then(|r| r["ingested"].as_bool()).unwrap_or(false),
        error: text("index_error"),
    }))
}

pub async fn of_session(env: &CodeEnv, session_id: &str) -> Result<Option<IndexedVersion>> {
    let session = sessions::get(env, session_id).await?.context("the session no longer exists")?;
    let project = projects::get(&env.db, &session.project_id)
        .await?
        .context("the project no longer exists")?;
    of_project(&env.db, &project).await
}

pub async fn file_count(db: &Db, indexed: &IndexedVersion) -> Result<i64> {
    let rows = db
        .query(
            "SELECT count(*) AS files FROM files f
             JOIN versions v ON v.id = f.version_id
             JOIN repositories r ON r.id = v.repository_id
             WHERE r.name = $repo AND v.tag = $version",
            json!({ "repo": indexed.repo, "version": indexed.version }),
        )
        .await?;
    Ok(rows.first().and_then(|row| row["files"].as_i64()).unwrap_or(0))
}

pub fn needs_reindex(indexed: &IndexedVersion, remote_commit: &str) -> bool {
    indexed.commit.as_deref() != Some(remote_commit)
}

pub async fn refresh_due(env: &CodeEnv, graph: &GraphState) -> Result<()> {
    let due = env
        .db
        .query(
            "SELECT id FROM code_projects
             WHERE last_used_at > now() - make_interval(secs => $warm::float8)
               AND (index_checked_at IS NULL OR index_checked_at < now() - make_interval(secs => $every::float8))",
            json!({
                "warm": env.settings.warm_for.as_secs_f64(),
                "every": env.settings.reindex_every.as_secs_f64(),
            }),
        )
        .await?;
    for row in &due {
        let id = row["id"].as_str().unwrap_or_default();
        let outcome = refresh(env, graph, id).await;
        if let Err(e) = &outcome {
            tracing::warn!(project = id, error = %e, "could not refresh a code project's index");
        }
        record_outcome(&env.db, id, outcome.err().map(|e| e.to_string())).await?;
    }
    Ok(())
}

async fn refresh(env: &CodeEnv, graph: &GraphState, project_id: &str) -> Result<()> {
    let project = projects::get(&env.db, project_id).await?.context("the project no longer exists")?;
    let Some(indexed) = of_project(&env.db, &project).await? else {
        return Ok(());
    };
    let user = indexing_user(&env.db, &project).await?.context("nobody has used this project yet")?;
    let credentials = env.git_credentials(&user, &project.url).await;
    let clone = repo_git::local_clone(&project.name, &project.url, credentials.clone()).await?;
    let branch = indexed.version.clone();
    let remote = tokio::task::spawn_blocking(move || repo_git::remote_commit(&clone, &branch)).await??;
    if !needs_reindex(&indexed, &remote) {
        return Ok(());
    }
    let source = IngestionSource {
        name: project.name.clone(),
        url: project.url.clone(),
        browse_url: None,
        refs: vec![indexed.version.clone()],
        credentials,
        force: true,
    };
    match start_ingestion(graph, source).await {
        Ok(()) | Err(IngestionRefused::AlreadyRunning) => Ok(()),
        Err(IngestionRefused::Storage(e)) => anyhow::bail!(e),
    }
}

async fn indexing_user(db: &Db, project: &CodeProject) -> Result<Option<String>> {
    let rows = db
        .query(
            "SELECT user_id FROM code_sessions WHERE project_id = $pid ORDER BY created_at DESC LIMIT 1",
            json!({ "pid": project.id }),
        )
        .await?;
    Ok(rows
        .first()
        .and_then(|r| r["user_id"].as_str().map(String::from))
        .or_else(|| project.created_by.clone()))
}

async fn record_outcome(db: &Db, project_id: &str, error: Option<String>) -> Result<()> {
    db.execute(
        "UPDATE code_projects SET index_checked_at = now(), index_error = $error WHERE id = $pid",
        json!({ "pid": project_id, "error": error }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed(commit: Option<&str>) -> IndexedVersion {
        IndexedVersion {
            repo: "app".into(),
            version: "main".into(),
            commit: commit.map(String::from),
            ingested: true,
            error: None,
        }
    }

    #[test]
    fn a_moved_or_unknown_commit_needs_reindexing() {
        assert!(!needs_reindex(&indexed(Some("abc")), "abc"));
        assert!(needs_reindex(&indexed(Some("abc")), "def"));
        assert!(needs_reindex(&indexed(None), "abc"));
    }

    #[test]
    fn the_index_describes_its_own_repository_and_version() {
        let index = indexed(None);
        assert!(index.describes("app", Some("main")));
        assert!(index.describes("app", None));
        assert!(!index.describes("app", Some("v1.0")));
        assert!(!index.describes("other", Some("main")));
    }
}
