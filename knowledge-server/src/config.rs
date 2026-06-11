use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub neo4j: Neo4jConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub documentation: DocumentationConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
}

#[derive(Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    #[serde(default)]
    pub google: Option<GoogleConfig>,
}

#[derive(Deserialize, Clone)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

#[derive(Deserialize, Default)]
pub struct DocumentationConfig {
    pub docs_dir: Option<std::path::PathBuf>,
}

#[derive(Deserialize, Default, Clone)]
pub struct AgentsConfig {
    pub binary_path: Option<std::path::PathBuf>,
    pub public_url:  Option<String>,
}

#[derive(Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String { "0.0.0.0".into() }
fn default_port() -> u16 { 8080 }

#[derive(Deserialize)]
pub struct Neo4jConfig {
    pub uri: String,
    pub user: String,
    pub password: String,
}

#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum LlmConfig {
    Anthropic {
        model: String,
        api_key: String,
        #[serde(default = "default_max_iterations")]
        max_iterations: usize,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        #[serde(default = "default_compaction_threshold_chars")]
        compaction_threshold_chars: usize,
        #[serde(default = "default_compaction_keep_last")]
        compaction_keep_last: usize,
    },
    Gemini {
        model: String,
        api_key: String,
        #[serde(default = "default_max_iterations")]
        max_iterations: usize,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        #[serde(default = "default_compaction_threshold_chars")]
        compaction_threshold_chars: usize,
        #[serde(default = "default_compaction_keep_last")]
        compaction_keep_last: usize,
    },
    #[serde(rename = "openai-compatible")]
    OpenAiCompat {
        base_url: String,
        api_key: String,
        model: String,
        #[serde(default = "default_max_iterations")]
        max_iterations: usize,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        #[serde(default = "default_compaction_threshold_chars")]
        compaction_threshold_chars: usize,
        #[serde(default = "default_compaction_keep_last")]
        compaction_keep_last: usize,
    },
}

fn default_max_iterations() -> usize { 20 }
fn default_timeout_secs() -> u64 { 120 }
fn default_max_retries() -> u32 { 3 }
fn default_compaction_threshold_chars() -> usize { 40_000 }
fn default_compaction_keep_last() -> usize { 6 }

impl LlmConfig {
    pub fn max_iterations(&self) -> usize {
        match self {
            Self::Anthropic    { max_iterations, .. } => *max_iterations,
            Self::Gemini       { max_iterations, .. } => *max_iterations,
            Self::OpenAiCompat { max_iterations, .. } => *max_iterations,
        }
    }

    pub fn timeout_secs(&self) -> u64 {
        match self {
            Self::Anthropic    { timeout_secs, .. } => *timeout_secs,
            Self::Gemini       { timeout_secs, .. } => *timeout_secs,
            Self::OpenAiCompat { timeout_secs, .. } => *timeout_secs,
        }
    }

    pub fn max_retries(&self) -> u32 {
        match self {
            Self::Anthropic    { max_retries, .. } => *max_retries,
            Self::Gemini       { max_retries, .. } => *max_retries,
            Self::OpenAiCompat { max_retries, .. } => *max_retries,
        }
    }

    pub fn compaction_threshold_chars(&self) -> usize {
        match self {
            Self::Anthropic    { compaction_threshold_chars, .. } => *compaction_threshold_chars,
            Self::Gemini       { compaction_threshold_chars, .. } => *compaction_threshold_chars,
            Self::OpenAiCompat { compaction_threshold_chars, .. } => *compaction_threshold_chars,
        }
    }

    pub fn compaction_keep_last(&self) -> usize {
        match self {
            Self::Anthropic    { compaction_keep_last, .. } => *compaction_keep_last,
            Self::Gemini       { compaction_keep_last, .. } => *compaction_keep_last,
            Self::OpenAiCompat { compaction_keep_last, .. } => *compaction_keep_last,
        }
    }
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        toml::from_str(&text).context("parsing config TOML")
    }
}
