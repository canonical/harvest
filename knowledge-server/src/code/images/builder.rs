use std::time::Duration;

use anyhow::{bail, Result};

use super::claim::BuildClaim;
use super::{quote, setup, tail};
use crate::code::projects::CodeProject;
use crate::code::{repo_git, terminals, CodeEnv, WORKSPACE};
use crate::lxd::containers::{Command, ExecOutput, Source};

const PACKAGE_INDEX_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(90);

pub struct Built {
    pub base_commit: String,
    pub log: String,
    pub setup_warning: Option<String>,
}

pub async fn build_in(env: &CodeEnv, project: &CodeProject, claim: &BuildClaim) -> Result<Built> {
    let owner = project.created_by.clone().unwrap_or_default();
    let credentials = env.git_credentials(&owner, &project.url).await;
    let clone = repo_git::local_clone(&project.name, &project.url, credentials).await?;
    let branch = project.default_branch.clone().unwrap_or_else(|| "main".into());
    let base_commit = repo_git::remote_commit(&clone, &branch)?;
    let archive = tokio::task::spawn_blocking(move || repo_git::archive(&clone)).await??;

    let builder = claim.builder_name();
    let labels = [("project", project.id.as_str()), ("attempt", claim.attempt_id.as_str())];
    env.lxd.launch(&builder, Source::Base, &labels).await?;
    env.lxd.wait_for_ipv4(&builder, NETWORK_TIMEOUT).await?;
    let refresh = Command::shell("apt-get update -q").timeout(PACKAGE_INDEX_TIMEOUT);
    expect_success(env.lxd.run(&builder, refresh).await?, "refreshing the package index")?;
    let terminals = Command::shell(&terminals::tmux::prepare_script()).timeout(PACKAGE_INDEX_TIMEOUT);
    expect_success(env.lxd.run(&builder, terminals).await?, "preparing terminals")?;
    env.lxd.push_file(&builder, "/tmp/repo.tgz", archive, 0o600).await?;
    expect_success(env.lxd.run(&builder, Command::shell(&unpack_script(project, &branch))).await?, "unpacking the repository")?;

    let steps = setup::plan_for(env, project, &builder).await?;
    let report = setup::run(env, &builder, &steps).await?;

    env.lxd.stop_instance(&builder).await?;
    env.lxd.publish(&builder, &claim.image_alias()).await?;
    Ok(Built {
        base_commit,
        log: report.log,
        setup_warning: report.warning,
    })
}

fn unpack_script(project: &CodeProject, branch: &str) -> String {
    format!(
        "mkdir -p {WORKSPACE} && tar --no-same-owner -xzf /tmp/repo.tgz -C {WORKSPACE} && rm /tmp/repo.tgz && \
         cd {WORKSPACE} && git checkout -q -B {branch} origin/{branch} && git remote set-url origin {url}",
        branch = quote(branch),
        url = quote(&project.url),
    )
}

fn expect_success(output: ExecOutput, what: &str) -> Result<()> {
    if !output.succeeded() {
        bail!("{what} failed: {}", tail(&format!("{}{}", output.stdout, output.stderr), 2_000));
    }
    Ok(())
}
