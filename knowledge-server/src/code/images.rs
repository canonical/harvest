use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use super::projects::CodeProject;
use super::repo_git;
use super::{short_id, CodeEnv, WORKSPACE};
use crate::lxd::containers::{Command, Source};

pub const REPO_SETUP_FILE: &str = ".harvest/setup.sh";
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const LOG_TAIL_CHARS: usize = 8_000;
const STALE_BUILD_SECS: u64 = SETUP_TIMEOUT.as_secs() + 15 * 60;
const PACKAGE_INDEX_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RECIPE_VERSION: u32 = 2;

pub fn alias_for(project_id: &str) -> String {
    format!("harvest-code-{}", short_id(project_id))
}

fn builder_for(project_id: &str) -> String {
    format!("hcb-{}", short_id(project_id))
}

pub fn setup_hash(script: Option<&str>) -> String {
    let mut hasher = DefaultHasher::new();
    RECIPE_VERSION.hash(&mut hasher);
    script.unwrap_or_default().trim().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max)).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub alias: String,
    pub status: String,
    pub base_commit: Option<String>,
    pub setup_hash: String,
    pub stale: bool,
}

impl Image {
    pub fn is_building(&self) -> bool {
        self.status == "building" && !self.stale
    }
}

pub async fn current(env: &CodeEnv, project_id: &str) -> Result<Option<Image>> {
    let rows = env
        .db
        .query(
            "SELECT alias, status, base_commit, setup_hash,
                    built_at < now() - make_interval(secs => $stale::float8) AS stale
             FROM code_images WHERE project_id = $pid",
            json!({ "pid": project_id, "stale": STALE_BUILD_SECS as f64 }),
        )
        .await?;
    Ok(rows.first().map(|row: &Value| Image {
        alias: row["alias"].as_str().unwrap_or_default().to_string(),
        status: row["status"].as_str().unwrap_or_default().to_string(),
        base_commit: row["base_commit"].as_str().map(String::from),
        setup_hash: row["setup_hash"].as_str().unwrap_or_default().to_string(),
        stale: row["stale"].as_bool().unwrap_or(false),
    }))
}

async fn record(env: &CodeEnv, project_id: &str, status: &str, fields: Value) -> Result<()> {
    let mut params = json!({ "pid": project_id, "alias": alias_for(project_id), "status": status });
    if let (Some(target), Some(source)) = (params.as_object_mut(), fields.as_object()) {
        target.extend(source.clone());
    }
    for key in ["base_commit", "setup_hash", "log"] {
        params.as_object_mut().unwrap().entry(key).or_insert(Value::Null);
    }
    env.db
        .execute(
            "INSERT INTO code_images (project_id, alias, status, base_commit, setup_hash, log, built_at)
             VALUES ($pid, $alias, $status, $base_commit, COALESCE($setup_hash, ''), COALESCE($log, ''), now())
             ON CONFLICT (project_id) DO UPDATE SET
                 status = EXCLUDED.status,
                 base_commit = COALESCE(EXCLUDED.base_commit, code_images.base_commit),
                 setup_hash = CASE WHEN $setup_hash::text IS NULL THEN code_images.setup_hash ELSE EXCLUDED.setup_hash END,
                 log = EXCLUDED.log,
                 built_at = now()",
            params,
        )
        .await?;
    Ok(())
}

pub async fn build(env: &CodeEnv, project: &CodeProject) -> Result<()> {
    record(env, &project.id, "building", json!({})).await?;
    let builder = builder_for(&project.id);
    let _ = env.lxd.delete_instance(&builder).await;
    let outcome = build_in(env, project, &builder).await;
    let _ = env.lxd.delete_instance(&builder).await;
    match outcome {
        Ok(built) => {
            record(
                env,
                &project.id,
                "ready",
                json!({ "base_commit": built.base_commit, "setup_hash": built.setup_hash, "log": built.log }),
            )
            .await
        }
        Err(e) => {
            tracing::warn!(project = %project.name, error = %e, "code image build failed");
            let fields = json!({
                "log": tail(&e.to_string(), LOG_TAIL_CHARS),
                "setup_hash": setup_hash(project.setup_script.as_deref()),
            });
            record(env, &project.id, "failed", fields).await?;
            Err(e)
        }
    }
}

