use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use super::chats;
use super::images::{self, quote};
use super::projects::{self, CodeProject};
use super::repo_git;
use super::{short_id, CodeEnv, WORKSPACE};

const IMAGE_WAIT: Duration = Duration::from_secs(40 * 60);
use crate::lxd::containers::{Command, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    Merged,
    Idle,
    Manual,
}

impl CloseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::Idle => "idle",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub user_id: String,
    pub branch: String,
    pub base_branch: String,
    pub container: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

const SELECT: &str = "SELECT s.id, s.project_id, s.user_id, s.branch, s.base_branch, s.container,
           s.status, s.error
    FROM code_sessions s";

fn from_row(row: &Value) -> Session {
    let text = |key: &str| row[key].as_str().map(String::from);
    Session {
        id: text("id").unwrap_or_default(),
        project_id: text("project_id").unwrap_or_default(),
        user_id: text("user_id").unwrap_or_default(),
        branch: text("branch").unwrap_or_default(),
        base_branch: text("base_branch").unwrap_or_default(),
        container: text("container"),
        status: text("status").unwrap_or_default(),
        error: text("error"),
    }
}

pub async fn get(env: &CodeEnv, id: &str) -> Result<Option<Session>> {
    let rows = env
        .db
        .query(&format!("{SELECT} WHERE s.id = $id"), json!({ "id": id }))
        .await?;
    Ok(rows.first().map(from_row))
}

pub async fn list_for(env: &CodeEnv, project_id: &str, user_id: &str) -> Result<Vec<Value>> {
    Ok(env
        .db
        .query(
            "SELECT s.id, s.branch, s.base_branch, s.status, s.close_reason, s.error,
                    s.created_at, s.last_active_at
             FROM code_sessions s
             WHERE s.project_id = $pid AND s.user_id = $uid
             ORDER BY s.status IN ('starting', 'active') DESC, s.last_active_at DESC",
            json!({ "pid": project_id, "uid": user_id }),
        )
        .await?)
}

