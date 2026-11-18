use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use harvest_db::Db;

use crate::access::repos::RepoScope;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CodeProject {
    pub id: String,
    pub name: String,
    pub url: String,
    pub visibility: String,
    pub default_branch: Option<String>,
    pub setup_script: Option<String>,
    pub checks: Option<String>,
    pub created_by: Option<String>,
}

const SELECT: &str = "SELECT p.id, r.name, r.url, r.visibility, r.default_branch, p.setup_script, p.checks, p.created_by
    FROM code_projects p JOIN repositories r ON r.id = p.repository_id";

fn from_row(row: &Value) -> CodeProject {
    let text = |key: &str| row[key].as_str().map(String::from);
    CodeProject {
        id: text("id").unwrap_or_default(),
        name: text("name").unwrap_or_default(),
        url: text("url").unwrap_or_default(),
        visibility: text("visibility").unwrap_or_else(|| "public".into()),
        default_branch: text("default_branch"),
        setup_script: text("setup_script"),
        checks: text("checks"),
        created_by: text("created_by"),
    }
}

pub async fn get(db: &Db, id: &str) -> Result<Option<CodeProject>> {
    let rows = db
        .query(&format!("{SELECT} WHERE p.id = $id"), json!({ "id": id }))
        .await?;
    Ok(rows.first().map(from_row))
}

pub async fn ensure(db: &Db, repo: &str, default_branch: Option<&str>, created_by: &str) -> Result<CodeProject> {
    db.execute(
        "UPDATE repositories SET default_branch = COALESCE($branch, default_branch) WHERE name = $repo",
        json!({ "repo": repo, "branch": default_branch }),
    )
    .await?;
    db.execute(
        "INSERT INTO code_projects (id, repository_id, created_by)
         SELECT $id, r.id, $uid FROM repositories r WHERE r.name = $repo
         ON CONFLICT (repository_id) DO UPDATE SET last_used_at = now()",
        json!({ "id": Uuid::new_v4().to_string(), "repo": repo, "uid": created_by }),
    )
    .await?;
    let rows = db
        .query(&format!("{SELECT} WHERE r.name = $repo"), json!({ "repo": repo }))
        .await?;
    rows.first().map(from_row).context("the code project was not created")
}

pub async fn list(db: &Db, scope: &RepoScope, user_id: &str) -> Result<Vec<Value>> {
    let mut params = json!({ "uid": user_id });
    scope.write_into(&mut params);
    let sql = format!(
        "SELECT p.id, r.name, r.url, r.visibility, r.default_branch,
                GREATEST(p.last_used_at, mine.last_active_at) AS last_used_at,
                CASE WHEN i.status = 'ready' THEN 'ready' ELSE i.attempt_status END AS image_status,
                coalesce(mine.open_sessions, 0) AS open_sessions
         FROM code_projects p
         JOIN repositories r ON r.id = p.repository_id
         LEFT JOIN code_images i ON i.project_id = p.id
         LEFT JOIN LATERAL (
             SELECT max(s.last_active_at) AS last_active_at,
                    count(*) FILTER (WHERE s.status IN ('starting', 'active')) AS open_sessions
             FROM code_sessions s
             WHERE s.project_id = p.id AND s.user_id = $uid
         ) mine ON true
         WHERE {}
         ORDER BY GREATEST(p.last_used_at, mine.last_active_at) DESC",
        RepoScope::sql_condition("r.name"),
    );
    Ok(db.query(&sql, params).await?)
}

pub async fn set_setup_script(db: &Db, id: &str, script: Option<&str>) -> Result<()> {
    db.execute(
        "UPDATE code_projects SET setup_script = $script WHERE id = $id",
        json!({ "id": id, "script": script.map(str::trim).filter(|s| !s.is_empty()) }),
    )
    .await?;
    Ok(())
}

pub async fn set_checks(db: &Db, id: &str, checks: Option<&str>) -> Result<()> {
    db.execute(
        "UPDATE code_projects SET checks = $checks WHERE id = $id",
        json!({ "id": id, "checks": checks.map(str::trim).filter(|s| !s.is_empty()) }),
    )
    .await?;
    Ok(())
}

pub async fn touch(db: &Db, id: &str) -> Result<()> {
    db.execute(
        "UPDATE code_projects SET last_used_at = now() WHERE id = $id",
        json!({ "id": id }),
    )
    .await?;
    Ok(())
}
