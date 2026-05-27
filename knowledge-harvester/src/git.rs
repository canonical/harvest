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

    pub fn checkout(&self, repo_path: &Path, tag: &str) -> Result<()> {
        let repo = Repository::open(repo_path)?;
        let ref_name = format!("refs/tags/{tag}");
        let obj = repo
            .revparse_single(&ref_name)
            .with_context(|| format!("resolving tag {tag}"))?;
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