pub fn is_valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with(['-', '/'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
}

pub async fn open(
    env: &CodeEnv,
    project: &CodeProject,
    user_id: &str,
    branch: &str,
    base_branch: &str,
) -> Result<Session> {
    if !is_valid_branch(branch) || !is_valid_branch(base_branch) {
        bail!("branch names may only use letters, digits, '-', '_', '/' and '.'");
    }
    let open = env
        .db
        .query(
            &format!(
                "{SELECT} WHERE s.project_id = $pid AND s.user_id = $uid AND s.branch = $branch
                   AND s.status IN ('starting', 'active')"
            ),
            json!({ "pid": project.id, "uid": user_id, "branch": branch }),
        )
        .await?;
    if let Some(row) = open.first() {
        return Ok(from_row(row));
    }
    let id = Uuid::new_v4().to_string();
    env.db
        .execute(
            "INSERT INTO code_sessions (id, project_id, user_id, branch, base_branch, status)
             VALUES ($id, $pid, $uid, $branch, $base, 'starting')",
            json!({ "id": id, "pid": project.id, "uid": user_id, "branch": branch, "base": base_branch }),
        )
        .await?;
    chats::create(&env.db, &id, user_id, branch).await?;
    projects::touch(&env.db, &project.id).await?;
    let started = {
        let env = env.clone();
        let project = project.clone();
        let id = id.clone();
        async move {
            if let Err(e) = start(&env, &project, &id).await {
                tracing::warn!(session = %id, error = %e, "code session failed to start");
                let _ = fail(&env, &id, &e.to_string()).await;
            }
        }
    };
    tokio::spawn(started);
    get(env, &id).await?.context("the session was not created")
}

async fn fail(env: &CodeEnv, id: &str, error: &str) -> Result<()> {
    if let Some(session) = get(env, id).await? {
        if let Some(container) = &session.container {
            release(env, container).await;
        }
    }
    env.db
        .execute(
            "UPDATE code_sessions SET status = 'failed', error = $error, container = NULL WHERE id = $id",
            json!({ "id": id, "error": images::tail(error, 2_000) }),
        )
        .await?;
    Ok(())
}

async fn start(env: &CodeEnv, project: &CodeProject, id: &str) -> Result<()> {
    let session = get(env, id).await?.context("session vanished")?;
    let container = match claim(env, &project.id).await? {
        Some(claimed) => claimed,
        None => launch_from_image(env, project).await?,
    };
    env.db
        .execute(
            "UPDATE code_sessions SET container = $container WHERE id = $id",
            json!({ "id": id, "container": container.name }),
        )
        .await?;
    checkout(env, project, &session, &container).await?;
    env.db
        .execute(
            "UPDATE code_sessions SET status = 'active', last_active_at = now() WHERE id = $id AND status = 'starting'",
            json!({ "id": id }),
        )
        .await?;
    super::preparation::spawn_for_session(env, id);
    Ok(())
}

pub struct Claimed {
    pub name: String,
    pub base_commit: Option<String>,
}

async fn claim(env: &CodeEnv, project_id: &str) -> Result<Option<Claimed>> {
    let rows = env
        .db
        .query(
            "UPDATE code_containers SET role = 'session', state = 'claimed', last_active_at = now()
             WHERE name = (
                 SELECT name FROM code_containers
                 WHERE project_id = $pid AND role = 'pool' AND state = 'ready'
                 ORDER BY created_at
                 FOR UPDATE SKIP LOCKED
                 LIMIT 1
             )
             RETURNING name, base_commit",
            json!({ "pid": project_id }),
        )
        .await?;
    Ok(rows.first().map(|row| Claimed {
        name: row["name"].as_str().unwrap_or_default().to_string(),
        base_commit: row["base_commit"].as_str().map(String::from),
    }))
}

async fn launch_from_image(env: &CodeEnv, project: &CodeProject) -> Result<Claimed> {
    let image = ready_image(env, project).await?;
    let name = container_name("hcs", &project.id);
    env.db
        .execute(
            "INSERT INTO code_containers (name, project_id, role, state, base_commit)
             VALUES ($name, $pid, 'session', 'claimed', $base)",
            json!({ "name": name, "pid": project.id, "base": image.base_commit }),
        )
        .await?;
    env.lxd
        .launch(&name, Source::Image(&image.alias), &[("project", &project.id)])
        .await?;
    Ok(Claimed {
        name,
        base_commit: image.base_commit,
    })
}

async fn ready_image(env: &CodeEnv, project: &CodeProject) -> Result<images::Image> {
    let deadline = tokio::time::Instant::now() + IMAGE_WAIT;
    loop {
        match images::current(env, &project.id).await? {
            Some(image) if image.status == "ready" => return Ok(image),
            Some(image) if image.is_building() && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Some(image) if image.is_building() => bail!("the project's containers are still being prepared"),
            _ => images::build(env, project).await?,
        }
    }
}

pub fn container_name(prefix: &str, project_id: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        short_id(project_id),
        short_id(&Uuid::new_v4().simple().to_string())
    )
}

async fn checkout(env: &CodeEnv, project: &CodeProject, session: &Session, container: &Claimed) -> Result<()> {
    let credentials = env.git_credentials(&session.user_id, &project.url).await;
    let clone = repo_git::local_clone(&project.name, &project.url, credentials).await?;
    let source = if repo_git::remote_commit(&clone, &session.branch).is_ok() {
        session.branch.clone()
    } else {
        session.base_branch.clone()
    };
    let bundle = {
        let clone = clone.clone();
        let base = container.base_commit.clone();
        let source = source.clone();
        tokio::task::spawn_blocking(move || repo_git::bundle_since(&clone, base.as_deref(), &source)).await??
    };
    let source_ref = format!("refs/remotes/origin/{source}");
    if let Some(bundle) = bundle {
        env.lxd
            .push_file(&container.name, "/tmp/update.bundle", bundle, 0o600)
            .await?;
        let fetch = format!(
            "git fetch -q /tmp/update.bundle {r}:{r} && rm /tmp/update.bundle",
            r = quote(&source_ref)
        );
        run_checked(env, &container.name, &fetch).await?;
    }
    let author = author(env, &session.user_id).await?;
    let script = format!(
        "git checkout -q -B {branch} {source} && git config user.name {name} && git config user.email {email}",
        branch = quote(&session.branch),
        source = quote(&source_ref),
        name = quote(&author.0),
        email = quote(&author.1),
    );
    run_checked(env, &container.name, &script).await?;
    super::change_requests::prepare_container(env, session, &project.url, &container.name).await?;
    env.lxd
        .wait_for_ipv4(&container.name, Duration::from_secs(60))
        .await?;
    Ok(())
}