struct Built {
    base_commit: String,
    setup_hash: String,
    log: String,
}

async fn build_in(env: &CodeEnv, project: &CodeProject, builder: &str) -> Result<Built> {
    let owner = project.created_by.clone().unwrap_or_default();
    let credentials = env.git_credentials(&owner, &project.url).await;
    let clone = repo_git::local_clone(&project.name, &project.url, credentials).await?;
    let branch = project.default_branch.clone().unwrap_or_else(|| "main".into());
    let base_commit = repo_git::remote_commit(&clone, &branch)?;
    let archive = {
        let clone = clone.clone();
        tokio::task::spawn_blocking(move || repo_git::archive(&clone)).await??
    };

    env.lxd.launch(builder, Source::Base, &[("project", &project.id)]).await?;
    env.lxd.wait_for_ipv4(builder, Duration::from_secs(90)).await?;
    expect_success(
        env.lxd
            .run(builder, Command::shell("apt-get update -q").timeout(PACKAGE_INDEX_TIMEOUT))
            .await?,
        "refreshing the package index",
    )?;
    env.lxd.push_file(builder, "/tmp/repo.tgz", archive, 0o600).await?;
    let unpack = format!(
        "mkdir -p {WORKSPACE} && tar --no-same-owner -xzf /tmp/repo.tgz -C {WORKSPACE} && rm /tmp/repo.tgz && \
         cd {WORKSPACE} && git checkout -q -B {branch} origin/{branch} && git remote set-url origin {url}",
        branch = quote(&branch),
        url = quote(&project.url),
    );
    expect_success(env.lxd.run(builder, Command::shell(&unpack)).await?, "unpacking the repository")?;

    let script = setup_script(env, project, builder).await?;
    let mut log = String::new();
    if let Some(script) = &script {
        env.lxd
            .push_file(builder, "/tmp/harvest-setup.sh", script.as_bytes().to_vec(), 0o700)
            .await?;
        let output = env
            .lxd
            .run(
                builder,
                Command::shell("bash /tmp/harvest-setup.sh").in_dir(WORKSPACE).timeout(SETUP_TIMEOUT),
            )
            .await?;
        log = tail(&format!("{}{}", output.stdout, output.stderr), LOG_TAIL_CHARS);
        if !output.succeeded() {
            bail!("the setup script failed with exit code {}:\n{log}", output.exit_code);
        }
    }

    env.lxd.stop_instance(builder).await?;
    env.lxd.publish(builder, &alias_for(&project.id)).await?;
    Ok(Built {
        base_commit,
        setup_hash: setup_hash(project.setup_script.as_deref()),
        log,
    })
}

async fn setup_script(env: &CodeEnv, project: &CodeProject, container: &str) -> Result<Option<String>> {
    let from_repo = env
        .lxd
        .run(container, Command::shell(&format!("cat {REPO_SETUP_FILE}")).in_dir(WORKSPACE))
        .await?;
    if from_repo.succeeded() && !from_repo.stdout.trim().is_empty() {
        return Ok(Some(from_repo.stdout));
    }
    Ok(project.setup_script.clone().filter(|s| !s.trim().is_empty()))
}

fn expect_success(output: crate::lxd::containers::ExecOutput, what: &str) -> Result<()> {
    if !output.succeeded() {
        bail!("{what} failed: {}", tail(&format!("{}{}", output.stdout, output.stderr), 2_000));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_keeps_single_quotes_inside_a_shell_word() {
        assert_eq!(quote("feat/x"), "'feat/x'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn the_setup_hash_ignores_surrounding_whitespace_only() {
        assert_eq!(setup_hash(Some("make\n")), setup_hash(Some("make")));
        assert_ne!(setup_hash(Some("make")), setup_hash(Some("make test")));
        assert_eq!(setup_hash(None), setup_hash(Some("")));
    }
}
