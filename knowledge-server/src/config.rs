use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize, Clone, Default)]
pub struct SystemOneThresholds {
    #[serde(default = "default_system_one_confidence")]
    pub confidence_threshold: f64,
}

fn default_system_one_confidence() -> f64 {
    0.5
}
fn default_system_one_model() -> String {
    "~typesafe/jev-latest".into()
}
fn default_system_one_timeout() -> u64 {
    10
}
fn default_fast_path_threshold() -> f64 {
    0.7
}
fn default_early_synthesis_threshold() -> f64 {
    0.85
}
fn default_early_synthesis_research_threshold() -> f64 {
    0.95
}
fn default_relevance_threshold() -> f64 {
    0.4
}
fn default_relevance_preserve_recent() -> usize {
    2
}
fn default_early_synthesis_coverage_threshold() -> f64 {
    0.8
}
fn default_early_synthesis_min_iterations_first_turn() -> usize {
    5
}
fn default_early_synthesis_uniform_threshold() -> f64 {
    0.7
}

fn default_early_synthesis_capability_gate_threshold() -> f64 {
    0.8
}
fn default_next_action_confidence() -> f64 {
    0.5
}

#[derive(Deserialize, Clone, Default)]
pub struct SystemOneModelRouting {
    #[serde(default = "default_system_one_confidence")]
    pub confidence_threshold: f64,
    #[serde(default)]
    pub small_tier: Option<String>,
    #[serde(default)]
    pub medium_tier: Option<String>,
    #[serde(default)]
    pub large_tier: Option<String>,
}

#[derive(Deserialize, Clone, Default)]
pub struct SystemOneConfig {
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_system_one_model")]
    pub model: String,
    #[serde(default = "default_system_one_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub user_provided_key: bool,
    #[serde(default)]
    pub intent: SystemOneThresholds,
    #[serde(default)]
    pub model_routing: SystemOneModelRouting,
    #[serde(default)]
    pub tool_filter: SystemOneThresholds,
    #[serde(default)]
    pub compaction: SystemOneThresholds,
    #[serde(default)]
    pub parallel_research: SystemOneThresholds,
    #[serde(default)]
    pub prompt_sections: SystemOneThresholds,
    #[serde(default = "default_fast_path_threshold")]
    pub fast_path: f64,
    #[serde(default = "default_early_synthesis_threshold")]
    pub early_synthesis: f64,
    #[serde(default = "default_early_synthesis_research_threshold")]
    pub early_synthesis_research: f64,
    #[serde(default = "default_relevance_threshold")]
    pub relevance: f64,
    #[serde(default = "default_relevance_preserve_recent")]
    pub relevance_preserve_recent: usize,
    #[serde(default = "default_early_synthesis_coverage_threshold")]
    pub early_synthesis_coverage: f64,
    #[serde(default = "default_early_synthesis_min_iterations_first_turn")]
    pub early_synthesis_min_iterations_first_turn: usize,
    #[serde(default = "default_early_synthesis_uniform_threshold")]
    pub early_synthesis_uniform: f64,
    #[serde(default = "default_early_synthesis_capability_gate_threshold")]
    pub early_synthesis_capability_gate: f64,
    #[serde(default = "default_next_action_confidence")]
    pub next_action_confidence: f64,
}

impl SystemOneConfig {
    pub fn is_enabled(&self) -> bool {
        !self.endpoint.is_empty()
    }

    pub fn bundled_llm_provider_id(&self, llm_configs: &[LlmProviderConfig]) -> Option<String> {
        let so_host = reqwest::Url::parse(&self.endpoint)
            .ok()?
            .host_str()?
            .to_string();
        for config in llm_configs {
            if let Some(base_url) = config.base_url() {
                if let Ok(url) = reqwest::Url::parse(base_url) {
                    if url.host_str() == Some(&so_host) {
                        return Some(config.id().to_string());
                    }
                }
            }
        }
        None
    }

    pub fn is_bundled_with_llm(&self, llm_configs: &[LlmProviderConfig]) -> bool {
        if self.user_provided_key {
            return false;
        }
        if !self.api_key.is_empty() {
            return false;
        }
        self.bundled_llm_provider_id(llm_configs).is_some()
    }

    pub fn key_provider_id(&self, llm_configs: &[LlmProviderConfig]) -> String {
        if let Some(llm_id) = self.bundled_llm_provider_id(llm_configs) {
            if self.api_key.is_empty() && !self.user_provided_key {
                return llm_id;
            }
        }
        "system-one".to_string()
    }

    pub fn needs_user_key(&self, llm_configs: &[LlmProviderConfig]) -> bool {
        if !self.api_key.is_empty() {
            return false;
        }
        if self.is_bundled_with_llm(llm_configs) {
            return false;
        }
        true
    }
}

#[derive(Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub llm: Vec<LlmProviderConfig>,
    #[serde(default)]
    pub system_one: Option<SystemOneConfig>,
    #[serde(default)]
    pub agent: AgentBehaviorConfig,
    #[serde(default)]
    pub documentation: DocumentationConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub lxd: Option<LxdConfig>,
    #[serde(default)]
    pub collocate: Option<CollocateConfig>,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub semantic: SemanticConfig,
    #[serde(default)]
    pub cluster: ClusterConfig,
}

