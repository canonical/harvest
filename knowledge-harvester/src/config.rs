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
    /// If set, only these git refs (tags, branches, SHAs) are ingested.
    /// When absent, all git tags are ingested (original behaviour).
    pub refs: Option<Vec<String>>,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        toml::from_str(&text).context("parsing config TOML")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_toml(extra: &str) -> String {
        format!(
            r#"
[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/repo.git"
{extra}
"#
        )
    }

    #[test]
    fn repo_without_refs_defaults_to_none() {
        let config: Config = toml::from_str(&base_toml("")).unwrap();
        assert!(config.repositories[0].refs.is_none());
    }

    #[test]
    fn repo_with_refs_list_parsed() {
        let config: Config = toml::from_str(&base_toml(r#"refs = ["v1.0", "v2.0", "main"]"#)).unwrap();
        let refs = config.repositories[0].refs.as_ref().unwrap();
        assert_eq!(refs, &["v1.0", "v2.0", "main"]);
    }

    #[test]
    fn repo_with_single_ref() {
        let config: Config = toml::from_str(&base_toml(r#"refs = ["main"]"#)).unwrap();
        let refs = config.repositories[0].refs.as_ref().unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0], "main");
    }

    #[test]
    fn repo_with_empty_refs_list() {
        let config: Config = toml::from_str(&base_toml("refs = []")).unwrap();
        let refs = config.repositories[0].refs.as_ref().unwrap();
        assert!(refs.is_empty());
    }

    #[test]
    fn multiple_repos_each_own_refs() {
        let toml = r#"
[neo4j]
uri = "bolt://localhost:7687"
user = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[[repositories]]
name = "repo-a"
url  = "https://github.com/a.git"
refs = ["v1.0"]

[[repositories]]
name = "repo-b"
url  = "https://github.com/b.git"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        assert!(config.repositories[0].refs.is_some());
        assert!(config.repositories[1].refs.is_none());
    }
}
