use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub neo4j: Neo4jConfig,
    pub llm: Vec<LlmProviderConfig>,
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
}

#[derive(Deserialize, Default, Clone)]
pub struct UiConfig {
    #[serde(default)]
    pub enable_docs: bool,
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
}

fn default_true() -> bool { true }

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
    pub public_url:  Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct LxdConfig {
    pub endpoint: String,
    /// Manual override: set both this and `client_key` to manage the client
    /// identity yourself. Omit both to let Harvest generate and self-register
    /// its own identity via `trust_token`.
    #[serde(default)]
    pub client_cert: Option<String>,
    #[serde(default)]
    pub client_key: Option<String>,
    /// One-time token from `lxc config trust add --name <name>` (no cert
    /// argument), used to self-register a Harvest-generated identity. Only
    /// consulted while no trusted identity has been persisted yet.
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

fn default_lxd_project() -> String { "default".into() }
fn default_lxd_image_alias() -> String { "24.04".into() }
fn default_lxd_image_server() -> String { "https://cloud-images.ubuntu.com/releases".into() }
fn default_lxd_profile() -> String { "default".into() }

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
pub struct AgentBehaviorConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_compaction_threshold_chars")]
    pub compaction_threshold_chars: usize,
    #[serde(default = "default_compaction_keep_last")]
    pub compaction_keep_last: usize,
}

impl Default for AgentBehaviorConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            compaction_threshold_chars: default_compaction_threshold_chars(),
            compaction_keep_last: default_compaction_keep_last(),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
pub enum LlmProviderConfig {
    Anthropic {
        model: String,
        api_key: String,
        #[serde(default)]
        priority: u32,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
    Gemini {
        model: String,
        api_key: String,
        #[serde(default)]
        priority: u32,
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
        #[serde(default)]
        priority: u32,
        #[serde(default = "default_timeout_secs")]
        timeout_secs: u64,
        #[serde(default = "default_max_retries")]
        max_retries: u32,
    },
}

impl LlmProviderConfig {
    pub fn priority(&self) -> u32 {
        match self {
            Self::Anthropic    { priority, .. } => *priority,
            Self::Gemini       { priority, .. } => *priority,
            Self::OpenAiCompat { priority, .. } => *priority,
        }
    }
}

fn default_max_iterations() -> usize { 20 }
fn default_timeout_secs() -> u64 { 120 }
fn default_max_retries() -> u32 { 3 }
fn default_compaction_threshold_chars() -> usize { 40_000 }
fn default_compaction_keep_last() -> usize { 6 }

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

    fn parse_auth(toml: &str) -> AuthConfig {
        #[derive(serde::Deserialize)]
        struct Wrapper { auth: AuthConfig }
        toml::from_str::<Wrapper>(toml).expect("parse failed").auth
    }

