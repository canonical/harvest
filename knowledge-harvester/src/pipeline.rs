use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{Duration, interval};

use crate::config::{Config, RepoConfig};
use crate::git::GitClient;
use crate::graph::writer::GraphWriter;
use crate::parser::ParserRegistry;

pub struct Pipeline {
    config: Config,
    git: GitClient,
    parsers: Arc<ParserRegistry>,
    writer: GraphWriter,
}

impl Pipeline {
    pub async fn new(config: Config) -> Result<Self> {
        let git = GitClient::new(config.storage.clone_root.clone());
        let parsers = Arc::new(ParserRegistry::with_defaults());
        let writer = GraphWriter::new(
            &config.neo4j.uri,
            &config.neo4j.user,
            &config.neo4j.password,
        )
        .await?;
        writer.ensure_indexes().await?;
        Ok(Self { config, git, parsers, writer })
    }

    pub async fn run(&self) -> Result<()> {
        for repo in &self.config.repositories {
            if let Err(e) = self.process_repo(repo).await {
                tracing::error!(repo = repo.name, error = %e, "repository failed");
            }
        }
        Ok(())
    }

    pub async fn watch(&self, interval_secs: u64) -> Result<()> {
        let mut ticker = interval(Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            self.run().await?;
        }
    }

    pub async fn status(&self) -> Result<()> {
        for repo in &self.config.repositories {
            let versions = self.writer.ingested_versions(&repo.name).await?;
            println!("{}: {} version(s) ingested", repo.name, versions.len());
            for v in versions {
                println!("  {v}");
            }
        }
        Ok(())
    }

    async fn process_repo(&self, repo: &RepoConfig) -> Result<()> {
        let repo_path = self.git.ensure_cloned(repo)?;
        let tags = match &repo.refs {
            Some(wanted) => self.git.resolve_refs(&repo_path, wanted)?,
            None => self.git.list_tags(&repo_path)?,
        };

        for tag in tags {
            if self.writer.is_ingested(&repo.name, &tag.name).await? {
                tracing::debug!(repo = repo.name, tag = tag.name, "already ingested, skipping");
                continue;
            }
            self.process_version(&repo_path, &repo.name, &tag.name, tag.timestamp).await?;
        }
        Ok(())
    }

    async fn process_version(
        &self,
        repo_path: &Path,
        repo: &str,
        tag: &str,
        timestamp: i64,
    ) -> Result<()> {
        tracing::info!(repo, tag, "ingesting version");

        self.writer.upsert_version(repo, tag, timestamp, false).await?;

        self.git.checkout(repo_path, tag)?;

        let files = self.git.walk_source_files(repo_path)?;
        let parsers = Arc::clone(&self.parsers);
        let repo_owned = repo.to_owned();
        let tag_owned = tag.to_owned();
        let repo_clone = repo_owned.clone();
        let tag_clone = tag_owned.clone();
        // Capture repo_path as PathBuf so paths stored in the graph are
        // relative to the repo root rather than absolute clone paths.
        let repo_root = repo_path.to_path_buf();

        let parsed = tokio::task::spawn_blocking(move || {
            let mut out = Vec::new();
            for file_path in &files {
                let ext = file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if let Some(parser) = parsers.get(ext) {
                    let relative = file_path.strip_prefix(&repo_root).unwrap_or(file_path);
                    match std::fs::read_to_string(file_path) {
                        Ok(source) => out.push(parser.parse(&source, relative, &repo_clone, &tag_clone)),
                        Err(e) => tracing::warn!(path = %file_path.display(), error = %e, "skipping unreadable file"),
                    }
                }
            }
            out
        })
        .await?;

        self.writer.write_version(&repo_owned, &tag_owned, &parsed).await?;
        Ok(())
    }
}
