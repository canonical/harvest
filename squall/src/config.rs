use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

fn default_true() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    120
}

fn default_max_retries() -> u32 {
    3
}

fn default_max_iterations() -> usize {
    40
}

fn default_output_dir() -> String {
    "./squall-reports".to_string()
}

#[derive(Deserialize, Clone, Default)]
pub struct PricingConfig {
    #[serde(default)]
    pub input_per_1k: i64,
    #[serde(default)]
    pub cache_read_per_1k: i64,
    #[serde(default)]
    pub cache_creation_per_1k: i64,
    #[serde(default)]
    pub output_per_1k: i64,
    #[serde(default)]
    pub reasoning_per_1k: i64,
}

impl PricingConfig {
    pub fn to_pricing(&self) -> crate::llm::pricing::ModelPricing {
        crate::llm::pricing::ModelPricing {
            input_per_1k: self.input_per_1k,
            cache_read_per_1k: self.cache_read_per_1k,
            cache_creation_per_1k: self.cache_creation_per_1k,
            output_per_1k: self.output_per_1k,
            reasoning_per_1k: self.reasoning_per_1k,
        }
    }
}

#[derive(Deserialize, Clone)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum LlmProviderConfig {
    Anthropic {
        model: String,
        api_key: String,
        #[serde(default)]
        id: String,
        #[serde(default)]
        priority: u32,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        #[serde(default = "default_true")]
        expose_to_ui: bool,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        models: Option<Vec<String>>,
        #[serde(default)]
        user_provided_key: bool,
        #[serde(default)]
        pricing: Option<PricingConfig>,
    },
    Gemini {
        model: String,
        api_key: String,
        #[serde(default)]
        id: String,
        #[serde(default)]
        priority: u32,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        #[serde(default = "default_true")]
        expose_to_ui: bool,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        models: Option<Vec<String>>,
        #[serde(default)]
        user_provided_key: bool,
        #[serde(default)]
        pricing: Option<PricingConfig>,
    },
    #[serde(rename = "openai-compatible")]
    OpenAiCompat {
        base_url: String,
        api_key: String,
        model: String,
        #[serde(default)]
        id: String,
        #[serde(default)]
        priority: u32,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
        #[serde(default = "default_true")]
        expose_to_ui: bool,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        models: Option<Vec<String>>,
        #[serde(default)]
        user_provided_key: bool,
        #[serde(default)]
        pricing: Option<PricingConfig>,
    },
}

impl LlmProviderConfig {
    pub fn priority(&self) -> u32 {
        match self {
            Self::Anthropic { priority, .. } => *priority,
            Self::Gemini { priority, .. } => *priority,
            Self::OpenAiCompat { priority, .. } => *priority,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Anthropic { id, .. } => id,
            Self::Gemini { id, .. } => id,
            Self::OpenAiCompat { id, .. } => id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Anthropic { .. } => "anthropic",
            Self::Gemini { .. } => "gemini",
            Self::OpenAiCompat { .. } => "openai-compatible",
        }
    }

    pub fn expose_to_ui(&self) -> bool {
        match self {
            Self::Anthropic { expose_to_ui, .. } => *expose_to_ui,
            Self::Gemini { expose_to_ui, .. } => *expose_to_ui,
            Self::OpenAiCompat { expose_to_ui, .. } => *expose_to_ui,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Anthropic { name, .. } => name.as_deref(),
            Self::Gemini { name, .. } => name.as_deref(),
            Self::OpenAiCompat { name, .. } => name.as_deref(),
        }
    }

    pub fn models(&self) -> Option<&[String]> {
        match self {
            Self::Anthropic { models, .. } => models.as_deref(),
            Self::Gemini { models, .. } => models.as_deref(),
            Self::OpenAiCompat { models, .. } => models.as_deref(),
        }
    }

    pub fn user_provided_key(&self) -> bool {
        match self {
            Self::Anthropic { user_provided_key, .. } => *user_provided_key,
            Self::Gemini { user_provided_key, .. } => *user_provided_key,
            Self::OpenAiCompat { user_provided_key, .. } => *user_provided_key,
        }
    }

    pub fn api_key(&self) -> &str {
        match self {
            Self::Anthropic { api_key, .. } => api_key,
            Self::Gemini { api_key, .. } => api_key,
            Self::OpenAiCompat { api_key, .. } => api_key,
        }
    }

    pub fn timeout_secs(&self) -> u64 {
        match self {
            Self::Anthropic { timeout_secs, .. } => *timeout_secs,
            Self::Gemini { timeout_secs, .. } => *timeout_secs,
            Self::OpenAiCompat { timeout_secs, .. } => *timeout_secs,
        }
    }

    pub fn max_retries(&self) -> u32 {
        match self {
            Self::Anthropic { max_retries, .. } => *max_retries,
            Self::Gemini { max_retries, .. } => *max_retries,
            Self::OpenAiCompat { max_retries, .. } => *max_retries,
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::OpenAiCompat { base_url, .. } => Some(base_url),
            _ => None,
        }
    }