fn default_heartbeat_interval_ms() -> u64 {
    5_000
}
fn default_node_timeout_ms() -> u64 {
    20_000
}
fn default_bus_batch_window_ms() -> u64 {
    50
}
fn default_drain_grace_secs() -> u64 {
    10
}
fn default_drain_timeout_secs() -> u64 {
    120
}
fn default_election_timeout_ms() -> u64 {
    1_000
}
fn default_join_timeout_secs() -> u64 {
    60
}
fn default_snapshot_logs() -> u64 {
    1_000
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClusterBackend {
    #[default]
    Postgres,
    Quorium,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BusTransport {
    Notify,
    Mesh,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClusterBootstrap {
    Init,
    Join,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ClusterConfig {
    #[serde(default)]
    pub node_name: Option<String>,
    #[serde(default)]
    pub internal_listen: Option<String>,
    #[serde(default)]
    pub internal_url: Option<String>,
    #[serde(default)]
    pub shared_secret: Option<String>,
    #[serde(default = "default_heartbeat_interval_ms")]
    pub heartbeat_interval_ms: u64,
    #[serde(default = "default_node_timeout_ms")]
    pub node_timeout_ms: u64,
    #[serde(default = "default_bus_batch_window_ms")]
    pub bus_batch_window_ms: u64,
    #[serde(default = "default_drain_grace_secs")]
    pub drain_grace_secs: u64,
    #[serde(default = "default_drain_timeout_secs")]
    pub drain_timeout_secs: u64,
    #[serde(default)]
    pub backend: ClusterBackend,
    #[serde(default)]
    pub bus: Option<BusTransport>,
    #[serde(default)]
    pub data_dir: Option<std::path::PathBuf>,
    #[serde(default)]
    pub bootstrap: Option<ClusterBootstrap>,
    #[serde(default)]
    pub join_token: Option<String>,
    #[serde(default)]
    pub join_addresses: Vec<String>,
    #[serde(default)]
    pub accepted_secrets: Vec<String>,
    #[serde(default = "default_election_timeout_ms")]
    pub election_timeout_ms: u64,
    #[serde(default = "default_join_timeout_secs")]
    pub join_timeout_secs: u64,
    #[serde(default = "default_snapshot_logs")]
    pub snapshot_logs: u64,
    #[serde(default)]
    pub replace_absent_voter_after_secs: u64,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            node_name: None,
            internal_listen: None,
            internal_url: None,
            shared_secret: None,
            heartbeat_interval_ms: default_heartbeat_interval_ms(),
            node_timeout_ms: default_node_timeout_ms(),
            bus_batch_window_ms: default_bus_batch_window_ms(),
            drain_grace_secs: default_drain_grace_secs(),
            drain_timeout_secs: default_drain_timeout_secs(),
            backend: ClusterBackend::default(),
            bus: None,
            data_dir: None,
            bootstrap: None,
            join_token: None,
            join_addresses: Vec::new(),
            accepted_secrets: Vec::new(),
            election_timeout_ms: default_election_timeout_ms(),
            join_timeout_secs: default_join_timeout_secs(),
            snapshot_logs: default_snapshot_logs(),
            replace_absent_voter_after_secs: 0,
        }
    }
}

impl ClusterConfig {
    pub fn advertised_url(&self, bound: std::net::SocketAddr) -> Result<String> {
        if let Some(url) = self.internal_url.as_deref().filter(|u| !u.is_empty()) {
            return Ok(url.trim_end_matches('/').to_string());
        }
        if bound.ip().is_unspecified() {
            anyhow::bail!("cluster.internal_url is required when cluster.internal_listen binds every interface");
        }
        Ok(format!("http://{bound}"))
    }

    pub fn effective_shared_secret(&self) -> Option<String> {
        self.shared_secret
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                let token = self.join_token.as_deref().filter(|t| !t.is_empty())?;
                quorium::JoinToken::decode(token)
                    .ok()
                    .map(|t| t.shared_secret)
                    .filter(|s| !s.is_empty())
            })
    }

    pub fn validate(&self) -> Result<()> {
        if self.internal_listen.is_some() && self.effective_shared_secret().is_none() {
            anyhow::bail!("cluster.shared_secret is required when cluster.internal_listen is set");
        }
        if self.node_timeout_ms < self.heartbeat_interval_ms.saturating_mul(2) {
            anyhow::bail!(
                "cluster.node_timeout_ms must be at least twice cluster.heartbeat_interval_ms"
            );
        }
        if self.backend == ClusterBackend::Quorium {
            self.validate_quorium()?;
        }
        if self.bus_transport() == BusTransport::Mesh && self.internal_listen.is_none() {
            anyhow::bail!("cluster.bus = \"mesh\" requires cluster.internal_listen");
        }
        Ok(())
    }

    fn validate_quorium(&self) -> Result<()> {
        if self.internal_listen.is_none() {
            anyhow::bail!("cluster.backend = \"quorium\" requires cluster.internal_listen");
        }
        if self.data_dir.is_none() {
            anyhow::bail!("cluster.backend = \"quorium\" requires cluster.data_dir");
        }
        if self.bootstrap == Some(ClusterBootstrap::Join)
            && self.join_token.as_deref().map_or(true, str::is_empty)
            && self.join_addresses.is_empty()
        {
            anyhow::bail!(
                "cluster.bootstrap = \"join\" requires cluster.join_token or cluster.join_addresses"
            );
        }
        if self.snapshot_logs == 0 {
            anyhow::bail!("cluster.snapshot_logs must be positive");
        }
        let replace_after = self.replace_absent_voter_after_secs.saturating_mul(1_000);
        if replace_after > 0 && replace_after <= self.node_timeout_ms {
            anyhow::bail!(
                "cluster.replace_absent_voter_after_secs must be longer than cluster.node_timeout_ms"
            );
        }
        let election_worst_case = self
            .election_timeout_ms
            .saturating_mul(2)
            .saturating_add(self.heartbeat_interval_ms);
        if self.node_timeout_ms <= election_worst_case {
            anyhow::bail!(
                "cluster.node_timeout_ms must exceed twice cluster.election_timeout_ms plus cluster.heartbeat_interval_ms"
            );
        }
        Ok(())
    }

    pub fn bus_transport(&self) -> BusTransport {
        self.bus.unwrap_or(match self.backend {
            ClusterBackend::Postgres => BusTransport::Notify,
            ClusterBackend::Quorium => BusTransport::Mesh,
        })
    }
}

fn default_semantic_model() -> String {
    "text-embedding-004".to_string()
}
fn default_semantic_dimensions() -> usize {
    768
}
fn default_semantic_lexical_weight() -> f64 {
    0.6
}
fn default_semantic_semantic_weight() -> f64 {
    0.4
}
fn default_semantic_batch_size() -> usize {
    64
}

#[derive(Deserialize, Clone)]
pub struct SemanticConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_semantic_model")]
    pub model: String,
    #[serde(default = "default_semantic_dimensions")]
    pub dimensions: usize,
    #[serde(default = "default_semantic_lexical_weight")]
    pub lexical_weight: f64,
    #[serde(default = "default_semantic_semantic_weight")]
    pub semantic_weight: f64,
    #[serde(default)]
    pub backfill_on_start: bool,
    #[serde(default = "default_semantic_batch_size")]
    pub batch_size: usize,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: default_semantic_model(),
            dimensions: default_semantic_dimensions(),
            lexical_weight: default_semantic_lexical_weight(),
            semantic_weight: default_semantic_semantic_weight(),
            backfill_on_start: false,
            batch_size: default_semantic_batch_size(),
        }
    }
}

impl SemanticConfig {
    pub fn effective_weights(&self) -> (f64, f64) {
        crate::agent::semantic::hybrid_weights(self.lexical_weight, self.semantic_weight).unwrap_or(
            (
                crate::agent::semantic::DEFAULT_LEXICAL_WEIGHT,
                crate::agent::semantic::DEFAULT_SEMANTIC_WEIGHT,
            ),
        )
    }

    pub fn effective_batch_size(&self) -> usize {
        crate::agent::semantic::backfill_batch_limit(Some(self.batch_size))
    }
}

#[derive(Deserialize, Default, Clone)]
pub struct UiConfig {
    #[serde(default)]
    pub enable_docs: bool,
}

#[derive(Deserialize, Default, Clone)]
pub struct SecurityConfig {
    #[serde(default)]
    pub user_key_encryption_key: Option<String>,
}

#[derive(Deserialize)]
pub struct AuthConfig {
    pub jwt_secret: String,
    #[serde(default = "default_true")]
    pub allow_local_login: bool,
    #[serde(default)]
    pub google: Option<GoogleConfig>,
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    #[serde(default)]
    pub public_url: Option<String>,
}

