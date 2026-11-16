pub mod platforms;

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use knowledge_harvester::git::GitClient;

use super::images::quote;
use super::projects::{self, CodeProject};
use super::sessions::{self, CloseReason, Session};
use super::{repo_git, CodeEnv};
use crate::credentials::platform::Platform;
use crate::credentials::remote::RemoteUrl;
use platforms::{Account, GitPlatform, Proposal, Repository, Status};

const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);
const OUTGOING_BUNDLE: &str = "/tmp/harvest-out.bundle";

pub const CHANGE_ID_HOOK: &str = "#!/bin/sh\n\
grep -q '^Change-Id: I' \"$1\" && exit 0\n\
id=$(head -c 64 /dev/urandom | sha1sum | cut -c1-40)\n\
git interpret-trailers --in-place --trailer \"Change-Id: I$id\" \"$1\"\n";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChangeRequest {
    pub platform: String,
    pub number: String,
    pub url: String,
    pub state: String,
}

pub async fn for_session(env: &CodeEnv, session_id: &str) -> Result<Option<ChangeRequest>> {
    let rows = env
        .db
        .query(
            "SELECT platform, number, url, state FROM code_change_requests WHERE session_id = $sid",
            json!({ "sid": session_id }),
        )
        .await?;
    Ok(rows.first().map(|row: &Value| ChangeRequest {
        platform: row["platform"].as_str().unwrap_or_default().to_string(),
        number: row["number"].as_str().unwrap_or_default().to_string(),
        url: row["url"].as_str().unwrap_or_default().to_string(),
        state: row["state"].as_str().unwrap_or_default().to_string(),
    }))
}

pub async fn platform_of(env: &CodeEnv, user_id: &str, url: &str) -> Result<Option<Platform>> {
    let remote = RemoteUrl::parse(url).context("the repository URL is not a git remote")?;
    if let Some(platform) = Platform::for_host(&remote.host) {
        return Ok(Some(platform));
    }
    let Some(store) = &env.credentials else {
        return Ok(None);
    };
    Ok(store
        .account_for_host(user_id, &remote.host)
        .await?
        .and_then(|(account, _)| account.platform.parse().ok()))
}

async fn connect(env: &CodeEnv, user_id: &str, url: &str) -> Result<(Platform, Box<dyn GitPlatform>)> {
    let remote = RemoteUrl::parse(url).context("the repository URL is not a git remote")?;
    let platform = platform_of(env, user_id, url).await?.with_context(|| {
        format!("Harvest does not know which platform hosts {}; add an account for it in Settings → Git access", remote.host)
    })?;
    let store = env.credentials.as_ref().context("credentials are not configured on this server")?;
    let host = platforms::account_host(platform, &remote);
    let (account, secret) = store.account_for_host(user_id, &host).await?.with_context(|| {
        format!("add your {} account for {host} in Settings → Git access to open change requests", platform.label())
    })?;
    let connection = platforms::build(
        platform,
        Repository { url: url.to_string(), remote },
        Account { username: account.username, secret },
    );
    Ok((platform, connection))
}

pub async fn prepare_container(env: &CodeEnv, session: &Session, url: &str, container: &str) -> Result<()> {
    if platform_of(env, &session.user_id, url).await? != Some(Platform::Gerrit) {
        return Ok(());
    }
    env.lxd
        .push_file(container, "/workspace/.git/hooks/commit-msg", CHANGE_ID_HOOK.as_bytes().to_vec(), 0o755)
        .await
}

pub async fn submit(env: &CodeEnv, session_id: &str, title: &str, description: &str) -> Result<ChangeRequest> {
    let title = title.trim();
    if title.is_empty() {
        bail!("a change request needs a title");
    }
    let session = sessions::get(env, session_id).await?.context("the session no longer exists")?;
    let container = sessions::active_container(env, session_id).await?;
    let project = projects::get(&env.db, &session.project_id)
        .await?
        .context("the project no longer exists")?;
    let (platform, connection) = connect(env, &session.user_id, &project.url).await?;
    let bundle = export_work(env, &session, &container).await?;
    let proposal = Proposal {
        branch: &session.branch,
        base: &session.base_branch,
        title,
        description,
    };
    let messages = push(env, &session, &project, &*connection, &proposal, bundle).await?;
    if let Some(existing) = for_session(env, session_id).await?.filter(|cr| cr.state == "open") {
        return Ok(existing);
    }
    let opened = connection.open(&proposal, &messages).await?;
    env.db
        .execute(
            "INSERT INTO code_change_requests (session_id, platform, number, url)
             VALUES ($sid, $platform, $number, $url)
             ON CONFLICT (session_id) DO UPDATE SET
                 platform = EXCLUDED.platform, number = EXCLUDED.number, url = EXCLUDED.url,
                 state = 'open', created_at = now(), checked_at = now()",
            json!({ "sid": session_id, "platform": platform.as_str(), "number": opened.number, "url": opened.url }),
        )
        .await?;
    sessions::touch(env, session_id).await?;
    for_session(env, session_id).await?.context("the change request was not recorded")
}