    pub fn pricing(&self) -> Option<&PricingConfig> {
        match self {
            Self::Anthropic { pricing, .. } => pricing.as_ref(),
            Self::Gemini { pricing, .. } => pricing.as_ref(),
            Self::OpenAiCompat { pricing, .. } => pricing.as_ref(),
        }
    }

    pub fn model(&self) -> &str {
        match self {
            Self::Anthropic { model, .. } => model,
            Self::Gemini { model, .. } => model,
            Self::OpenAiCompat { model, .. } => model,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HarvestConfig {
    pub base_url: String,
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub group_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ExplorationConfig {
    pub max_iterations: usize,
    pub allow_infra: bool,
    pub output_dir: String,
}

impl Default for ExplorationConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            allow_infra: true,
            output_dir: default_output_dir(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub harvest: HarvestConfig,
    #[serde(rename = "llm", default)]
    pub llm: Vec<LlmProviderConfig>,
    #[serde(default)]
    pub exploration: ExplorationConfig,
}

impl std::fmt::Debug for LlmProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmProviderConfig").field("kind", &self.kind()).field("id", &self.id()).finish()
    }
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config: {}", path.display()))?;
        toml::from_str(&text).context("parsing config TOML")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let cfg: Config = toml::from_str(r#"
            [harvest]
            base_url = "http://localhost:8080"
            email = "qa@example.com"
            password = "secret"
        "#).unwrap();
        assert_eq!(cfg.harvest.base_url, "http://localhost:8080");
        assert_eq!(cfg.harvest.email, "qa@example.com");
        assert!(cfg.harvest.group_id.is_none());
        assert!(cfg.llm.is_empty());
    }

    #[test]
    fn exploration_defaults_applied_when_section_missing() {
        let cfg: Config = toml::from_str(r#"
            [harvest]
            base_url = "http://localhost:8080"
            email = "qa@example.com"
            password = "secret"
        "#).unwrap();
        assert_eq!(cfg.exploration.max_iterations, 40);
        assert!(cfg.exploration.allow_infra);
        assert_eq!(cfg.exploration.output_dir, "./squall-reports");
    }

    #[test]
    fn exploration_section_overrides_defaults() {
        let cfg: Config = toml::from_str(r#"
            [harvest]
            base_url = "http://localhost:8080"
            email = "qa@example.com"
            password = "secret"

            [exploration]
            max_iterations = 5
            allow_infra = false
            output_dir = "/tmp/out"
        "#).unwrap();
        assert_eq!(cfg.exploration.max_iterations, 5);
        assert!(!cfg.exploration.allow_infra);
        assert_eq!(cfg.exploration.output_dir, "/tmp/out");
    }

    #[test]
    fn parses_llm_provider_list() {
        let cfg: Config = toml::from_str(r#"
            [harvest]
            base_url = "http://localhost:8080"
            email = "qa@example.com"
            password = "secret"

            [[llm]]
            provider = "anthropic"
            model = "claude-sonnet-4-6"
            api_key = "k1"
            priority = 1

            [[llm]]
            provider = "openai-compatible"
            base_url = "https://openrouter.ai/api/v1"
            model = "google/gemini-3.5-flash"
            api_key = "k2"
            priority = 2
        "#).unwrap();
        assert_eq!(cfg.llm.len(), 2);
        assert_eq!(cfg.llm[0].kind(), "anthropic");
        assert_eq!(cfg.llm[0].model(), "claude-sonnet-4-6");
        assert_eq!(cfg.llm[1].kind(), "openai-compatible");
        assert_eq!(cfg.llm[1].base_url(), Some("https://openrouter.ai/api/v1"));
    }

    #[test]
    fn llm_provider_defaults_are_applied() {
        let cfg: LlmProviderConfig = toml::from_str(r#"
            provider = "anthropic"
            model = "m"
            api_key = "k"
        "#).unwrap();
        assert_eq!(cfg.priority(), 0);
        assert_eq!(cfg.timeout_secs(), 120);
        assert_eq!(cfg.max_retries(), 3);
        assert!(cfg.expose_to_ui());
        assert!(!cfg.user_provided_key());
        assert!(cfg.pricing().is_none());
    }

    #[test]
    fn from_file_reads_and_parses_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("squall.toml");
        std::fs::write(&path, r#"
            [harvest]
            base_url = "http://localhost:9000"
            email = "a@b.com"
            password = "p"
        "#).unwrap();
        let cfg = Config::from_file(&path).unwrap();
        assert_eq!(cfg.harvest.base_url, "http://localhost:9000");
    }

    #[test]
    fn from_file_errors_on_missing_file() {
        let err = Config::from_file(Path::new("/nonexistent/squall.toml")).unwrap_err();
        assert!(err.to_string().contains("reading config"));
    }
}