impl AuthConfig {
    pub fn secure_cookies(&self) -> bool {
        self.public_url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://"))
    }
}

fn default_true() -> bool {
    true
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

#[derive(Deserialize, Default)]
pub struct DocumentationConfig {
    pub docs_dir: Option<std::path::PathBuf>,
}

#[derive(Deserialize, Default, Clone)]
pub struct AgentsConfig {
    pub binary_path: Option<std::path::PathBuf>,
    pub public_url: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct LxdConfig {
    pub endpoint: String,
    #[serde(default)]
    pub client_cert: Option<String>,
    #[serde(default)]
    pub client_key: Option<String>,
    #[serde(default)]
    pub trust_token: Option<String>,
    #[serde(default)]
    pub ca_cert: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default = "default_lxd_project")]
    pub project: String,
    #[serde(default = "default_lxd_image_alias")]
    pub image_alias: String,
    #[serde(default = "default_lxd_image_server")]
    pub image_server: String,
    #[serde(default = "default_lxd_profile")]
    pub profile: String,
}

fn default_lxd_project() -> String {
    "default".into()
}
fn default_lxd_image_alias() -> String {
    "24.04".into()
}
fn default_lxd_image_server() -> String {
    "https://cloud-images.ubuntu.com/releases".into()
}
fn default_lxd_profile() -> String {
    "default".into()
}

#[derive(Deserialize, Clone)]
pub struct CollocateConfig {
    pub endpoint: String,
    #[serde(default)]
    pub trust_token: Option<String>,
    #[serde(default)]
    pub client_cert: Option<String>,
    #[serde(default)]
    pub client_key: Option<String>,
    #[serde(default = "default_collocate_project")]
    pub project: String,
    #[serde(default = "default_collocate_image")]
    pub default_image: String,
    #[serde(default = "default_collocate_user")]
    pub default_user: String,
    #[serde(default = "default_collocate_idle_timeout")]
    pub default_idle_timeout_secs: u64,
    #[serde(default = "default_collocate_max_containers")]
    pub max_containers: usize,
    #[serde(default)]
    pub ca_cert: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub server_fingerprint: Option<String>,
    #[serde(default)]
    pub image_presets: HashMap<String, String>,
}

fn default_collocate_project() -> String {
    "harvest".into()
}
fn default_collocate_image() -> String {
    "ubuntu:24.04".into()
}
fn default_collocate_user() -> String {
    "root".into()
}
fn default_collocate_idle_timeout() -> u64 {
    600
}
fn default_collocate_max_containers() -> usize {
    20
}

#[derive(Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8080
}

fn default_pool_size() -> usize {
    harvest_db::DEFAULT_POOL_SIZE
}

#[derive(Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default = "default_pool_size")]
    pub pool_size: usize,
    #[serde(default)]
    pub ca_file: Option<std::path::PathBuf>,
    #[serde(default = "default_true")]
    pub migrate_on_start: bool,
}

impl DatabaseConfig {
    pub fn options(&self) -> harvest_db::DbOptions {
        harvest_db::DbOptions::new(self.url.clone())
            .with_pool_size(self.pool_size)
            .with_ca_file(self.ca_file.clone())
    }
}

#[derive(Deserialize)]
pub struct AgentBehaviorConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_compaction_threshold_chars")]
    pub compaction_threshold_chars: usize,
    #[serde(default = "default_compaction_keep_last")]
    pub compaction_keep_last: usize,
    #[serde(default = "default_max_active_turns")]
    pub max_active_turns: usize,
    #[serde(default = "default_max_active_turns_per_user")]
    pub max_active_turns_per_user: usize,
}

impl Default for AgentBehaviorConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            compaction_threshold_chars: default_compaction_threshold_chars(),
            compaction_keep_last: default_compaction_keep_last(),
            max_active_turns: default_max_active_turns(),
            max_active_turns_per_user: default_max_active_turns_per_user(),
        }
    }
}

fn default_max_active_turns() -> usize {
    200
}

fn default_max_active_turns_per_user() -> usize {
    10
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
        #[serde(default)]
        max_concurrency: usize,
        #[serde(default)]
        context_window: Option<usize>,
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
        #[serde(default)]
        max_concurrency: usize,
        #[serde(default)]
        context_window: Option<usize>,
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
        #[serde(default)]
        enable_prompt_caching: bool,
        #[serde(default)]
        max_concurrency: usize,
        #[serde(default)]
        context_window: Option<usize>,
    },
    Mock {
        #[serde(default = "default_mock_model")]
        model: String,
        #[serde(default)]
        id: String,
        #[serde(default)]
        priority: u32,
        #[serde(default = "default_true")]
        expose_to_ui: bool,
        #[serde(default)]
        name: Option<String>,
        #[serde(flatten)]
        behaviour: MockBehaviour,
        #[serde(default)]
        max_concurrency: usize,
        #[serde(default)]
        context_window: Option<usize>,
    },
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MockBehaviour {
    #[serde(default = "default_mock_chunks")]
    pub chunks: usize,
    #[serde(default = "default_mock_chunk_delay_ms")]
    pub chunk_delay_ms: u64,
    #[serde(default = "default_mock_first_token_ms")]
    pub first_token_ms: u64,
    #[serde(default)]
    pub reply: Option<String>,
}

impl Default for MockBehaviour {
    fn default() -> Self {
        Self {
            chunks: default_mock_chunks(),
            chunk_delay_ms: default_mock_chunk_delay_ms(),
            first_token_ms: default_mock_first_token_ms(),
            reply: None,
        }
    }
}

fn default_mock_model() -> String {
    "mock".to_string()
}

fn default_mock_chunks() -> usize {
    8
}

fn default_mock_chunk_delay_ms() -> u64 {
    20
}

fn default_mock_first_token_ms() -> u64 {
    100
}

impl LlmProviderConfig {
    pub fn priority(&self) -> u32 {
        match self {
            Self::Anthropic { priority, .. } => *priority,
            Self::Gemini { priority, .. } => *priority,
            Self::OpenAiCompat { priority, .. } => *priority,
            Self::Mock { priority, .. } => *priority,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Anthropic { id, .. } => id,
            Self::Gemini { id, .. } => id,
            Self::OpenAiCompat { id, .. } => id,
            Self::Mock { id, .. } => id,
        }
    }

    fn id_mut(&mut self) -> &mut String {
        match self {
            Self::Anthropic { id, .. } => id,
            Self::Gemini { id, .. } => id,
            Self::OpenAiCompat { id, .. } => id,
            Self::Mock { id, .. } => id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Anthropic { .. } => "anthropic",
            Self::Gemini { .. } => "gemini",
            Self::OpenAiCompat { .. } => "openai-compatible",
            Self::Mock { .. } => "mock",
        }
    }

