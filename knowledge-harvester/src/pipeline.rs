use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{Duration, interval};

use crate::config::{Config, Neo4jConfig, RepoConfig};
use crate::git::GitClient;
use crate::graph::writer::GraphWriter;
use crate::parser::ParserRegistry;

pub struct Pipeline {
    config: Config,
    git: GitClient,
    parsers: Arc<ParserRegistry>,
    writer: GraphWriter,
    progress_tx: Option<UnboundedSender<String>>,
}

impl Pipeline {
    pub fn set_progress_tx(&mut self, tx: UnboundedSender<String>) {
        self.progress_tx = Some(tx);
    }

    fn emit(&self, msg: impl Into<String>) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(msg.into());
        }
    }

    pub async fn new(config: Config) -> Result<Self> {
        let clone_root = std::env::temp_dir().join("harvest-repos");
        std::fs::create_dir_all(&clone_root)?;
        let mut git = GitClient::new(clone_root);
        if let Some(git_cfg) = &config.git {
            git = git.with_ssh_key(git_cfg.ssh_key_path.clone(), git_cfg.ssh_passphrase.clone());
        }
        let parsers = Arc::new(ParserRegistry::with_defaults());
        let writer = GraphWriter::new(
            &config.neo4j.uri,
            &config.neo4j.user,
            &config.neo4j.password,
        )
        .await?;
        writer.ensure_indexes().await?;
        Ok(Self { config, git, parsers, writer, progress_tx: None })
    }

    pub async fn new_with_neo4j(uri: &str, user: &str, password: &str) -> Result<Self> {
        let clone_root = std::env::temp_dir().join("harvest-repos");
        std::fs::create_dir_all(&clone_root)?;
        let git = GitClient::new(clone_root);
        let parsers = Arc::new(ParserRegistry::with_defaults());
        let writer = GraphWriter::new(uri, user, password).await?;
        writer.ensure_indexes().await?;
        let config = Config {
            neo4j: Neo4jConfig {
                uri: uri.to_string(),
                user: user.to_string(),
                password: password.to_string(),
            },
            git: None,
            repositories: vec![],
            llm: None,
            documentation: None,
        };
        Ok(Self { config, git, parsers, writer, progress_tx: None })
    }

    pub async fn process_single(&self, repo: &RepoConfig, force: bool) -> Result<()> {
        self.process_repo(repo, force).await
    }

    pub async fn list_remote_refs(&self, url: &str) -> Result<Vec<crate::git::TagInfo>> {
        let git = self.git.clone();
        let url = url.to_string();
        tokio::task::spawn_blocking(move || git.list_remote_refs(&url))
            .await?
    }

    pub async fn ingested_versions(&self, repo: &str) -> Result<Vec<String>> {
        self.writer.ingested_versions(repo).await
    }

    pub async fn run(&self, force: bool) -> Result<()> {
        for repo in &self.config.repositories {
            if let Err(e) = self.process_repo(repo, force).await {
                tracing::error!(repo = repo.name, error = %e, "repository failed");
            }
        }
        Ok(())
    }

    pub async fn watch(&self, interval_secs: u64) -> Result<()> {
        let mut ticker = interval(Duration::from_secs(interval_secs));
        loop {
            ticker.tick().await;
            self.run(false).await?;
        }
    }

    pub async fn reingest(&self) -> Result<()> {
        self.writer.reset_ingested().await?;
        tracing::info!("all versions marked for re-ingestion");
        self.run(false).await
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

    async fn process_repo(&self, repo: &RepoConfig, force: bool) -> Result<()> {
        self.emit(format!("Registering repository '{}' in Neo4j…", repo.name));
        self.writer.upsert_repository(&repo.name, &repo.resolved_browse_url()).await?;
        self.emit("Cloning repository…");
        let repo_path = self.git.ensure_cloned(repo)?;
        self.emit("Repository cloned. Discovering refs…");
        let tags = match &repo.refs {
            Some(wanted) => {
                self.emit(format!("Resolving {} specified ref(s)…", wanted.len()));
                self.git.resolve_refs(&repo_path, wanted)?
            }
            None => {
                self.emit("Listing all tags…");
                self.git.list_tags(&repo_path)?
            }
        };

        self.emit(format!("Found {} ref(s) to process.", tags.len()));

        for tag in tags {
            if !force && self.writer.is_ingested(&repo.name, &tag.name).await? {
                self.emit(format!("Ref '{}' already ingested, skipping.", tag.name));
                tracing::debug!(repo = repo.name, tag = tag.name, "already ingested, skipping");
                continue;
            }
            self.process_version(&repo_path, &repo.name, &tag.name, tag.timestamp).await?;
        }

        self.emit("Cleaning up temporary clone…");
        if let Err(e) = std::fs::remove_dir_all(&repo_path) {
            tracing::warn!(repo = repo.name, error = %e, "failed to remove cloned repository");
        }
        self.emit("Ingestion complete.");
        Ok(())
    }

    async fn process_version(
        &self,
        repo_path: &Path,
        repo: &str,
        tag: &str,
        timestamp: i64,
    ) -> Result<()> {
        self.emit(format!("Processing ref '{}'…", tag));
        tracing::info!(repo, tag, "ingesting version");

        self.writer.upsert_version(repo, tag, timestamp, false).await?;

        self.emit(format!("Checking out '{}'…", tag));
        self.git.checkout(repo_path, tag)?;

        self.emit("Walking source files…");
        let files = self.git.walk_source_files(repo_path)?;
        self.emit(format!("Found {} source files to parse.", files.len()));

        let parsers = Arc::clone(&self.parsers);
        let repo_owned = repo.to_owned();
        let tag_owned = tag.to_owned();
        let repo_clone = repo_owned.clone();
        let tag_clone = tag_owned.clone();
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

        let parsed_count = parsed.len();
        self.emit(format!("Parsed {} file(s). Writing to Neo4j…", parsed_count));

        self.writer.write_version(&repo_owned, &tag_owned, &parsed).await?;
        self.emit(format!("Ref '{}' ingested successfully ({} files).", tag, parsed_count));
        Ok(())
    }
}
