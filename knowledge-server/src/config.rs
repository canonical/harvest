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
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
}

#[derive(Deserialize, Clone)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

#[derive(Deserialize, Clone)]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub display_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_auth(toml: &str) -> AuthConfig {
        #[derive(serde::Deserialize)]
        struct Wrapper { auth: AuthConfig }
        toml::from_str::<Wrapper>(toml).expect("parse failed").auth
    }

    #[test]
    fn oidc_config_parses_all_fields() {
        let cfg = parse_auth(r#"
            [auth]
            jwt_secret = "s3cr3t"
            [auth.oidc]
            issuer_url   = "https://login.ubuntu.com"
            client_id    = "harvest"
            client_secret = "abc123"
            redirect_uri = "https://harvest.example.com/auth/oidc/callback"
            display_name = "Ubuntu One"
        "#);
        let oidc = cfg.oidc.expect("oidc should be present");
        assert_eq!(oidc.issuer_url, "https://login.ubuntu.com");
        assert_eq!(oidc.client_id, "harvest");
        assert_eq!(oidc.client_secret, "abc123");
        assert_eq!(oidc.redirect_uri, "https://harvest.example.com/auth/oidc/callback");
        assert_eq!(oidc.display_name.as_deref(), Some("Ubuntu One"));
    }

    #[test]
    fn oidc_display_name_is_optional() {
        let cfg = parse_auth(r#"
            [auth]
            jwt_secret = "s3cr3t"
            [auth.oidc]
            issuer_url   = "https://login.ubuntu.com"
            client_id    = "harvest"
            client_secret = "abc123"
            redirect_uri = "https://harvest.example.com/auth/oidc/callback"
        "#);
        let oidc = cfg.oidc.expect("oidc should be present");
        assert!(oidc.display_name.is_none());
    }

    #[test]
    fn oidc_absent_gives_none() {
        let cfg = parse_auth(r#"
            [auth]
            jwt_secret = "s3cr3t"
        "#);
        assert!(cfg.oidc.is_none());
    }

    #[test]
    fn google_and_oidc_can_coexist() {
        let cfg = parse_auth(r#"
            [auth]
            jwt_secret = "s3cr3t"
            [auth.google]
            client_id     = "gid"
            client_secret = "gsec"
            redirect_uri  = "https://example.com/auth/google/callback"
            [auth.oidc]
            issuer_url    = "https://idp.example.com"
            client_id     = "harvest"
            client_secret = "oidcsec"
            redirect_uri  = "https://example.com/auth/oidc/callback"
        "#);
        assert!(cfg.google.is_some());
        assert!(cfg.oidc.is_some());
    }
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
