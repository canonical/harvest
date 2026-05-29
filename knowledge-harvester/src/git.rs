use anyhow::{Context, Result};
use git2::{build::RepoBuilder, FetchOptions, Repository};
use std::path::{Path, PathBuf};

use crate::config::RepoConfig;

#[derive(Debug, Clone)]
pub struct TagInfo {
    pub name: String,
    pub commit_sha: String,
    pub timestamp: i64,
}

pub struct GitClient {
    clone_root: PathBuf,
}

impl GitClient {
    pub fn new(clone_root: PathBuf) -> Self {
        Self { clone_root }
    }

    pub fn repo_path(&self, repo_name: &str) -> PathBuf {
        self.clone_root.join(repo_name)
    }

    pub fn ensure_cloned(&self, config: &RepoConfig) -> Result<PathBuf> {
        let path = self.repo_path(&config.name);
        if path.exists() {
            self.fetch(&path)
                .with_context(|| format!("fetching {}", config.name))?;
        } else {
            tracing::info!(repo = config.name, url = config.url, "cloning");
            RepoBuilder::new()
                .fetch_options(FetchOptions::new())
                .clone(&config.url, &path)
                .with_context(|| format!("cloning {}", config.url))?;
        }
        Ok(path)
    }

    pub fn fetch(&self, repo_path: &Path) -> Result<()> {
        let repo = Repository::open(repo_path)?;
        for remote_name in repo.remotes()?.iter().flatten() {
            repo.find_remote(remote_name)?.fetch(
                &[] as &[&str],
                Some(&mut FetchOptions::new()),
                None,
            )?;
        }
        Ok(())
    }

    pub fn list_tags(&self, repo_path: &Path) -> Result<Vec<TagInfo>> {
        let repo = Repository::open(repo_path)?;
        let mut tags = Vec::new();

        repo.tag_foreach(|oid, name_bytes| {
            let name = String::from_utf8_lossy(name_bytes)
                .trim_start_matches("refs/tags/")
                .to_string();

            if let Ok(obj) = repo.find_object(oid, None) {
                if let Ok(commit) = obj.peel_to_commit() {
                    tags.push(TagInfo {
                        name,
                        commit_sha: commit.id().to_string(),
                        timestamp: commit.time().seconds(),
                    });
                }
            }
            true
        })?;

        tags.sort_by_key(|t| t.timestamp);
        Ok(tags)
    }

    /// Resolve a list of git ref names (tags, branches, commit SHAs) to
    /// `TagInfo` values. Returns an error if any ref cannot be found.
    pub fn resolve_refs(&self, repo_path: &Path, refs: &[String]) -> Result<Vec<TagInfo>> {
        let repo = Repository::open(repo_path)?;
        let mut out = Vec::with_capacity(refs.len());
        for refname in refs {
            let obj = resolve_one(&repo, refname)
                .with_context(|| format!("resolving ref '{refname}'"))?;
            let commit = obj
                .peel_to_commit()
                .with_context(|| format!("peeling '{refname}' to commit"))?;
            out.push(TagInfo {
                name: refname.clone(),
                commit_sha: commit.id().to_string(),
                timestamp: commit.time().seconds(),
            });
        }
        Ok(out)
    }

    pub fn checkout(&self, repo_path: &Path, refname: &str) -> Result<()> {
        let repo = Repository::open(repo_path)?;
        let obj = resolve_one(&repo, refname)
            .with_context(|| format!("resolving ref '{refname}'"))?;
        repo.checkout_tree(&obj, None)?;
        repo.set_head_detached(obj.peel_to_commit()?.id())?;
        Ok(())
    }