    pub fn expose_to_ui(&self) -> bool {
        match self {
            Self::Anthropic { expose_to_ui, .. } => *expose_to_ui,
            Self::Gemini { expose_to_ui, .. } => *expose_to_ui,
            Self::OpenAiCompat { expose_to_ui, .. } => *expose_to_ui,
            Self::Mock { expose_to_ui, .. } => *expose_to_ui,
        }
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Anthropic { name, .. } => name.as_deref(),
            Self::Gemini { name, .. } => name.as_deref(),
            Self::OpenAiCompat { name, .. } => name.as_deref(),
            Self::Mock { name, .. } => name.as_deref(),
        }
    }

    pub fn models(&self) -> Option<&[String]> {
        match self {
            Self::Anthropic { models, .. } => models.as_deref(),
            Self::Gemini { models, .. } => models.as_deref(),
            Self::OpenAiCompat { models, .. } => models.as_deref(),
            Self::Mock { .. } => None,
        }
    }

    pub fn user_provided_key(&self) -> bool {
        match self {
            Self::Anthropic {
                user_provided_key, ..
            } => *user_provided_key,
            Self::Gemini {
                user_provided_key, ..
            } => *user_provided_key,
            Self::OpenAiCompat {
                user_provided_key, ..
            } => *user_provided_key,
            Self::Mock { .. } => false,
        }
    }

    pub fn api_key(&self) -> &str {
        match self {
            Self::Anthropic { api_key, .. } => api_key,
            Self::Gemini { api_key, .. } => api_key,
            Self::OpenAiCompat { api_key, .. } => api_key,
            Self::Mock { .. } => "",
        }
    }

    pub fn timeout_secs(&self) -> u64 {
        match self {
            Self::Anthropic { timeout_secs, .. } => *timeout_secs,
            Self::Gemini { timeout_secs, .. } => *timeout_secs,
            Self::OpenAiCompat { timeout_secs, .. } => *timeout_secs,
            Self::Mock { .. } => default_timeout_secs(),
        }
    }

    pub fn max_retries(&self) -> u32 {
        match self {
            Self::Anthropic { max_retries, .. } => *max_retries,
            Self::Gemini { max_retries, .. } => *max_retries,
            Self::OpenAiCompat { max_retries, .. } => *max_retries,
            Self::Mock { .. } => 0,
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        match self {
            Self::OpenAiCompat { base_url, .. } => Some(base_url),
            _ => None,
        }
    }

    pub fn enable_prompt_caching(&self) -> bool {
        match self {
            Self::OpenAiCompat {
                enable_prompt_caching,
                ..
            } => *enable_prompt_caching,
            _ => false,
        }
    }

    pub fn pricing(&self) -> Option<&PricingConfig> {
        match self {
            Self::Anthropic { pricing, .. } => pricing.as_ref(),
            Self::Gemini { pricing, .. } => pricing.as_ref(),
            Self::OpenAiCompat { pricing, .. } => pricing.as_ref(),
            Self::Mock { .. } => None,
        }
    }

    pub fn max_concurrency(&self) -> usize {
        match self {
            Self::Anthropic {
                max_concurrency, ..
            }
            | Self::Gemini {
                max_concurrency, ..
            }
            | Self::OpenAiCompat {
                max_concurrency, ..
            }
            | Self::Mock {
                max_concurrency, ..
            } => *max_concurrency,
        }
    }

    pub fn context_window(&self) -> Option<usize> {
        match self {
            Self::Anthropic { context_window, .. }
            | Self::Gemini { context_window, .. }
            | Self::OpenAiCompat { context_window, .. }
            | Self::Mock { context_window, .. } => *context_window,
        }
    }

    pub fn model(&self) -> &str {
        match self {
            Self::Anthropic { model, .. } => model,
            Self::Gemini { model, .. } => model,
            Self::OpenAiCompat { model, .. } => model,
            Self::Mock { model, .. } => model,
        }
    }
}

