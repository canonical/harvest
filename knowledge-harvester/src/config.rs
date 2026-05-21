use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub struct Config {
    pub neo4j: Neo4jConfig,
    pub storage: StorageConfig,
    pub repositories: Vec<RepoConfig>,
}

#[derive(Deserialize)]
pub struct Neo4jConfig {
    pub uri: String,
    pub user: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct StorageConfig {
    pub clone_root: PathBuf,
}

#[derive(Deserialize, Clone)]
pub struct RepoConfig {
    pub name: String,
    pub url: String,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        toml::from_str(&text).context("parsing config TOML")
    }
}
