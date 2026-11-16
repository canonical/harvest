use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use knowledge_harvester::config::RepoConfig;
use knowledge_harvester::git::{GitClient, GitCredentials};

fn clone_root() -> PathBuf {
    std::env::temp_dir().join("harvest-repos")
}

pub async fn local_clone(name: &str, url: &str, credentials: GitCredentials) -> Result<PathBuf> {
    let config = RepoConfig {
        name: name.to_string(),
        url: url.to_string(),
        browse_url: None,
        refs: None,
    };
    tokio::task::spawn_blocking(move || {
        GitClient::new(clone_root())
            .with_credentials(credentials)
            .ensure_cloned(&config)
    })
    .await?
}

fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .context("running git")?;
    if !output.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

pub fn remote_commit(repo: &Path, branch: &str) -> Result<String> {
    let out = git(repo, &["rev-parse", &format!("refs/remotes/origin/{branch}")])?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

pub fn archive(repo: &Path) -> Result<Vec<u8>> {
    let output = Command::new("tar")
        .arg("-C")
        .arg(repo)
        .args(["-czf", "-", "."])
        .output()
        .context("archiving the repository")?;
    if !output.status.success() {
        bail!("tar: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

pub fn bundle_since(repo: &Path, base_commit: Option<&str>, branch: &str) -> Result<Option<Vec<u8>>> {
    let target = format!("refs/remotes/origin/{branch}");
    let tip = remote_commit(repo, branch)?;
    if base_commit == Some(tip.as_str()) {
        return Ok(None);
    }
    let range = match base_commit {
        Some(base) if is_ancestor(repo, base, &tip) => format!("{base}..{target}"),
        _ => target,
    };
    Ok(Some(git(repo, &["bundle", "create", "-", &range])?))
}

fn is_ancestor(repo: &Path, ancestor: &str, descendant: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