    pub fn walk_source_files(&self, root: &Path) -> Result<Vec<PathBuf>> {
        use ignore::WalkBuilder;
        let files = WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .filter_entry(|e| e.file_name() != ".git")
            .build()
            .filter_map(|entry| entry.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .map(|e| e.into_path())
            .collect();
        Ok(files)
    }
}

/// Resolve a ref name to a git object using git's DWIM rules, with an
/// additional fallback for remote-tracking branches on `origin`.
///
/// git2 DWIM tries (among others): refs/tags/<n>, refs/heads/<n>,
/// refs/remotes/<n>. That last step treats the first path segment as the
/// remote name, so a slashed branch like "stable/2023.1.1" would look for
/// remote "stable", branch "2023.1.1" — which is wrong for an `origin`-
/// cloned repo. We fall back to refs/remotes/origin/<n> explicitly.
fn resolve_one<'r>(repo: &'r Repository, refname: &str) -> Result<git2::Object<'r>> {
    if let Ok(obj) = repo.revparse_single(refname) {
        return Ok(obj);
    }
    let origin_ref = format!("refs/remotes/origin/{refname}");
    repo.revparse_single(&origin_ref)
        .with_context(|| format!("resolving ref '{refname}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{Repository, Signature, Time};
    use tempfile::TempDir;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn sig() -> Signature<'static> {
        Signature::new("Test", "test@example.com", &Time::new(1_000_000, 0)).unwrap()
    }

    /// Repo with two commits:
    ///   commit A → lightweight tag "v1.0"
    ///   commit B → branch "develop"
    fn make_repo() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let repo = Repository::init(&path).unwrap();

        let empty_tree = {
            let mut idx = repo.index().unwrap();
            let oid = idx.write_tree().unwrap();
            repo.find_tree(oid).unwrap()
        };

        let c1_oid = repo
            .commit(Some("HEAD"), &sig(), &sig(), "Initial", &empty_tree, &[])
            .unwrap();
        let c1 = repo.find_commit(c1_oid).unwrap();
        repo.tag_lightweight("v1.0", c1.as_object(), false).unwrap();

        let c2_oid = repo
            .commit(Some("HEAD"), &sig(), &sig(), "Second", &empty_tree, &[&c1])
            .unwrap();
        let c2 = repo.find_commit(c2_oid).unwrap();
        repo.branch("develop", &c2, false).unwrap();

        (dir, path)
    }

    // ── resolve_refs ──────────────────────────────────────────────────────────

    #[test]
    fn resolve_refs_resolves_tag() {
        let (_dir, repo_path) = make_repo();
        let client = GitClient::new(PathBuf::from("/tmp"));
        let refs = client.resolve_refs(&repo_path, &["v1.0".to_string()]).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "v1.0");
        assert!(!refs[0].commit_sha.is_empty());
    }

    #[test]
    fn resolve_refs_resolves_branch() {
        let (_dir, repo_path) = make_repo();
        let client = GitClient::new(PathBuf::from("/tmp"));
        let refs = client.resolve_refs(&repo_path, &["develop".to_string()]).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "develop");
        assert!(!refs[0].commit_sha.is_empty());
    }

    #[test]
    fn resolve_refs_tag_and_branch_have_different_shas() {
        let (_dir, repo_path) = make_repo();
        let client = GitClient::new(PathBuf::from("/tmp"));
        let refs = client
            .resolve_refs(&repo_path, &["v1.0".to_string(), "develop".to_string()])
            .unwrap();
        assert_eq!(refs.len(), 2);
        assert_ne!(refs[0].commit_sha, refs[1].commit_sha);
    }

    #[test]
    fn resolve_refs_preserves_order() {
        let (_dir, repo_path) = make_repo();
        let client = GitClient::new(PathBuf::from("/tmp"));
        let refs = client
            .resolve_refs(&repo_path, &["develop".to_string(), "v1.0".to_string()])
            .unwrap();
        assert_eq!(refs[0].name, "develop");
        assert_eq!(refs[1].name, "v1.0");
    }

    #[test]
    fn resolve_refs_unknown_ref_returns_error() {
        let (_dir, repo_path) = make_repo();
        let client = GitClient::new(PathBuf::from("/tmp"));
        let result = client.resolve_refs(&repo_path, &["nonexistent".to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }

    #[test]
    fn resolve_refs_empty_list_returns_empty() {
        let (_dir, repo_path) = make_repo();
        let client = GitClient::new(PathBuf::from("/tmp"));
        let refs = client.resolve_refs(&repo_path, &[]).unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn resolve_refs_resolves_slashed_remote_tracking_branch() {
        // Simulate a cloned repo where "stable/2023.1.1" only exists as
        // refs/remotes/origin/stable/2023.1.1 (no local branch).
        let (_dir, repo_path) = make_repo();
        let repo = Repository::open(&repo_path).unwrap();
        let head_oid = repo.head().unwrap().peel_to_commit().unwrap().id();
        repo.reference(
            "refs/remotes/origin/stable/2023.1.1",
            head_oid,
            false,
            "remote tracking ref",
        ).unwrap();
        drop(repo);

        let client = GitClient::new(PathBuf::from("/tmp"));
        let refs = client
            .resolve_refs(&repo_path, &["stable/2023.1.1".to_string()])
            .unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "stable/2023.1.1");
        assert!(!refs[0].commit_sha.is_empty());
    }
}