fn default_max_iterations() -> usize {
    20
}
fn default_timeout_secs() -> u64 {
    120
}
fn default_max_retries() -> u32 {
    3
}
fn default_compaction_threshold_chars() -> usize {
    40_000
}
fn default_compaction_keep_last() -> usize {
    6
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        let mut config: Config = toml::from_str(&text).context("parsing config TOML")?;
        config.normalize_llm_ids()?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        self.cluster.validate()
    }

    fn normalize_llm_ids(&mut self) -> Result<()> {
        for provider in &mut self.llm {
            if provider.id_mut().is_empty() {
                let derived = format!("{}-{}", provider.kind(), provider.priority());
                *provider.id_mut() = derived;
            }
        }
        let mut seen = std::collections::HashSet::new();
        for provider in &self.llm {
            if !seen.insert(provider.id()) {
                anyhow::bail!("duplicate LLM provider id: {}", provider.id());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_auth(toml: &str) -> AuthConfig {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            auth: AuthConfig,
        }
        toml::from_str::<Wrapper>(toml).expect("parse failed").auth
    }

    fn parse_config(toml: &str) -> Config {
        toml::from_str::<Config>(toml).expect("parse failed")
    }

    fn parse_and_normalize(toml: &str) -> Result<Config> {
        let mut cfg = toml::from_str::<Config>(toml).expect("parse failed");
        cfg.normalize_llm_ids()?;
        Ok(cfg)
    }

    #[test]
    fn oidc_config_parses_all_fields() {
        let cfg = parse_auth(
            r#"
            [auth]
            jwt_secret = "s3cr3t"
            [auth.oidc]
            issuer_url   = "https://login.ubuntu.com"
            client_id    = "harvest"
            client_secret = "abc123"
            redirect_uri = "https://harvest.example.com/auth/oidc/callback"
            display_name = "Ubuntu One"
        "#,
        );
        let oidc = cfg.oidc.expect("oidc should be present");
        assert_eq!(oidc.issuer_url, "https://login.ubuntu.com");
        assert_eq!(oidc.client_id, "harvest");
        assert_eq!(oidc.client_secret, "abc123");
        assert_eq!(
            oidc.redirect_uri,
            "https://harvest.example.com/auth/oidc/callback"
        );
        assert_eq!(oidc.display_name.as_deref(), Some("Ubuntu One"));
    }

    #[test]
    fn oidc_display_name_is_optional() {
        let cfg = parse_auth(
            r#"
            [auth]
            jwt_secret = "s3cr3t"
            [auth.oidc]
            issuer_url   = "https://login.ubuntu.com"
            client_id    = "harvest"
            client_secret = "abc123"
            redirect_uri = "https://harvest.example.com/auth/oidc/callback"
        "#,
        );
        let oidc = cfg.oidc.expect("oidc should be present");
        assert!(oidc.display_name.is_none());
    }

    #[test]
    fn oidc_absent_gives_none() {
        let cfg = parse_auth(
            r#"
            [auth]
            jwt_secret = "s3cr3t"
        "#,
        );
        assert!(cfg.oidc.is_none());
    }

    #[test]
    fn local_login_defaults_to_true() {
        let cfg = parse_auth(
            r#"
            [auth]
            jwt_secret = "s3cr3t"
        "#,
        );
        assert!(cfg.allow_local_login);
    }

    #[test]
    fn local_login_can_be_disabled() {
        let cfg = parse_auth(
            r#"
            [auth]
            jwt_secret = "s3cr3t"
            allow_local_login = false
        "#,
        );
        assert!(!cfg.allow_local_login);
    }

    #[test]
    fn google_and_oidc_can_coexist() {
        let cfg = parse_auth(
            r#"
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
        "#,
        );
        assert!(cfg.google.is_some());
        assert!(cfg.oidc.is_some());
    }

    const TEST_LLM: &str = r#"
        [[llm]]
        provider = "gemini"
        model    = "gemini-2.5-flash"
        api_key  = "key1"
    "#;

    fn minimal_config(llm_block: &str) -> String {
        format!(
            r#"
            [server]
            [database]
            url = "postgres://harvest:pw@localhost:5432/harvest"
            [auth]
            jwt_secret = "secret"
            {llm_block}
        "#
        )
    }

    #[test]
    fn single_gemini_provider_parses() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-2.5-flash"
            api_key  = "key1"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::Gemini { model, .. } => assert_eq!(model, "gemini-2.5-flash"),
            other => panic!(
                "expected Gemini, got something else: {other:?}",
                other = std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn single_anthropic_provider_parses() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "anthropic"
            model    = "claude-sonnet-4-6"
            api_key  = "key2"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::Anthropic { model, .. } => assert_eq!(model, "claude-sonnet-4-6"),
            other => panic!(
                "expected Anthropic: {other:?}",
                other = std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn two_providers_parse_as_vec_of_two() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-flash"
            api_key  = "k1"
            priority = 1

            [[llm]]
            provider = "anthropic"
            model    = "claude-sonnet-4-6"
            api_key  = "k2"
            priority = 2
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 2);
        assert_eq!(cfg.llm[0].priority(), 1);
        assert_eq!(cfg.llm[1].priority(), 2);
    }

    #[test]
    fn priority_defaults_to_zero() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-flash"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm[0].priority(), 0);
    }

    #[test]
    fn agent_section_uses_defaults_when_absent() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.agent.max_iterations, 20);
        assert_eq!(cfg.agent.compaction_threshold_chars, 40_000);
        assert_eq!(cfg.agent.compaction_keep_last, 6);
    }

    #[test]
    fn agent_section_parses_explicit_values() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [agent]
            max_iterations = 10
            compaction_threshold_chars = 10000
            compaction_keep_last = 3
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.agent.max_iterations, 10);
        assert_eq!(cfg.agent.compaction_threshold_chars, 10_000);
        assert_eq!(cfg.agent.compaction_keep_last, 3);
    }

    #[test]
    fn openai_compat_provider_parses() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "openai-compatible"
            base_url = "https://openai.example.com"
            api_key  = "k"
            model    = "gpt-4o"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::OpenAiCompat {
                model, base_url, ..
            } => {
                assert_eq!(model, "gpt-4o");
                assert_eq!(base_url, "https://openai.example.com");
            }
            other => panic!(
                "expected OpenAiCompat: {other:?}",
                other = std::mem::discriminant(other)
            ),
        }
    }

    #[test]
    fn openrouter_provider_with_user_supplied_key_parses() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider           = "openai-compatible"
            base_url           = "https://openrouter.ai/api/v1"
            api_key            = ""
            model              = "z-ai/glm-5.3-flash"
            models             = ["z-ai/glm-5.3-flash"]
            id                 = "openrouter"
            name               = "OpenRouter"
            priority           = 1
            user_provided_key  = true
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::OpenAiCompat {
                base_url,
                model,
                id,
                name,
                priority,
                user_provided_key,
                ..
            } => {
                assert_eq!(base_url, "https://openrouter.ai/api/v1");
                assert_eq!(model, "z-ai/glm-5.3-flash");
                assert_eq!(id, "openrouter");
                assert_eq!(name.as_deref(), Some("OpenRouter"));
                assert_eq!(*priority, 1);
                assert!(*user_provided_key);
            }
            other => panic!("expected OpenAiCompat: {:?}", std::mem::discriminant(other)),
        }
        assert!(
            cfg.llm[0].api_key().is_empty(),
            "user-supplied keys must not be baked into the config"
        );
    }

    #[test]
    fn opencode_zen_free_fallback_sits_after_openrouter() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider           = "openai-compatible"
            base_url           = "https://openrouter.ai/api/v1"
            api_key            = ""
            model              = "z-ai/glm-5.3-flash"
            models             = ["z-ai/glm-5.3-flash"]
            id                 = "openrouter"
            name               = "OpenRouter"
            priority           = 1
            user_provided_key  = true

            [[llm]]
            provider           = "openai-compatible"
            base_url           = "https://opencode.ai/zen/v1"
            api_key            = ""
            model              = "longcat-2.5-preview-free"
            models             = ["longcat-2.5-preview-free", "jev-1.13-free"]
            id                 = "opencode-zen"
            name               = "OpenCode Zen"
            priority           = 2
            user_provided_key  = true
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 2);
        let priorities: Vec<u32> = cfg.llm.iter().map(|c| c.priority()).collect();
        assert_eq!(priorities, vec![1, 2]);
        assert_eq!(cfg.llm[0].id(), "openrouter");
        assert_eq!(cfg.llm[1].id(), "opencode-zen");
        match &cfg.llm[1] {
            LlmProviderConfig::OpenAiCompat {
                base_url,
                model,
                models,
                user_provided_key,
                ..
            } => {
                assert_eq!(base_url, "https://opencode.ai/zen/v1");
                assert_eq!(model, "longcat-2.5-preview-free");
                assert_eq!(
                    *models,
                    Some(vec![
                        "longcat-2.5-preview-free".to_string(),
                        "jev-1.13-free".to_string(),
                    ])
                );
                assert!(*user_provided_key);
            }
            other => panic!("expected OpenAiCompat: {:?}", std::mem::discriminant(other)),
        }
        assert_eq!(
            cfg.llm[1].api_key().len(),
            0,
            "zen fallback must not bake in a key"
        );
    }

    #[test]
    fn timeout_and_retries_have_defaults() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "anthropic"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        match &cfg.llm[0] {
            LlmProviderConfig::Anthropic {
                timeout_secs,
                max_retries,
                ..
            } => {
                assert_eq!(*timeout_secs, 120);
                assert_eq!(*max_retries, 3);
            }
            _ => panic!("expected Anthropic"),
        }
    }

    #[test]
    fn expose_to_ui_defaults_to_true() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(cfg.llm[0].expose_to_ui());
    }

    #[test]
    fn expose_to_ui_can_be_set_false() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider     = "openai-compatible"
            base_url     = "https://openai.example.com"
            model        = "gpt-4o"
            api_key      = "k"
            expose_to_ui = false
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(!cfg.llm[0].expose_to_ui());
    }

    #[test]
    fn name_defaults_to_none() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm[0].name(), None);
    }

    #[test]
    fn name_is_preserved_when_set() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "openai-compatible"
            base_url = "https://lemonade.example.com/api/v1"
            model    = "Mistral-3-3B"
            api_key  = ""
            name     = "Lemonade (local)"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm[0].name(), Some("Lemonade (local)"));
    }

    #[test]
    fn models_defaults_to_none() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm[0].models(), None);
    }

    #[test]
    fn models_allowlist_is_preserved_when_set() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-3.1-pro-preview"
            api_key  = "k"
            models   = ["gemini-2.5-flash", "gemini-2.5-pro"]
        "#,
        );
        let cfg = parse_config(&toml);
        assert_eq!(
            cfg.llm[0].models(),
            Some(&["gemini-2.5-flash".to_string(), "gemini-2.5-pro".to_string()][..])
        );
    }

    #[test]
    fn user_provided_key_defaults_to_false() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(!cfg.llm[0].user_provided_key());
    }

    #[test]
    fn user_provided_key_can_be_set_true() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider           = "anthropic"
            model              = "m"
            api_key            = ""
            user_provided_key  = true
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(cfg.llm[0].user_provided_key());
    }

    #[test]
    fn blank_id_normalizes_to_kind_and_priority() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-flash"
            api_key  = "k"
            priority = 2
        "#,
        );
        let cfg = parse_and_normalize(&toml).expect("normalize failed");
        assert_eq!(cfg.llm[0].id(), "gemini-2");
        assert_eq!(cfg.llm[0].kind(), "gemini");
    }

    #[test]
    fn explicit_id_is_preserved() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "anthropic"
            model    = "claude-sonnet-4-6"
            api_key  = "k"
            id       = "main-anthropic"
        "#,
        );
        let cfg = parse_and_normalize(&toml).expect("normalize failed");
        assert_eq!(cfg.llm[0].id(), "main-anthropic");
    }

    #[test]
    fn duplicate_explicit_ids_are_rejected() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-flash"
            api_key  = "k1"
            id       = "dup"

            [[llm]]
            provider = "anthropic"
            model    = "claude-sonnet-4-6"
            api_key  = "k2"
            id       = "dup"
        "#,
        );
        let result = parse_and_normalize(&toml);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("dup"),
            "expected error to mention id, got: {err}"
        );
    }

    #[test]
    fn two_blank_ids_with_same_kind_and_priority_still_collide() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-flash"
            api_key  = "k1"

            [[llm]]
            provider = "gemini"
            model    = "gemini-pro"
            api_key  = "k2"
        "#,
        );
        assert!(parse_and_normalize(&toml).is_err());
    }

    #[test]
    fn lxd_absent_gives_none() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(cfg.lxd.is_none());
    }

    #[test]
    fn lxd_config_parses_required_fields() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint    = "https://lxd.example.com:8443"
            client_cert = "cert-pem"
            client_key  = "key-pem"
        "#,
        );
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(lxd.endpoint, "https://lxd.example.com:8443");
        assert_eq!(lxd.client_cert.as_deref(), Some("cert-pem"));
        assert_eq!(lxd.client_key.as_deref(), Some("key-pem"));
        assert!(lxd.ca_cert.is_none());
        assert!(!lxd.insecure);
    }

    #[test]
    fn lxd_config_client_cert_and_key_are_optional() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint = "https://lxd.example.com:8443"
        "#,
        );
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert!(lxd.client_cert.is_none());
        assert!(lxd.client_key.is_none());
        assert!(lxd.trust_token.is_none());
    }

    #[test]
    fn lxd_config_trust_token_parses() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint    = "https://lxd.example.com:8443"
            trust_token = "eyJjbGllbnRfbmFtZSI6Li4ufQ=="
        "#,
        );
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(
            lxd.trust_token.as_deref(),
            Some("eyJjbGllbnRfbmFtZSI6Li4ufQ==")
        );
    }

    #[test]
    fn lxd_config_defaults_apply_when_omitted() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint    = "https://lxd.example.com:8443"
            client_cert = "cert-pem"
            client_key  = "key-pem"
        "#,
        );
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(lxd.project, "default");
        assert_eq!(lxd.image_alias, "24.04");
        assert_eq!(lxd.image_server, "https://cloud-images.ubuntu.com/releases");
        assert_eq!(lxd.profile, "default");
    }

    #[test]
    fn lxd_config_explicit_values_override_defaults() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint     = "https://lxd.example.com:8443"
            client_cert  = "cert-pem"
            client_key   = "key-pem"
            ca_cert      = "ca-pem"
            insecure     = true
            project      = "harvest"
            image_alias  = "22.04"
            image_server = "https://images.example.com"
            profile      = "harvest-profile"
        "#,
        );
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(lxd.ca_cert.as_deref(), Some("ca-pem"));
        assert!(lxd.insecure);
        assert_eq!(lxd.project, "harvest");
        assert_eq!(lxd.image_alias, "22.04");
        assert_eq!(lxd.image_server, "https://images.example.com");
        assert_eq!(lxd.profile, "harvest-profile");
    }

    #[test]
    fn openai_compat_enable_prompt_caching_defaults_to_false() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "openai-compatible"
            base_url = "https://openai.example.com"
            api_key  = "k"
            model    = "gpt-4o"
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(!cfg.llm[0].enable_prompt_caching());
    }

    #[test]
    fn openai_compat_enable_prompt_caching_can_be_set_true() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider           = "openai-compatible"
            base_url           = "https://openai.example.com"
            api_key            = "k"
            model              = "gpt-4o"
            enable_prompt_caching = true
        "#,
        );
        let cfg = parse_config(&toml);
        assert!(cfg.llm[0].enable_prompt_caching());
    }

    #[test]
    fn system_one_thresholds_default_when_absent() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [system_one]
            endpoint = "https://so.example.com"
        "#,
        );
        let cfg = parse_config(&toml);
        let so = cfg.system_one.expect("system_one should be present");
        assert!((so.fast_path - 0.7).abs() < f64::EPSILON);
        assert!((so.early_synthesis - 0.85).abs() < f64::EPSILON);
        assert!((so.relevance - 0.4).abs() < f64::EPSILON);
        assert!((so.early_synthesis_research - 0.95).abs() < f64::EPSILON);
        assert_eq!(so.relevance_preserve_recent, 2);
        assert!((so.early_synthesis_coverage - 0.8).abs() < f64::EPSILON);
        assert_eq!(so.early_synthesis_min_iterations_first_turn, 5);
        assert!((so.early_synthesis_uniform - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn system_one_thresholds_can_be_overridden() {
        let toml = minimal_config(
            r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [system_one]
            endpoint       = "https://so.example.com"
            fast_path      = 0.8
            early_synthesis = 0.9
            relevance      = 0.5
            early_synthesis_research = 0.97
            relevance_preserve_recent = 3
            early_synthesis_coverage = 0.85
            early_synthesis_min_iterations_first_turn = 6
            early_synthesis_uniform = 0.75
        "#,
        );
        let cfg = parse_config(&toml);
        let so = cfg.system_one.expect("system_one should be present");
        assert!((so.fast_path - 0.8).abs() < f64::EPSILON);
        assert!((so.early_synthesis - 0.9).abs() < f64::EPSILON);
        assert!((so.relevance - 0.5).abs() < f64::EPSILON);
        assert!((so.early_synthesis_research - 0.97).abs() < f64::EPSILON);
        assert_eq!(so.relevance_preserve_recent, 3);
        assert!((so.early_synthesis_coverage - 0.85).abs() < f64::EPSILON);
        assert_eq!(so.early_synthesis_min_iterations_first_turn, 6);
        assert!((so.early_synthesis_uniform - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn next_action_confidence_defaults_to_a_half() {
        let toml = minimal_config(&format!(
            r#"{TEST_LLM}
            [system_one]
            endpoint = "http://localhost:1"
            "#
        ));
        let so = parse_config(&toml)
            .system_one
            .expect("system_one should be present");
        assert!((so.next_action_confidence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn next_action_confidence_can_be_overridden() {
        let toml = minimal_config(&format!(
            r#"{TEST_LLM}
            [system_one]
            endpoint = "http://localhost:1"
            next_action_confidence = 0.9
            "#
        ));
        let so = parse_config(&toml)
            .system_one
            .expect("system_one should be present");
        assert!((so.next_action_confidence - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn semantic_search_is_disabled_by_default() {
        let cfg = parse_config(&minimal_config(TEST_LLM));
        let sem = cfg.semantic;
        assert!(!sem.enabled, "semantic search must be opt-in");
        assert_eq!(sem.model, "text-embedding-004");
        assert_eq!(sem.dimensions, 768);
        assert!(!sem.backfill_on_start, "backfill must be opt-in");
        assert_eq!(sem.batch_size, 64);
    }

    #[test]
    fn semantic_weights_default_to_the_agreed_mix() {
        let cfg = parse_config(&minimal_config(TEST_LLM));
        let sem = cfg.semantic;
        assert!((sem.lexical_weight - 0.6).abs() < f64::EPSILON);
        assert!((sem.semantic_weight - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn semantic_settings_are_read_from_toml() {
        let toml = minimal_config(&format!(
            r#"{TEST_LLM}
            [semantic]
            enabled = true
            model = "gemini-embed"
            dimensions = 1024
            lexical_weight = 0.2
            semantic_weight = 0.8
            backfill_on_start = true
            batch_size = 16
            "#,
        ));
        let sem = parse_config(&toml).semantic;
        assert!(sem.enabled);
        assert_eq!(sem.model, "gemini-embed");
        assert_eq!(sem.dimensions, 1024);
        assert!((sem.lexical_weight - 0.2).abs() < f64::EPSILON);
        assert!((sem.semantic_weight - 0.8).abs() < f64::EPSILON);
        assert!(sem.backfill_on_start);
        assert_eq!(sem.batch_size, 16);
    }

    #[test]
    fn semantic_absent_block_falls_back_to_disabled_defaults() {
        let cfg = parse_config(&minimal_config(TEST_LLM));
        let sem = cfg.semantic;
        assert!(!sem.enabled);
        assert_eq!(sem.dimensions, 768);
    }

    #[test]
    fn semantic_config_exposes_effective_weights() {
        let toml = minimal_config(&format!(
            r#"{TEST_LLM}
            [semantic]
            enabled = true
            lexical_weight = 1.0
            semantic_weight = 1.0
            "#,
        ));
        let sem = parse_config(&toml).semantic;
        let (lex, sm) = sem.effective_weights();
        assert!((lex - 0.5).abs() < 1e-9);
        assert!((sm - 0.5).abs() < 1e-9);
    }

    #[test]
    fn semantic_config_caps_the_batch_size() {
        let toml = minimal_config(&format!("{TEST_LLM}\n[semantic]\nbatch_size = 5000\n"));
        let sem = parse_config(&toml).semantic;
        assert_eq!(sem.effective_batch_size(), 64);
    }

    #[test]
    fn semantic_absent_block_is_ignored() {
        let cfg = parse_config(&minimal_config(TEST_LLM));
        let sem = cfg.semantic;
        assert!(!sem.enabled);
        assert_eq!(sem.dimensions, 768);
    }
}

#[cfg(test)]
mod cluster_config_tests {
    use super::*;

    fn with_sections(extra: &str) -> Config {
        let text = format!(
            r#"
            [server]
            [database]
            url = "postgres://harvest:pw@localhost:5432/harvest"
            {extra}
            [auth]
            jwt_secret = "secret"
            [[llm]]
            provider = "gemini"
            model    = "gemini-2.5-flash"
            api_key  = "key1"
        "#
        );
        toml::from_str::<Config>(&text).expect("parse failed")
    }

    #[test]
    fn database_defaults_keep_a_single_node_deployment_working() {
        let cfg = with_sections("");
        assert_eq!(cfg.database.pool_size, 16);
        assert!(cfg.database.ca_file.is_none());
        assert!(cfg.database.migrate_on_start);
    }

    #[test]
    fn database_pool_size_ca_file_and_migration_mode_are_configurable() {
        let cfg = toml::from_str::<Config>(
            r#"
            [server]
            [database]
            url = "postgres://h1:5432,h2:5432/harvest?target_session_attrs=read-write"
            pool_size = 24
            ca_file = "/var/snap/harvest/common/db-ca.pem"
            migrate_on_start = false
            [auth]
            jwt_secret = "secret"
            [[llm]]
            provider = "gemini"
            model    = "gemini-2.5-flash"
            api_key  = "key1"
        "#,
        )
        .unwrap();
        assert_eq!(cfg.database.pool_size, 24);
        assert_eq!(
            cfg.database.ca_file.as_deref(),
            Some(std::path::Path::new("/var/snap/harvest/common/db-ca.pem"))
        );
        assert!(!cfg.database.migrate_on_start);
        let options = cfg.database.options();
        assert_eq!(options.pool_size, 24);
        assert!(options.requires_tls().unwrap());
    }

    #[test]
    fn cluster_defaults_without_a_cluster_section() {
        let cfg = with_sections("");
        assert!(cfg.cluster.internal_listen.is_none());
        assert!(cfg.cluster.internal_url.is_none());
        assert!(cfg.cluster.shared_secret.is_none());
        assert_eq!(cfg.cluster.heartbeat_interval_ms, 5_000);
        assert_eq!(cfg.cluster.node_timeout_ms, 20_000);
        assert_eq!(cfg.cluster.bus_batch_window_ms, 50);
        assert_eq!(cfg.cluster.drain_grace_secs, 10);
        assert_eq!(cfg.cluster.drain_timeout_secs, 120);
        assert!(cfg.cluster.node_name.is_none());
    }

    #[test]
    fn cluster_section_parses() {
        let cfg = toml::from_str::<Config>(
            r#"
            [server]
            [database]
            url = "postgres://harvest:pw@localhost:5432/harvest"
            [cluster]
            node_name = "harvest-server-0"
            internal_listen = "0.0.0.0:8081"
            internal_url = "http://10.0.0.11:8081"
            shared_secret = "cluster-secret"
            heartbeat_interval_ms = 1000
            node_timeout_ms = 4000
            bus_batch_window_ms = 20
            drain_grace_secs = 2
            drain_timeout_secs = 30
            [auth]
            jwt_secret = "secret"
            [[llm]]
            provider = "gemini"
            model    = "gemini-2.5-flash"
            api_key  = "key1"
        "#,
        )
        .unwrap();
        assert_eq!(cfg.cluster.node_name.as_deref(), Some("harvest-server-0"));
        assert_eq!(cfg.cluster.internal_listen.as_deref(), Some("0.0.0.0:8081"));
        assert_eq!(
            cfg.cluster.internal_url.as_deref(),
            Some("http://10.0.0.11:8081")
        );
        assert_eq!(cfg.cluster.shared_secret.as_deref(), Some("cluster-secret"));
        assert_eq!(cfg.cluster.heartbeat_interval_ms, 1000);
        assert_eq!(cfg.cluster.node_timeout_ms, 4000);
        assert_eq!(cfg.cluster.bus_batch_window_ms, 20);
        assert_eq!(cfg.cluster.drain_grace_secs, 2);
        assert_eq!(cfg.cluster.drain_timeout_secs, 30);
    }

    #[test]
    fn cluster_validation_requires_a_secret_when_the_internal_listener_is_enabled() {
        let mut cfg = with_sections("");
        cfg.cluster.internal_listen = Some("0.0.0.0:8081".into());
        cfg.cluster.internal_url = Some("http://10.0.0.11:8081".into());
        assert!(cfg.validate().is_err());
        cfg.cluster.shared_secret = Some("s".into());
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn advertised_url_prefers_the_configured_value() {
        let mut cfg = with_sections("");
        cfg.cluster.internal_url = Some("http://10.0.0.5:8081/".into());
        assert_eq!(
            cfg.cluster
                .advertised_url("0.0.0.0:8081".parse().unwrap())
                .unwrap(),
            "http://10.0.0.5:8081"
        );
    }

    #[test]
    fn advertised_url_is_derived_from_a_specific_bind_address() {
        let cfg = with_sections("");
        assert_eq!(
            cfg.cluster
                .advertised_url("127.0.0.1:9000".parse().unwrap())
                .unwrap(),
            "http://127.0.0.1:9000"
        );
        assert!(cfg
            .cluster
            .advertised_url("0.0.0.0:9000".parse().unwrap())
            .is_err());
    }

    #[test]
    fn node_timeout_must_exceed_twice_the_heartbeat_interval() {
        let mut cfg = with_sections("");
        cfg.cluster.heartbeat_interval_ms = 5_000;
        cfg.cluster.node_timeout_ms = 6_000;
        assert!(cfg.validate().is_err());
    }

    fn quorium_config() -> Config {
        with_sections(
            r#"
            [cluster]
            backend = "quorium"
            internal_listen = "0.0.0.0:8081"
            internal_url = "http://10.0.0.11:8081"
            shared_secret = "s"
            data_dir = "/var/snap/harvest/common/quorium"
            bootstrap = "init"
            heartbeat_interval_ms = 1000
            node_timeout_ms = 5000
            "#,
        )
    }

    #[test]
    fn the_postgres_backend_and_notify_bus_are_the_defaults() {
        let cfg = with_sections("");
        assert_eq!(cfg.cluster.backend, ClusterBackend::Postgres);
        assert_eq!(cfg.cluster.bus_transport(), BusTransport::Notify);
    }

    #[test]
    fn quorium_defaults_to_the_mesh_bus_and_validates() {
        let cfg = quorium_config();
        assert_eq!(cfg.cluster.backend, ClusterBackend::Quorium);
        assert_eq!(cfg.cluster.bootstrap, Some(ClusterBootstrap::Init));
        assert_eq!(cfg.cluster.bus_transport(), BusTransport::Mesh);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn quorium_requires_a_data_dir_and_a_token_to_join() {
        let mut cfg = quorium_config();
        cfg.cluster.data_dir = None;
        assert!(cfg.validate().is_err());
        let mut cfg = quorium_config();
        cfg.cluster.bootstrap = Some(ClusterBootstrap::Join);
        assert!(cfg.validate().is_err());
        cfg.cluster.join_token = Some("token".into());
        assert!(cfg.validate().is_ok());
        cfg.cluster.join_token = None;
        cfg.cluster.join_addresses = vec!["http://10.0.0.10:8081".into()];
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn absent_voters_are_replaced_only_after_their_session_expired() {
        let mut cfg = quorium_config();
        assert_eq!(cfg.cluster.replace_absent_voter_after_secs, 0);
        cfg.cluster.replace_absent_voter_after_secs = 3;
        assert!(cfg.validate().is_err());
        cfg.cluster.replace_absent_voter_after_secs = 600;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn quorium_sessions_must_outlive_an_election() {
        let mut cfg = quorium_config();
        cfg.cluster.election_timeout_ms = 2_000;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn a_join_token_supplies_the_shared_secret() {
        let mut cfg = quorium_config();
        cfg.cluster.shared_secret = None;
        cfg.cluster.bootstrap = Some(ClusterBootstrap::Join);
        cfg.cluster.join_token = Some(
            quorium::JoinToken {
                cluster_id: "c".into(),
                addresses: vec![],
                secret: "once".into(),
                shared_secret: "from-token".into(),
            }
            .encode(),
        );
        assert_eq!(
            cfg.cluster.effective_shared_secret().as_deref(),
            Some("from-token")
        );
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn a_mock_provider_parses_with_defaults_and_overrides() {
        let cfg = toml::from_str::<Config>(
            r#"
            [server]
            [database]
            url = "postgres://harvest:pw@localhost:5432/harvest"
            [auth]
            jwt_secret = "secret"
            [[llm]]
            provider = "mock"
            chunks = 40
            first_token_ms = 250
            max_concurrency = 8
            context_window = 32000
        "#,
        )
        .unwrap();
        let llm = &cfg.llm[0];
        assert_eq!(llm.kind(), "mock");
        assert_eq!(llm.model(), "mock");
        assert_eq!(llm.max_concurrency(), 8);
        assert_eq!(llm.context_window(), Some(32_000));
        let LlmProviderConfig::Mock { behaviour, .. } = llm else {
            panic!("expected a mock provider");
        };
        assert_eq!(behaviour.chunks, 40);
        assert_eq!(behaviour.first_token_ms, 250);
        assert_eq!(behaviour.chunk_delay_ms, 20);
    }

    #[test]
    fn the_mesh_bus_needs_the_internal_listener() {
        let mut cfg = with_sections("");
        cfg.cluster.bus = Some(BusTransport::Mesh);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn secure_cookies_follow_an_https_public_url() {
        let mut cfg = with_sections("");
        assert!(!cfg.auth.secure_cookies());
        cfg.auth.public_url = Some("https://harvest.example.com".into());
        assert!(cfg.auth.secure_cookies());
        cfg.auth.public_url = Some("http://localhost:8080".into());
        assert!(!cfg.auth.secure_cookies());
    }
}