async fn export_work(env: &CodeEnv, session: &Session, container: &str) -> Result<Vec<u8>> {
    let base = quote(&format!("refs/remotes/origin/{}", session.base_branch));
    let dirty = sessions::run_checked(env, container, "git status --porcelain").await?;
    if !dirty.trim().is_empty() {
        bail!("commit or discard the uncommitted changes first");
    }
    let ahead = sessions::run_checked(env, container, &format!("git rev-list --count {base}..HEAD")).await?;
    if ahead.trim() == "0" {
        bail!("there are no commits to propose yet");
    }
    sessions::run_checked(env, container, &format!("git bundle create {OUTGOING_BUNDLE} HEAD ^{base}")).await?;
    let bundle = env.lxd.pull_file(container, OUTGOING_BUNDLE).await?;
    let _ = sessions::run_checked(env, container, &format!("rm -f {OUTGOING_BUNDLE}")).await;
    Ok(bundle)
}

async fn push(
    env: &CodeEnv,
    session: &Session,
    project: &CodeProject,
    connection: &dyn GitPlatform,
    proposal: &Proposal<'_>,
    bundle: Vec<u8>,
) -> Result<String> {
    let fetch_credentials = env.git_credentials(&session.user_id, &project.url).await;
    let clone = repo_git::local_clone(&project.name, &project.url, fetch_credentials).await?;
    let target = connection.push_target(proposal);
    let push_credentials = env.git_credentials(&session.user_id, &target.url).await;
    let local_ref = format!("refs/harvest/{}", session.id);
    let refspec = format!("{local_ref}:{}", target.destination);
    tokio::task::spawn_blocking(move || -> Result<String> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), &bundle)?;
        import_bundle(&clone, file.path(), &local_ref)?;
        GitClient::new(clone.parent().map(Path::to_path_buf).unwrap_or_default())
            .with_credentials(push_credentials)
            .push(&clone, &target.url, &refspec)
    })
    .await?
}

fn import_bundle(clone: &Path, bundle: &Path, local_ref: &str) -> Result<()> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(clone)
        .arg("fetch")
        .arg("-q")
        .arg(bundle)
        .arg(format!("+HEAD:{local_ref}"))
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!("importing the session's commits failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

pub async fn refresh(env: &CodeEnv) -> Result<()> {
    let due = env
        .db
        .query(
            "SELECT c.session_id, c.number, s.user_id, r.url
             FROM code_change_requests c
             JOIN code_sessions s ON s.id = c.session_id
             JOIN code_projects p ON p.id = s.project_id
             JOIN repositories r ON r.id = p.repository_id
             WHERE c.state = 'open' AND c.checked_at < now() - make_interval(secs => $secs::float8)",
            json!({ "secs": CHECK_EVERY.as_secs_f64() }),
        )
        .await?;
    for row in &due {
        let text = |key: &str| row[key].as_str().unwrap_or_default();
        let session_id = text("session_id");
        if let Err(e) = check(env, session_id, text("number"), text("user_id"), text("url")).await {
            tracing::warn!(session = session_id, error = %e, "could not check a change request");
        }
    }
    Ok(())
}

async fn check(env: &CodeEnv, session_id: &str, number: &str, user_id: &str, url: &str) -> Result<()> {
    env.db
        .execute(
            "UPDATE code_change_requests SET checked_at = now() WHERE session_id = $sid",
            json!({ "sid": session_id }),
        )
        .await?;
    let (_, connection) = connect(env, user_id, url).await?;
    let status = connection.status(number).await?;
    if status == Status::Open {
        return Ok(());
    }
    env.db
        .execute(
            "UPDATE code_change_requests SET state = $state WHERE session_id = $sid",
            json!({ "sid": session_id, "state": status.as_str() }),
        )
        .await?;
    if status == Status::Merged {
        sessions::close(env, session_id, CloseReason::Merged).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_change_id_hook_adds_a_trailer_only_once() {
        assert!(CHANGE_ID_HOOK.starts_with("#!/bin/sh\n"));
        assert!(CHANGE_ID_HOOK.contains("grep -q '^Change-Id: I'"));
        assert!(CHANGE_ID_HOOK.contains("interpret-trailers --in-place --trailer \"Change-Id: I$id\""));
    }
}