async fn author(env: &CodeEnv, user_id: &str) -> Result<(String, String)> {
    let rows = env
        .db
        .query("SELECT name, email FROM users WHERE id = $uid", json!({ "uid": user_id }))
        .await?;
    let row = rows.first().context("unknown user")?;
    let email = row["email"].as_str().unwrap_or("harvest@localhost").to_string();
    let name = row["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&email).to_string();
    Ok((name, email))
}

pub async fn run_checked(env: &CodeEnv, container: &str, script: &str) -> Result<String> {
    let output = env
        .lxd
        .run(container, Command::shell(script).in_dir(WORKSPACE))
        .await?;
    if !output.succeeded() {
        bail!("{}", images::tail(&format!("{}{}", output.stdout, output.stderr), 2_000));
    }
    Ok(output.stdout)
}

async fn release(env: &CodeEnv, container: &str) {
    if let Err(e) = env.lxd.delete_instance(container).await {
        tracing::warn!(container, error = %e, "could not delete a code container");
    }
    let _ = env
        .db
        .execute("DELETE FROM code_containers WHERE name = $name", json!({ "name": container }))
        .await;
}

pub async fn close(env: &CodeEnv, id: &str, reason: CloseReason) -> Result<()> {
    let Some(session) = get(env, id).await? else {
        return Ok(());
    };
    super::previews::clones::release_session(env, id).await?;
    if let Some(container) = &session.container {
        release(env, container).await;
    }
    env.db
        .execute(
            "UPDATE code_sessions SET status = 'closed', close_reason = $reason, closed_at = now(), container = NULL
             WHERE id = $id AND status IN ('starting', 'active', 'failed')",
            json!({ "id": id, "reason": reason.as_str() }),
        )
        .await?;
    Ok(())
}

pub async fn layout(env: &CodeEnv, id: &str) -> Result<Option<Value>> {
    let rows = env
        .db
        .query("SELECT layout FROM code_sessions WHERE id = $id", json!({ "id": id }))
        .await?;
    Ok(rows.first().map(|r| r["layout"].clone()).filter(|l| !l.is_null()))
}

pub async fn save_layout(env: &CodeEnv, id: &str, layout: &Value) -> Result<()> {
    env.db
        .execute(
            "UPDATE code_sessions SET layout = $layout::jsonb WHERE id = $id",
            json!({ "id": id, "layout": layout }),
        )
        .await?;
    Ok(())
}

pub async fn touch(env: &CodeEnv, id: &str) -> Result<()> {
    env.db
        .execute(
            "UPDATE code_sessions SET last_active_at = now() WHERE id = $id",
            json!({ "id": id }),
        )
        .await?;
    Ok(())
}

pub async fn active_container(env: &CodeEnv, id: &str) -> Result<String> {
    let session = get(env, id).await?.context("the session no longer exists")?;
    match (session.status.as_str(), session.container) {
        ("active", Some(container)) => Ok(container),
        ("starting", _) => bail!("the session is still starting; try again in a moment"),
        (status, _) => bail!("the session is {status}; open a new session to keep working"),
    }
}

pub async fn idle_sessions(env: &CodeEnv) -> Result<Vec<String>> {
    let rows = env
        .db
        .query(
            "SELECT id FROM code_sessions
             WHERE status = 'active' AND last_active_at < now() - make_interval(secs => $secs::float8)",
            json!({ "secs": env.settings.session_idle.as_secs_f64() }),
        )
        .await?;
    Ok(rows.iter().filter_map(|r| r["id"].as_str().map(String::from)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_reject_shell_and_git_tricks() {
        for good in ["main", "feat/login-page", "release-2.1", "user_x/fix.1"] {
            assert!(is_valid_branch(good), "{good}");
        }
        for bad in ["", "-x", "a..b", "x/", "a b", "x;rm", "$(id)", "feat/"] {
            assert!(!is_valid_branch(bad), "{bad}");
        }
    }

    #[test]
    fn close_reasons_match_the_schema() {
        let names: Vec<_> = [CloseReason::Merged, CloseReason::Idle, CloseReason::Manual]
            .iter()
            .map(|r| r.as_str())
            .collect();
        assert_eq!(names, ["merged", "idle", "manual"]);
    }
}