    fn parse_config(toml: &str) -> Config {
        toml::from_str::<Config>(toml).expect("parse failed")
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
    fn local_login_defaults_to_true() {
        let cfg = parse_auth(r#"
            [auth]
            jwt_secret = "s3cr3t"
        "#);
        assert!(cfg.allow_local_login);
    }

    #[test]
    fn local_login_can_be_disabled() {
        let cfg = parse_auth(r#"
            [auth]
            jwt_secret = "s3cr3t"
            allow_local_login = false
        "#);
        assert!(!cfg.allow_local_login);
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

    fn minimal_config(llm_block: &str) -> String {
        format!(r#"
            [server]
            [neo4j]
            uri = "bolt://localhost:7687"
            user = "neo4j"
            password = "pw"
            [auth]
            jwt_secret = "secret"
            {llm_block}
        "#)
    }

    #[test]
    fn single_gemini_provider_parses() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-2.5-flash"
            api_key  = "key1"
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::Gemini { model, .. } => assert_eq!(model, "gemini-2.5-flash"),
            other => panic!("expected Gemini, got something else: {other:?}", other = std::mem::discriminant(other)),
        }
    }

    #[test]
    fn single_anthropic_provider_parses() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "anthropic"
            model    = "claude-sonnet-4-6"
            api_key  = "key2"
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::Anthropic { model, .. } => assert_eq!(model, "claude-sonnet-4-6"),
            other => panic!("expected Anthropic: {other:?}", other = std::mem::discriminant(other)),
        }
    }

    #[test]
    fn two_providers_parse_as_vec_of_two() {
        let toml = minimal_config(r#"
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
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 2);
        assert_eq!(cfg.llm[0].priority(), 1);
        assert_eq!(cfg.llm[1].priority(), 2);
    }

    #[test]
    fn priority_defaults_to_zero() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "gemini-flash"
            api_key  = "k"
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm[0].priority(), 0);
    }

    #[test]
    fn agent_section_uses_defaults_when_absent() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.agent.max_iterations, 20);
        assert_eq!(cfg.agent.compaction_threshold_chars, 40_000);
        assert_eq!(cfg.agent.compaction_keep_last, 6);
    }

    #[test]
    fn agent_section_parses_explicit_values() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [agent]
            max_iterations = 10
            compaction_threshold_chars = 10000
            compaction_keep_last = 3
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.agent.max_iterations, 10);
        assert_eq!(cfg.agent.compaction_threshold_chars, 10_000);
        assert_eq!(cfg.agent.compaction_keep_last, 3);
    }

    #[test]
    fn openai_compat_provider_parses() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "openai-compatible"
            base_url = "https://openai.example.com"
            api_key  = "k"
            model    = "gpt-4o"
        "#);
        let cfg = parse_config(&toml);
        assert_eq!(cfg.llm.len(), 1);
        match &cfg.llm[0] {
            LlmProviderConfig::OpenAiCompat { model, base_url, .. } => {
                assert_eq!(model, "gpt-4o");
                assert_eq!(base_url, "https://openai.example.com");
            }
            other => panic!("expected OpenAiCompat: {other:?}", other = std::mem::discriminant(other)),
        }
    }

    #[test]
    fn timeout_and_retries_have_defaults() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "anthropic"
            model    = "m"
            api_key  = "k"
        "#);
        let cfg = parse_config(&toml);
        match &cfg.llm[0] {
            LlmProviderConfig::Anthropic { timeout_secs, max_retries, .. } => {
                assert_eq!(*timeout_secs, 120);
                assert_eq!(*max_retries, 3);
            }
            _ => panic!("expected Anthropic"),
        }
    }

    #[test]
    fn lxd_absent_gives_none() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"
        "#);
        let cfg = parse_config(&toml);
        assert!(cfg.lxd.is_none());
    }

    #[test]
    fn lxd_config_parses_required_fields() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint    = "https://lxd.example.com:8443"
            client_cert = "cert-pem"
            client_key  = "key-pem"
        "#);
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
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint = "https://lxd.example.com:8443"
        "#);
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert!(lxd.client_cert.is_none());
        assert!(lxd.client_key.is_none());
        assert!(lxd.trust_token.is_none());
    }

    #[test]
    fn lxd_config_trust_token_parses() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint    = "https://lxd.example.com:8443"
            trust_token = "eyJjbGllbnRfbmFtZSI6Li4ufQ=="
        "#);
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(lxd.trust_token.as_deref(), Some("eyJjbGllbnRfbmFtZSI6Li4ufQ=="));
    }

    #[test]
    fn lxd_config_defaults_apply_when_omitted() {
        let toml = minimal_config(r#"
            [[llm]]
            provider = "gemini"
            model    = "m"
            api_key  = "k"

            [lxd]
            endpoint    = "https://lxd.example.com:8443"
            client_cert = "cert-pem"
            client_key  = "key-pem"
        "#);
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(lxd.project, "default");
        assert_eq!(lxd.image_alias, "24.04");
        assert_eq!(lxd.image_server, "https://cloud-images.ubuntu.com/releases");
        assert_eq!(lxd.profile, "default");
    }

    #[test]
    fn lxd_config_explicit_values_override_defaults() {
        let toml = minimal_config(r#"
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
        "#);
        let cfg = parse_config(&toml);
        let lxd = cfg.lxd.expect("lxd should be present");
        assert_eq!(lxd.ca_cert.as_deref(), Some("ca-pem"));
        assert!(lxd.insecure);
        assert_eq!(lxd.project, "harvest");
        assert_eq!(lxd.image_alias, "22.04");
        assert_eq!(lxd.image_server, "https://images.example.com");
        assert_eq!(lxd.profile, "harvest-profile");
    }
}
