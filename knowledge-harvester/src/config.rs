use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub struct Config {
    pub neo4j: Neo4jConfig,
    pub storage: StorageConfig,
    pub git: Option<GitConfig>,
    pub repositories: Vec<RepoConfig>,
    pub llm: Option<LlmConfig>,
    pub documentation: Option<DocumentationConfig>,
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
pub struct GitConfig {
    pub ssh_key_path: PathBuf,
    pub ssh_passphrase: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct RepoConfig {
    pub name: String,
    pub url: String,
    pub refs: Option<Vec<String>>,
}

#[derive(Deserialize, Clone)]
pub struct DocumentationConfig {
    pub docs_dir: PathBuf,
}

#[derive(Deserialize, Clone)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum LlmConfig {
    Anthropic {
        model: String,
        api_key: String,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
    Gemini {
        model: String,
        api_key: String,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
    #[serde(rename = "openai-compatible")]
    OpenAiCompat {
        base_url: String,
        api_key: String,
        model: String,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
}

fn default_timeout_secs() -> u64 { 300 }
fn default_max_retries() -> u32 { 3 }

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        let mut config: Config = toml::from_str(&text).context("parsing config TOML")?;
        if let Some(git) = &mut config.git {
            git.ssh_key_path = expand_tilde(git.ssh_key_path.clone());
        }
        Ok(config)
    }
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(stripped);
        }
    }
    path
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

    #[test]
    fn llm_and_documentation_are_optional() {
        let config: Config = toml::from_str(&base_toml("")).unwrap();
        assert!(config.llm.is_none());
        assert!(config.documentation.is_none());
    }

    #[test]
    fn anthropic_llm_config_parsed() {
        let toml = r#"
[neo4j]
uri = "bolt://localhost:7687"
user = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/repo.git"

[llm]
provider     = "anthropic"
model        = "claude-sonnet-4-6"
api_key      = "sk-ant-test"
timeout_secs = 300
max_retries  = 3

[documentation]
docs_dir = "/tmp/docs"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let llm = config.llm.as_ref().unwrap();
        assert!(matches!(llm, LlmConfig::Anthropic { model, .. } if model == "claude-sonnet-4-6"));
        let doc = config.documentation.as_ref().unwrap();
        assert_eq!(doc.docs_dir, std::path::PathBuf::from("/tmp/docs"));
    }

    #[test]
    fn openai_compat_llm_config_parsed() {
        let toml = r#"
[neo4j]
uri = "bolt://localhost:7687"
user = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/repo.git"

[llm]
provider = "openai-compatible"
base_url = "https://api.groq.com/openai/v1"
api_key  = "gsk_test"
model    = "llama-3.3-70b"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let llm = config.llm.as_ref().unwrap();
        assert!(matches!(llm, LlmConfig::OpenAiCompat { model, .. } if model == "llama-3.3-70b"));
    }

    #[test]
    fn gemini_llm_config_parsed() {
        let toml = r#"
[neo4j]
uri = "bolt://localhost:7687"
user = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/repo.git"

[llm]
provider = "gemini"
model    = "gemini-2.5-flash"
api_key  = "AIza-test"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let llm = config.llm.as_ref().unwrap();
        assert!(matches!(llm, LlmConfig::Gemini { model, .. } if model == "gemini-2.5-flash"));
    }

    #[test]
    fn git_section_is_optional() {
        let config: Config = toml::from_str(&base_toml("")).unwrap();
        assert!(config.git.is_none());
    }

    #[test]
    fn git_ssh_key_path_parsed() {
        let toml = r#"
[neo4j]
uri = "bolt://localhost:7687"
user = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[git]
ssh_key_path = "/home/user/.ssh/id_ed25519"

[[repositories]]
name = "my-repo"
url  = "git@github.com:owner/repo.git"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let git = config.git.as_ref().unwrap();
        assert_eq!(git.ssh_key_path, PathBuf::from("/home/user/.ssh/id_ed25519"));
        assert!(git.ssh_passphrase.is_none());
    }

    #[test]
    fn git_ssh_passphrase_parsed() {
        let toml = r#"
[neo4j]
uri = "bolt://localhost:7687"
user = "neo4j"
password = "pass"

[storage]
clone_root = "/tmp/repos"

[git]
ssh_key_path    = "/home/user/.ssh/id_rsa"
ssh_passphrase  = "hunter2"

[[repositories]]
name = "my-repo"
url  = "git@github.com:owner/repo.git"
"#;
        let config: Config = toml::from_str(toml).unwrap();
        let git = config.git.as_ref().unwrap();
        assert_eq!(git.ssh_passphrase.as_deref(), Some("hunter2"));
    }

    #[test]
    fn expand_tilde_replaces_home() {
        std::env::set_var("HOME", "/home/testuser");
        let expanded = expand_tilde(PathBuf::from("~/.ssh/id_rsa"));
        assert_eq!(expanded, PathBuf::from("/home/testuser/.ssh/id_rsa"));
    }

    #[test]
    fn expand_tilde_leaves_absolute_path_unchanged() {
        let path = PathBuf::from("/absolute/path/key");
        assert_eq!(expand_tilde(path.clone()), path);
    }
}
