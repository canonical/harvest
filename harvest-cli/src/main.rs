use std::collections::{hash_map::DefaultHasher, BTreeSet, HashMap};
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Password, Select};
use indicatif::{ProgressBar, ProgressStyle};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use serde::{Deserialize, Serialize};

#[derive(Parser)]
#[command(name = "harvest", about = "Harvest operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init(InitArgs),
    PrepareDbScript(PrepareDbScriptArgs),
    Status(StatusArgs),
    ShowPublicUrl(ShowPublicUrlArgs),
    Llm(LlmArgs),
    Identity(IdentityArgs),
    Auth(AuthArgs),
    Security(SecurityArgs),
    Database(DatabaseArgs),
    Server(ServerArgs),
    Ui(UiArgs),
    Agents(AgentsArgs),
    Config(ConfigArgs),
    Cluster(ClusterArgs),
}

#[derive(Args, Clone, Default)]
struct ApplyArgs {
    #[arg(long, action = ArgAction::Set, default_value_t = true)]
    apply_now: bool,
    #[arg(long)]
    no_restart: bool,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser)]
struct LlmArgs {
    #[command(subcommand)]
    command: LlmCommand,
}

#[derive(Subcommand)]
enum LlmCommand {
    Provider(LlmProviderArgs),
}

#[derive(Parser)]
struct LlmProviderArgs {
    #[command(subcommand)]
    command: LlmProviderCommand,
}

#[derive(Subcommand)]
enum LlmProviderCommand {
    List(JsonArgs),
    Show(LlmProviderShowArgs),
    Add(LlmProviderUpsertArgs),
    Update(LlmProviderUpdateArgs),
    Remove(LlmProviderRemoveArgs),
    Enable(LlmProviderEnableArgs),
    Disable(LlmProviderEnableArgs),
    SetPrimary(LlmProviderSetPrimaryArgs),
    Reorder(LlmProviderReorderArgs),
    Models(LlmProviderModelsArgs),
    Key(LlmProviderKeyArgs),
    DetectModels(LlmProviderDetectModelsArgs),
}

#[derive(Parser, Default)]
struct JsonArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct LlmProviderShowArgs {
    id: String,
    #[arg(long)]
    json: bool,
}

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum, Serialize, Deserialize)]
enum ProviderKindArg {
    Anthropic,
    Gemini,
    OpenAiCompatible,
}

impl ProviderKindArg {
    fn to_provider_kind(self) -> ProviderKind {
        match self {
            ProviderKindArg::Anthropic => ProviderKind::Anthropic,
            ProviderKindArg::Gemini => ProviderKind::Gemini,
            ProviderKindArg::OpenAiCompatible => ProviderKind::OpenAiCompatible,
        }
    }
}

#[derive(Parser)]
struct LlmProviderUpsertArgs {
    #[arg(long)]
    id: Option<String>,
    #[arg(long, value_enum)]
    provider: Option<ProviderKindArg>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    user_provided_key: bool,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    #[arg(long)]
    models: Option<String>,
    #[arg(long)]
    timeout_secs: Option<u32>,
    #[arg(long)]
    max_retries: Option<u32>,
    #[arg(long)]
    priority: Option<i32>,
    #[arg(long)]
    expose_to_ui: Option<bool>,
    #[arg(long)]
    enabled: Option<bool>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderUpdateArgs {
    id: String,
    #[arg(long, value_enum)]
    provider: Option<ProviderKindArg>,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    clear_api_key: bool,
    #[arg(long)]
    user_provided_key: Option<bool>,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    #[arg(long)]
    models: Option<String>,
    #[arg(long)]
    timeout_secs: Option<u32>,
    #[arg(long)]
    max_retries: Option<u32>,
    #[arg(long)]
    priority: Option<i32>,
    #[arg(long)]
    expose_to_ui: Option<bool>,
    #[arg(long)]
    enabled: Option<bool>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderRemoveArgs {
    id: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderEnableArgs {
    id: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderSetPrimaryArgs {
    id: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderReorderArgs {
    #[arg(long)]
    order: Option<String>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderModelsArgs {
    id: String,
    #[command(subcommand)]
    command: LlmProviderModelsCommand,
}

#[derive(Subcommand)]
enum LlmProviderModelsCommand {
    Set(LlmProviderModelsSetArgs),
    Edit(LlmProviderModelsEditArgs),
    Clear(LlmProviderModelsClearArgs),
}

#[derive(Parser)]
struct LlmProviderModelsSetArgs {
    models: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderModelsEditArgs {
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderModelsClearArgs {
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderKeyArgs {
    id: String,
    #[command(subcommand)]
    command: LlmProviderKeyCommand,
}

#[derive(Subcommand)]
enum LlmProviderKeyCommand {
    Set(LlmProviderKeySetArgs),
    Clear(LlmProviderKeyClearArgs),
    UserProvided(LlmProviderKeyUserProvidedArgs),
}

#[derive(Parser)]
struct LlmProviderKeySetArgs {
    key: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderKeyClearArgs {
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderKeyUserProvidedArgs {
    value: bool,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct LlmProviderDetectModelsArgs {
    id: String,
    #[arg(long)]
    set_primary: bool,
    #[arg(long)]
    set_allowlist: bool,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct IdentityArgs {
    #[command(subcommand)]
    command: IdentityCommand,
}

#[derive(Subcommand)]
enum IdentityCommand {
    Provider(IdentityProviderArgs),
}

#[derive(Parser)]
struct IdentityProviderArgs {
    #[command(subcommand)]
    command: IdentityProviderCommand,
}

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum)]
enum IdentityProviderKind {
    Google,
    Oidc,
}

#[derive(Subcommand)]
enum IdentityProviderCommand {
    List(JsonArgs),
    Show(IdentityProviderShowArgs),
    Enable(IdentityProviderUpdateArgs),
    Disable(IdentityProviderDisableArgs),
    Update(IdentityProviderUpdateArgs),
}

#[derive(Parser)]
struct IdentityProviderShowArgs {
    provider: IdentityProviderKind,
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct IdentityProviderDisableArgs {
    provider: IdentityProviderKind,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct IdentityProviderUpdateArgs {
    provider: IdentityProviderKind,
    #[arg(long)]
    client_id: Option<String>,
    #[arg(long)]
    client_secret: Option<String>,
    #[arg(long)]
    redirect_uri: Option<String>,
    #[arg(long)]
    issuer_url: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Subcommand)]
enum AuthCommand {
    LocalLogin(AuthLocalLoginArgs),
    Jwt(AuthJwtArgs),
    PublicUrl(AuthPublicUrlArgs),
}

#[derive(Parser)]
struct AuthLocalLoginArgs {
    #[command(subcommand)]
    command: AuthLocalLoginCommand,
}

#[derive(Subcommand)]
enum AuthLocalLoginCommand {
    Get(JsonArgs),
    Set(AuthLocalLoginSetArgs),
}

#[derive(Parser)]
struct AuthLocalLoginSetArgs {
    value: bool,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct AuthJwtArgs {
    #[command(subcommand)]
    command: AuthJwtCommand,
}

#[derive(Subcommand)]
enum AuthJwtCommand {
    Rotate(AuthJwtRotateArgs),
}

#[derive(Parser)]
struct AuthJwtRotateArgs {
    #[arg(long)]
    auto: bool,
    #[arg(long)]
    value: Option<String>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct AuthPublicUrlArgs {
    #[command(subcommand)]
    command: AuthPublicUrlCommand,
}

#[derive(Subcommand)]
enum AuthPublicUrlCommand {
    Get(JsonArgs),
    Set(AuthPublicUrlSetArgs),
}

#[derive(Parser)]
struct AuthPublicUrlSetArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct SecurityArgs {
    #[command(subcommand)]
    command: SecurityCommand,
}

#[derive(Subcommand)]
enum SecurityCommand {
    UserKeyEncryptionKey(SecurityUserKeyArgs),
}

#[derive(Parser)]
struct SecurityUserKeyArgs {
    #[command(subcommand)]
    command: SecurityUserKeyCommand,
}

#[derive(Subcommand)]
enum SecurityUserKeyCommand {
    Set(SecurityUserKeySetArgs),
    Rotate(SecurityUserKeyRotateArgs),
    Clear(SecurityUserKeyClearArgs),
}

#[derive(Parser)]
struct SecurityUserKeySetArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct SecurityUserKeyRotateArgs {
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct SecurityUserKeyClearArgs {
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct DatabaseArgs {
    #[command(subcommand)]
    command: DatabaseCommand,
}

#[derive(Subcommand)]
enum DatabaseCommand {
    Get(JsonArgs),
    SetUrl(DatabaseSetUrlArgs),
    SetPoolSize(DatabaseSetPoolSizeArgs),
    SetCaFile(DatabaseSetCaFileArgs),
}

#[derive(Parser)]
struct DatabaseSetUrlArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct DatabaseSetPoolSizeArgs {
    value: u32,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct DatabaseSetCaFileArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct ServerArgs {
    #[command(subcommand)]
    command: ServerCommand,
}

#[derive(Subcommand)]
enum ServerCommand {
    Get(JsonArgs),
    SetHost(ServerSetHostArgs),
    SetPort(ServerSetPortArgs),
}

#[derive(Parser)]
struct ServerSetHostArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct ServerSetPortArgs {
    value: u16,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct UiArgs {
    #[command(subcommand)]
    command: UiCommand,
}

#[derive(Subcommand)]
enum UiCommand {
    Get(JsonArgs),
    SetHost(UiSetHostArgs),
    SetPort(UiSetPortArgs),
    SetMode(UiSetModeArgs),
    EnableDocs(UiEnableDocsArgs),
}

#[derive(Parser)]
struct UiSetHostArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct UiSetPortArgs {
    value: u16,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct UiSetModeArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct UiEnableDocsArgs {
    value: bool,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct AgentsArgs {
    #[command(subcommand)]
    command: AgentsCommand,
}

#[derive(Subcommand)]
enum AgentsCommand {
    Get(JsonArgs),
    SetPublicUrl(AgentsSetPublicUrlArgs),
}

#[derive(Parser)]
struct AgentsSetPublicUrlArgs {
    value: String,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct ConfigArgs {
    #[command(subcommand)]
    command: ConfigCommand,
}

#[derive(Subcommand)]
enum ConfigCommand {
    Get(ConfigGetArgs),
    Set(ConfigSetArgs),
    Unset(ConfigUnsetArgs),
    List(ConfigListArgs),
    Export(ConfigExportArgs),
    Import(ConfigImportArgs),
    Diff(ConfigDiffArgs),
    Apply(ConfigApplyArgs),
    Validate(ConfigValidateArgs),
}

#[derive(Parser)]
struct ConfigGetArgs {
    key: String,
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ConfigSetArgs {
    entries: Vec<String>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct ConfigUnsetArgs {
    keys: Vec<String>,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct ConfigListArgs {
    #[arg(long)]
    prefix: Option<String>,
    #[arg(long)]
    json: bool,
}

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum)]
enum ExportFormat {
    Json,
    Yaml,
}

#[derive(Parser)]
struct ConfigExportArgs {
    #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
    format: ExportFormat,
}

#[derive(Parser)]
struct ConfigImportArgs {
    file: PathBuf,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser)]
struct ConfigDiffArgs {
    #[arg(long)]
    file: PathBuf,
}

#[derive(Parser)]
struct ConfigApplyArgs {
    #[arg(long)]
    no_restart: bool,
}

#[derive(Parser, Default)]
struct ConfigValidateArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ClusterArgs {
    #[command(subcommand)]
    command: ClusterCommand,
}

#[derive(Subcommand)]
enum ClusterCommand {
    Init(ClusterInitArgs),
    Join(ClusterJoinArgs),
    Preseed(ClusterPreseedArgs),
    Backend(ClusterBackendArgs),
    Status(ClusterStatusArgs),
    List(ClusterListArgs),
    Remove(ClusterNodeArgs),
    Evacuate(ClusterNodeArgs),
    Restore(ClusterNodeArgs),
    RotateSecrets,
    Token(ClusterTokenArgs),
}

#[derive(Parser)]
struct ClusterInitArgs {
    #[arg(long)]
    node_name: Option<String>,
    #[arg(long, default_value = "0.0.0.0:8081")]
    internal_listen: String,
    #[arg(long)]
    internal_url: Option<String>,
    #[arg(long)]
    shared_secret: Option<String>,
    #[arg(long, value_enum)]
    backend: Option<ClusterBackend>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser)]
struct ClusterJoinArgs {
    #[arg(long)]
    node_name: Option<String>,
    #[arg(long, default_value = "0.0.0.0:8081")]
    internal_listen: String,
    #[arg(long)]
    internal_url: Option<String>,
    #[arg(long)]
    shared_secret: Option<String>,
    #[arg(long)]
    token: Option<String>,
    #[arg(long, value_enum)]
    backend: Option<ClusterBackend>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser)]
struct ClusterPreseedArgs {
    #[arg(long)]
    file: Option<PathBuf>,
    #[arg(long)]
    dry_run: bool,
}

#[derive(Parser)]
struct ClusterBackendArgs {
    #[command(subcommand)]
    command: ClusterBackendCommand,
}

#[derive(Subcommand)]
enum ClusterBackendCommand {
    Get(JsonArgs),
    Set(ClusterBackendSetArgs),
}

#[derive(Parser)]
struct ClusterBackendSetArgs {
    #[arg(value_enum)]
    value: ClusterBackend,
    #[command(flatten)]
    apply: ApplyArgs,
}

#[derive(Parser, Default)]
struct ClusterStatusArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Parser, Default)]
struct ClusterListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Parser)]
struct ClusterNodeArgs {
    node: String,
}

#[derive(Parser)]
struct ClusterTokenArgs {
    #[command(subcommand)]
    command: ClusterTokenCommand,
}

#[derive(Subcommand)]
enum ClusterTokenCommand {
    Create(ClusterTokenCreateArgs),
    List(ClusterTokenListArgs),
    Revoke,
}

#[derive(Parser)]
struct ClusterTokenCreateArgs {
    #[arg(long, default_value_t = 1800)]
    ttl_secs: u64,
    #[arg(long)]
    json: bool,
}

#[derive(Parser, Default)]
struct ClusterTokenListArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Parser, Default)]
struct InitArgs {
    #[arg(long)]
    from_db_url: Option<String>,
}

#[derive(Parser)]
struct PrepareDbScriptArgs {
    #[arg(long, default_value = "harvest")]
    db_name: String,
    #[arg(long, default_value = "harvest")]
    db_user: String,
    #[arg(long, default_value = "localhost")]
    db_host: String,
    #[arg(long, default_value_t = 5432)]
    db_port: u16,
    #[arg(long)]
    db_password: Option<String>,
    #[arg(long, default_value = "postgresql")]
    postgresql_snap: String,
}

#[derive(Parser, Default)]
struct StatusArgs {
    #[arg(long)]
    json: bool,
    #[arg(long)]
    verbose: bool,
}

#[derive(Copy, Clone, Eq, PartialEq, ValueEnum)]
enum PublicUrlTarget {
    Auto,
    Auth,
    Agents,
    Ui,
}

#[derive(Parser)]
struct ShowPublicUrlArgs {
    #[arg(long, value_enum, default_value_t = PublicUrlTarget::Auto)]
    r#for: PublicUrlTarget,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    explain: bool,
}

#[derive(Clone, Copy)]
enum ProviderKind {
    Anthropic,
    Gemini,
    OpenAiCompatible,
}

impl ProviderKind {
    fn as_snap_value(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Gemini => "gemini",
            ProviderKind::OpenAiCompatible => "openai-compatible",
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "https://api.anthropic.com",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com",
            ProviderKind::OpenAiCompatible => "https://api.openai.com/v1",
        }
    }

    fn default_model(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "claude-sonnet-4-6",
            ProviderKind::Gemini => "gemini-2.5-flash",
            ProviderKind::OpenAiCompatible => "gpt-4o-mini",
        }
    }
}

impl Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Gemini => "gemini",
            ProviderKind::OpenAiCompatible => "openai-compatible",
        })
    }
}

#[derive(Serialize)]
struct ServiceReport {
    service: String,
    startup: String,
    current: String,
    healthy: bool,
    detail: String,
}

#[derive(Serialize)]
struct StatusReport {
    overall: String,
    config_complete: bool,
    services: Vec<ServiceReport>,
}

#[derive(Serialize)]
struct PublicUrlReport {
    url: String,
    source: String,
}

#[derive(Serialize, Deserialize)]
struct ModelCacheEntry {
    fetched_unix: u64,
    models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct LlmProviderConfig {
    provider: String,
    #[serde(rename = "api-key", default)]
    api_key: String,
    #[serde(rename = "user-provided-key", default)]
    user_provided_key: bool,
    #[serde(default)]
    model: String,
    #[serde(default)]
    priority: i32,
    #[serde(rename = "base-url", default)]
    base_url: String,
    #[serde(rename = "timeout-secs", default)]
    timeout_secs: u32,
    #[serde(rename = "max-retries", default)]
    max_retries: u32,
    #[serde(rename = "display-name", default)]
    display_name: String,
    #[serde(default)]
    models: String,
    #[serde(rename = "expose-to-ui", default = "default_true")]
    expose_to_ui: bool,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
struct ValidationReport {
    ok: bool,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct ClusterStatusReport {
    managed: bool,
    backend: String,
    node_name: String,
    internal_listen: String,
    internal_url: String,
    healthy: bool,
    members: Vec<ClusterMember>,
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
enum ClusterBackend {
    Postgres,
    Quorium,
}

impl ClusterBackend {
    fn as_snap_value(self) -> &'static str {
        match self {
            ClusterBackend::Postgres => "postgres",
            ClusterBackend::Quorium => "quorium",
        }
    }

    fn from_snap_value(value: &str) -> Result<Self> {
        match value.trim() {
            "" | "postgres" => Ok(ClusterBackend::Postgres),
            "quorium" => Ok(ClusterBackend::Quorium),
            other => bail!("unsupported cluster backend: {other}"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Eq, PartialEq, Debug)]
struct ClusterMember {
    name: String,
    url: String,
}

#[derive(Serialize)]
struct ClusterTokenReport {
    token: String,
    expires_unix: u64,
}

#[derive(Deserialize)]
struct ClusterPreseed {
    cluster: Option<ClusterPreseedCluster>,
    systems: Option<Vec<ClusterPreseedSystem>>,
}

#[derive(Deserialize)]
struct ClusterPreseedCluster {
    backend: Option<ClusterBackend>,
    internal_listen: Option<String>,
    shared_secret: Option<String>,
}

#[derive(Deserialize)]
struct ClusterPreseedSystem {
    name: String,
    address: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init(args) => run_init(args).await,
        Commands::PrepareDbScript(args) => run_prepare_db_script(args),
        Commands::Status(args) => run_status(args).await,
        Commands::ShowPublicUrl(args) => run_show_public_url(args),
        Commands::Llm(args) => run_llm(args).await,
        Commands::Identity(args) => run_identity(args),
        Commands::Auth(args) => run_auth(args),
        Commands::Security(args) => run_security(args),
        Commands::Database(args) => run_database(args),
        Commands::Server(args) => run_server(args),
        Commands::Ui(args) => run_ui(args),
        Commands::Agents(args) => run_agents(args),
        Commands::Config(args) => run_config(args),
        Commands::Cluster(args) => run_cluster(args).await,
    }
}

async fn run_init(args: InitArgs) -> Result<()> {
    require_snapctl()?;
    let theme = ColorfulTheme::default();

    let current_db = snapctl_get("database.url").unwrap_or_default();
    let current_server_host = non_empty_or(snapctl_get("server.host")?, "127.0.0.1");
    let current_server_port = non_empty_or(snapctl_get("server.port")?, "8080");
    let current_ui_host = non_empty_or(snapctl_get("ui.host")?, "127.0.0.1");
    let current_ui_port = non_empty_or(snapctl_get("ui.port")?, "3000");
    let current_ui_mode = non_empty_or(snapctl_get("ui.mode")?, "proxy");
    let current_allow_local = non_empty_or(snapctl_get("auth.allow-local-login")?, "true");
    let current_user_key_encryption_key =
        non_empty_or(snapctl_get("security.user-key-encryption-key")?, "");

    let database_url = if let Some(url) = args.from_db_url {
        validate_database_url(&url)?;
        url
    } else if !current_db.trim().is_empty() {
        validate_database_url(&current_db)?;
        current_db
    } else {
        ask_database_url(&theme, None)?
    };

    let secret_mode_items = vec!["Generate random jwt secret", "Enter jwt secret manually"];
    let secret_mode = Select::with_theme(&theme)
        .with_prompt("auth.jwt-secret")
        .items(&secret_mode_items)
        .default(0)
        .interact()?;
    let jwt_secret = if secret_mode == 0 {
        generate_secret(64)
    } else {
        Password::with_theme(&theme)
            .with_prompt("JWT secret")
            .with_confirmation("Confirm JWT secret", "Secrets do not match")
            .interact()?
    };

    let server_host: String = Input::with_theme(&theme)
        .with_prompt("server.host")
        .default(current_server_host)
        .interact_text()?;
    let server_port: u16 = Input::with_theme(&theme)
        .with_prompt("server.port")
        .default(current_server_port.parse::<u16>().unwrap_or(8080))
        .interact_text()?;
    let ui_host: String = Input::with_theme(&theme)
        .with_prompt("ui.host")
        .default(current_ui_host)
        .interact_text()?;
    let ui_port: u16 = Input::with_theme(&theme)
        .with_prompt("ui.port")
        .default(current_ui_port.parse::<u16>().unwrap_or(3000))
        .interact_text()?;

    let ui_modes = vec!["proxy", "static"];
    let default_ui_mode = ui_modes
        .iter()
        .position(|m| *m == current_ui_mode)
        .unwrap_or(0);
    let ui_mode_idx = Select::with_theme(&theme)
        .with_prompt("ui.mode")
        .items(&ui_modes)
        .default(default_ui_mode)
        .interact()?;
    let ui_mode = ui_modes[ui_mode_idx].to_string();

    let allow_local_default = current_allow_local.trim().eq_ignore_ascii_case("true");
    let allow_local_login = Confirm::with_theme(&theme)
        .with_prompt("Allow local username/password login?")
        .default(allow_local_default)
        .interact()?;

    let provider_options = vec!["anthropic", "gemini", "openai-compatible"];
    let provider_choice = Select::with_theme(&theme)
        .with_prompt("Primary LLM provider type")
        .items(&provider_options)
        .default(0)
        .interact()?;
    let provider_kind = match provider_choice {
        0 => ProviderKind::Anthropic,
        1 => ProviderKind::Gemini,
        _ => ProviderKind::OpenAiCompatible,
    };

    let provider_url: String = Input::with_theme(&theme)
        .with_prompt("Provider URL")
        .default(provider_kind.default_base_url().to_string())
        .interact_text()?;
    let use_user_provided_key = Confirm::with_theme(&theme)
        .with_prompt("Use user-provided API keys?")
        .default(false)
        .interact()?;

    let user_key_encryption_key = if use_user_provided_key {
        println!();
        println!("User-provided API keys are encrypted at rest.");
        println!("An encryption key is required to store and decrypt those keys securely.");
        println!();
        let key_mode_items = vec![
            "Generate random encryption key",
            "Enter encryption key manually",
        ];
        let key_mode = Select::with_theme(&theme)
            .with_prompt("security.user-key-encryption-key")
            .items(&key_mode_items)
            .default(0)
            .interact()?;
        if key_mode == 0 {
            generate_hex_secret(64)
        } else {
            let default = if current_user_key_encryption_key.trim().is_empty() {
                None
            } else {
                Some(current_user_key_encryption_key.clone())
            };
            let mut input = Password::with_theme(&theme)
                .with_prompt("Encryption key")
                .with_confirmation("Confirm encryption key", "Keys do not match");
            if let Some(v) = default {
                input = input.with_confirmation(
                    "Confirm encryption key",
                    "Keys do not match",
                );
                if Confirm::with_theme(&theme)
                    .with_prompt("Reuse existing encryption key from config?")
                    .default(true)
                    .interact()?
                {
                    v
                } else {
                    input.interact()?
                }
            } else {
                input.interact()?
            }
        }
    } else {
        String::new()
    };

    if use_user_provided_key {
        validate_user_key_encryption_key(&user_key_encryption_key)?;
    }

    let provider_api_key = if use_user_provided_key {
        String::new()
    } else {
        Password::with_theme(&theme)
            .with_prompt("Provider API key")
            .with_confirmation("Confirm provider API key", "Keys do not match")
            .interact()?
    };

    let detected_models = if use_user_provided_key && provider_api_key.trim().is_empty() {
        load_model_cache(provider_kind, &provider_url)?.unwrap_or_default()
    } else if let Some(cached) = load_model_cache(provider_kind, &provider_url)? {
        cached
    } else {
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(ProgressStyle::with_template("{spinner} {msg}")?);
        spinner.enable_steady_tick(Duration::from_millis(100));
        spinner.set_message("Detecting available models");
        let models = detect_models(provider_kind, &provider_url, &provider_api_key)
            .await
            .unwrap_or_default();
        spinner.finish_and_clear();
        if !models.is_empty() {
            let _ = store_model_cache(provider_kind, &provider_url, &models);
        }
        models
    };

    let model = if detected_models.is_empty() {
        Input::with_theme(&theme)
            .with_prompt("llm.primary.model")
            .default(provider_kind.default_model().to_string())
            .interact_text()?
    } else {
        let default_idx = detected_models
            .iter()
            .position(|m| m == provider_kind.default_model())
            .unwrap_or(0);
        let idx = FuzzySelect::with_theme(&theme)
            .with_prompt("Select primary model")
            .items(&detected_models)
            .default(default_idx)
            .interact()?;
        detected_models[idx].clone()
    };

    let models_allowlist = if detected_models.is_empty() {
        let use_allowlist = Confirm::with_theme(&theme)
            .with_prompt("Set llm.primary.models allowlist?")
            .default(false)
            .interact()?;
        if use_allowlist {
            let value: String = Input::with_theme(&theme)
                .with_prompt("Comma-separated models")
                .interact_text()?;
            let items: Vec<String> = value
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if items.is_empty() {
                None
            } else {
                Some(items)
            }
        } else {
            None
        }
    } else {
        select_models_allowlist(&theme, &detected_models, &model)?
    };

    let mut settings = vec![
        ("config.managed".to_string(), "false".to_string()),
        ("database.url".to_string(), database_url),
        ("auth.jwt-secret".to_string(), jwt_secret),
        ("server.host".to_string(), server_host),
        ("server.port".to_string(), server_port.to_string()),
        ("ui.host".to_string(), ui_host),
        ("ui.port".to_string(), ui_port.to_string()),
        ("ui.mode".to_string(), ui_mode),
        (
            "auth.allow-local-login".to_string(),
            allow_local_login.to_string(),
        ),
        (
            "llm.primary.provider".to_string(),
            provider_kind.as_snap_value().to_string(),
        ),
        (
            "llm.primary.user-provided-key".to_string(),
            use_user_provided_key.to_string(),
        ),
        ("llm.primary.api-key".to_string(), provider_api_key),
        ("llm.primary.model".to_string(), model),
        ("llm.primary.priority".to_string(), "0".to_string()),
    ];

    if let ProviderKind::OpenAiCompatible = provider_kind {
        settings.push(("llm.primary.base-url".to_string(), provider_url));
    }

    if use_user_provided_key {
        settings.push((
            "security.user-key-encryption-key".to_string(),
            user_key_encryption_key,
        ));
    }

    if let Some(models) = models_allowlist {
        settings.push(("llm.primary.models".to_string(), models.join(",")));
    }

    println!();
    println!("Pending configuration:");
    for (k, v) in &settings {
        if k.contains("secret") || k.contains("api-key") || k.contains("encryption-key") {
            println!("  {k}=<redacted>");
        } else {
            println!("  {k}={v}");
        }
    }

    let apply = Confirm::with_theme(&theme)
        .with_prompt("Apply configuration and start services?")
        .default(true)
        .interact()?;
    if !apply {
        println!("Aborted without changes.");
        return Ok(());
    }

    if !running_as_root() {
        bail!("init must be run as root inside the snap environment (try: sudo harvest init)");
    }

    apply_snap_settings(&settings)?;
    run_configure_hook()?;
    let _ = run_snapctl(["start", "--enable", "harvest.server"]);
    let _ = run_snapctl(["start", "--enable", "harvest.ui"]);

    println!();
    println!("Configuration applied.");
    run_status(StatusArgs {
        json: false,
        verbose: false,
    })
    .await
}

fn ask_database_url(theme: &ColorfulTheme, default: Option<String>) -> Result<String> {
    let mut input = Input::with_theme(theme);
    input = input.with_prompt("database.url");
    if let Some(v) = default {
        input = input.default(v);
    }
    let url: String = input.interact_text()?;
    validate_database_url(&url)?;
    Ok(url)
}

fn select_models_allowlist(
    theme: &ColorfulTheme,
    all_models: &[String],
    primary: &str,
) -> Result<Option<Vec<String>>> {
    if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
        if let Ok(selected) = pick_models_tui(all_models, primary) {
            if selected.is_empty() {
                return Ok(None);
            }
            return Ok(Some(selected));
        }
    }
    select_models_multi_fallback(theme, all_models, primary)
}

fn select_models_multi_fallback(
    theme: &ColorfulTheme,
    all_models: &[String],
    primary: &str,
) -> Result<Option<Vec<String>>> {
    let mut selected = BTreeSet::new();
    selected.insert(primary.to_string());
    loop {
        let query: String = Input::with_theme(theme)
            .with_prompt("Search models for allowlist (blank shows all)")
            .allow_empty(true)
            .interact_text()?;

        let lower_query = query.to_lowercase();
        let mut ordered: Vec<String> = selected.iter().cloned().collect();
        let mut matched: Vec<String> = all_models
            .iter()
            .filter(|m| {
                if selected.contains(*m) {
                    return false;
                }
                if lower_query.is_empty() {
                    true
                } else {
                    m.to_lowercase().contains(&lower_query)
                }
            })
            .cloned()
            .collect();
        matched.sort();
        ordered.extend(matched);

        if ordered.is_empty() {
            let done = Confirm::with_theme(theme)
                .with_prompt("No models match. Finish with only selected entries?")
                .default(true)
                .interact()?;
            if done {
                break;
            }
            continue;
        }

        let defaults: Vec<bool> = ordered.iter().map(|m| selected.contains(m)).collect();
        let picked = MultiSelect::with_theme(theme)
            .with_prompt("Select models (space toggles, enter confirms)")
            .items(&ordered)
            .defaults(&defaults)
            .interact()?;

        selected = picked.into_iter().map(|idx| ordered[idx].clone()).collect();
        if selected.is_empty() {
            selected.insert(primary.to_string());
        }

        let finish = Confirm::with_theme(theme)
            .with_prompt("Finish model allowlist selection?")
            .default(true)
            .interact()?;
        if finish {
            break;
        }
    }

    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected.into_iter().collect()))
    }
}

fn pick_models_tui(all_models: &[String], primary: &str) -> Result<Vec<String>> {
    struct State {
        query: String,
        selected: BTreeSet<String>,
        ordered: Vec<String>,
        list_state: ListState,
        focus_search: bool,
        last_input: std::time::Instant,
        dirty: bool,
    }

    impl State {
        fn new(all_models: &[String], primary: &str) -> Self {
            let mut selected = BTreeSet::new();
            selected.insert(primary.to_string());
            let mut state = Self {
                query: String::new(),
                selected,
                ordered: Vec::new(),
                list_state: ListState::default(),
                focus_search: true,
                last_input: std::time::Instant::now(),
                dirty: true,
            };
            state.rebuild(all_models);
            state
        }

        fn rebuild(&mut self, all_models: &[String]) {
            if !self.dirty {
                return;
            }
            let q = self.query.to_lowercase();
            let mut pinned: Vec<String> = self.selected.iter().cloned().collect();
            pinned.sort();
            let mut rest: Vec<String> = all_models
                .iter()
                .filter(|m| !self.selected.contains(*m))
                .filter(|m| q.is_empty() || m.to_lowercase().contains(&q))
                .cloned()
                .collect();
            rest.sort();
            self.ordered = pinned;
            self.ordered.extend(rest);
            if self.ordered.is_empty() {
                self.list_state.select(None);
            } else {
                let idx = self
                    .list_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.ordered.len() - 1);
                self.list_state.select(Some(idx));
            }
            self.dirty = false;
        }
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut state = State::new(all_models, primary);
    let mut result: Option<Vec<String>> = None;

    let loop_result = (|| -> Result<()> {
        loop {
            if state.last_input.elapsed() >= Duration::from_secs(2) {
                state.rebuild(all_models);
            }
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(1),
                        Constraint::Length(2),
                    ])
                    .split(f.area());
                let search_title = if state.focus_search {
                    "Search (focused)"
                } else {
                    "Search"
                };
                let search = Paragraph::new(state.query.clone())
                    .block(Block::default().borders(Borders::ALL).title(search_title));
                f.render_widget(search, chunks[0]);

                let items: Vec<ListItem> = state
                    .ordered
                    .iter()
                    .map(|m| {
                        let is_primary = m == primary;
                        let mark = if state.selected.contains(m) {
                            "[x]"
                        } else {
                            "[ ]"
                        };
                        let suffix = if is_primary { " (primary)" } else { "" };
                        ListItem::new(Line::from(format!("{mark} {m}{suffix}")))
                    })
                    .collect();
                let list_title = if state.focus_search {
                    "Models"
                } else {
                    "Models (focused)"
                };
                let list = List::new(items)
                    .block(Block::default().borders(Borders::ALL).title(list_title))
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
                f.render_stateful_widget(list, chunks[1], &mut state.list_state);

                let hint = Paragraph::new(
                    "↑/↓ move focus/cursor, space toggle, enter confirm, esc cancel",
                )
                .block(Block::default().borders(Borders::ALL));
                f.render_widget(hint, chunks[2]);
            })?;

            if !event::poll(Duration::from_millis(120))? {
                continue;
            }
            let evt = event::read()?;
            let CEvent::Key(key) = evt else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            state.last_input = std::time::Instant::now();
            match key.code {
                KeyCode::Esc => {
                    result = Some(vec![primary.to_string()]);
                    break;
                }
                KeyCode::Enter => {
                    let mut out: Vec<String> = state.selected.iter().cloned().collect();
                    out.sort();
                    if !out.iter().any(|m| m == primary) {
                        out.push(primary.to_string());
                        out.sort();
                    }
                    result = Some(out);
                    break;
                }
                KeyCode::Up => {
                    if state.focus_search {
                        continue;
                    }
                    if let Some(idx) = state.list_state.selected() {
                        if idx == 0 {
                            state.focus_search = true;
                        } else {
                            state.list_state.select(Some(idx - 1));
                        }
                    }
                }
                KeyCode::Down => {
                    if state.focus_search {
                        state.focus_search = false;
                        if state.list_state.selected().is_none() && !state.ordered.is_empty() {
                            state.list_state.select(Some(0));
                        }
                        continue;
                    }
                    if let Some(idx) = state.list_state.selected() {
                        let next = (idx + 1).min(state.ordered.len().saturating_sub(1));
                        state.list_state.select(Some(next));
                    }
                }
                KeyCode::Char(' ') => {
                    if state.focus_search {
                        continue;
                    }
                    if let Some(idx) = state.list_state.selected() {
                        if let Some(name) = state.ordered.get(idx) {
                            if name == primary {
                                continue;
                            }
                            if state.selected.contains(name) {
                                state.selected.remove(name);
                            } else {
                                state.selected.insert(name.clone());
                            }
                            state.dirty = true;
                            state.rebuild(all_models);
                        }
                    }
                }
                KeyCode::Backspace => {
                    if state.focus_search {
                        state.query.pop();
                        state.dirty = true;
                    }
                }
                KeyCode::Char(c) => {
                    if state.focus_search {
                        state.query.push(c);
                        state.dirty = true;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();

    let _ = disable_raw_mode();
    let _ = terminal.backend_mut().execute(LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    loop_result?;
    Ok(result.unwrap_or_else(|| vec![primary.to_string()]))
}

async fn detect_models(kind: ProviderKind, base_url: &str, api_key: &str) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()?;
    let mut models = match kind {
        ProviderKind::OpenAiCompatible => detect_openai_models(&client, base_url, api_key).await?,
        ProviderKind::Anthropic => detect_anthropic_models(&client, base_url, api_key).await?,
        ProviderKind::Gemini => detect_gemini_models(&client, base_url, api_key).await?,
    };
    models.sort();
    models.dedup();
    Ok(models)
}

async fn detect_openai_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = client.get(url).bearer_auth(api_key).send().await?;
    if !resp.status().is_success() {
        bail!("failed to detect models: {}", resp.status());
    }
    let value: serde_json::Value = resp.json().await?;
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("models response missing data array"))?;
    Ok(data
        .iter()
        .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
        .map(ToOwned::to_owned)
        .collect())
}

async fn detect_anthropic_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let resp = client
        .get(url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("failed to detect models: {}", resp.status());
    }
    let value: serde_json::Value = resp.json().await?;
    let data = value
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("models response missing data array"))?;
    Ok(data
        .iter()
        .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
        .map(ToOwned::to_owned)
        .collect())
}

async fn detect_gemini_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<String>> {
    let root = base_url.trim_end_matches('/');
    let url = if root.ends_with("/v1beta") {
        format!("{root}/models?key={api_key}")
    } else {
        format!("{root}/v1beta/models?key={api_key}")
    };
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("failed to detect models: {}", resp.status());
    }
    let value: serde_json::Value = resp.json().await?;
    let data = value
        .get("models")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("models response missing models array"))?;
    Ok(data
        .iter()
        .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
        .map(|name| name.trim_start_matches("models/").to_string())
        .collect())
}

fn run_prepare_db_script(args: PrepareDbScriptArgs) -> Result<()> {
    validate_identifier("db-name", &args.db_name)?;
    validate_identifier("db-user", &args.db_user)?;
    validate_shell_safe("db-host", &args.db_host)?;
    validate_shell_safe("postgresql-snap", &args.postgresql_snap)?;
    if let Some(password) = &args.db_password {
        validate_shell_safe("db-password", password)?;
    }

    let password_line = if let Some(password) = args.db_password {
        format!("DB_PASSWORD='{}'", password)
    } else {
        "DB_PASSWORD=".to_string()
    };

    let script = format!(
        r#"#!/bin/bash
set -euo pipefail

DB_NAME='{db_name}'
DB_USER='{db_user}'
DB_HOST='{db_host}'
DB_PORT='{db_port}'
PG_SNAP='{pg_snap}'
{password_line}

if [ "$(id -u)" -eq 0 ]; then
  ROOT_CMD=''
elif command -v sudo >/dev/null 2>&1; then
  ROOT_CMD='sudo'
else
  echo "This script needs root privileges (run as root or install sudo)." >&2
  exit 1
fi

if ! snap list "$PG_SNAP" >/dev/null 2>&1; then
  $ROOT_CMD snap install "$PG_SNAP"
fi

$ROOT_CMD snap start --enable "$PG_SNAP" >/dev/null 2>&1 || true

if command -v postgresql.psql >/dev/null 2>&1; then
  PSQL='postgresql.psql'
else
  PSQL='snap run postgresql.psql'
fi

if [ -z "${{DB_PASSWORD}}" ]; then
  DB_PASSWORD=$(openssl rand -hex 18)
fi

$ROOT_CMD bash -c "${{PSQL}} -v ON_ERROR_STOP=1 -U postgres -d postgres <<'SQL'
DO \$\$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '$DB_USER') THEN
    EXECUTE format('CREATE ROLE %I LOGIN PASSWORD %L', '$DB_USER', '$DB_PASSWORD');
  ELSE
    EXECUTE format('ALTER ROLE %I LOGIN PASSWORD %L', '$DB_USER', '$DB_PASSWORD');
  END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'harvest_graph_reader') THEN
    CREATE ROLE harvest_graph_reader NOLOGIN;
  END IF;
  EXECUTE format('GRANT harvest_graph_reader TO %I', '$DB_USER');
END \$\$;
SQL"

if ! $ROOT_CMD bash -c "${{PSQL}} -U postgres -d postgres -tAc \"SELECT 1 FROM pg_database WHERE datname = '$DB_NAME'\"" | grep -q '^1$'; then
  $ROOT_CMD bash -c "${{PSQL}} -v ON_ERROR_STOP=1 -U postgres -d postgres -c 'CREATE DATABASE \"'$DB_NAME'\" OWNER \"'$DB_USER'\";'"
fi

$ROOT_CMD bash -c "${{PSQL}} -v ON_ERROR_STOP=1 -U postgres -d postgres -c 'GRANT ALL PRIVILEGES ON DATABASE \"'$DB_NAME'\" TO \"'$DB_USER'\";'"

$ROOT_CMD bash -c "${{PSQL}} -v ON_ERROR_STOP=1 -U postgres -d '$DB_NAME' <<'SQL'
GRANT USAGE ON SCHEMA public TO harvest_graph_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA public TO harvest_graph_reader;
ALTER DEFAULT PRIVILEGES FOR ROLE \"$DB_USER\" IN SCHEMA public
  GRANT SELECT ON TABLES TO harvest_graph_reader;
SQL"

DATABASE_URL="postgres://$DB_USER:$DB_PASSWORD@$DB_HOST:$DB_PORT/$DB_NAME"
echo
echo "database.url=$DATABASE_URL"
echo "snap set harvest database.url=$DATABASE_URL"
"#,
        db_name = args.db_name,
        db_user = args.db_user,
        db_host = args.db_host,
        db_port = args.db_port,
        pg_snap = args.postgresql_snap,
        password_line = password_line,
    );

    let mut stdout = std::io::stdout();
    stdout.write_all(script.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

async fn run_status(args: StatusArgs) -> Result<()> {
    require_snapctl()?;

    let service_rows = parse_services_table(&snapctl_services().unwrap_or_default());
    let server_host = non_empty_or(snapctl_get("server.host")?, "127.0.0.1");
    let server_port = non_empty_or(snapctl_get("server.port")?, "8080");
    let ui_host = non_empty_or(snapctl_get("ui.host")?, "127.0.0.1");
    let ui_port = non_empty_or(snapctl_get("ui.port")?, "3000");

    let server_health_url = format!(
        "http://{}:{}/health",
        to_local_probe_host(&server_host),
        server_port
    );
    let ui_health_url = format!(
        "http://{}:{}/ui-health",
        to_local_probe_host(&ui_host),
        ui_port
    );

    let server_health = check_http_ok(&server_health_url).await;
    let ui_health = check_http_ok(&ui_health_url).await;

    let server_state = find_service_state(&service_rows, "harvest.server", "server");
    let ui_state = find_service_state(&service_rows, "harvest.ui", "ui");

    let database_url = snapctl_get("database.url")?;
    let jwt_secret = snapctl_get("auth.jwt-secret")?;
    let config_complete = !database_url.trim().is_empty() && !jwt_secret.trim().is_empty();

    let services = vec![
        service_report(
            "harvest.server",
            server_state,
            server_health,
            server_health_url,
        ),
        service_report("harvest.ui", ui_state, ui_health, ui_health_url),
    ];

    let all_active = services
        .iter()
        .all(|s| s.current.eq_ignore_ascii_case("active"));
    let all_healthy = services.iter().all(|s| s.healthy);
    let overall = if all_active && all_healthy && config_complete {
        "healthy"
    } else if config_complete {
        "degraded"
    } else {
        "incomplete"
    }
    .to_string();

    let report = StatusReport {
        overall,
        config_complete,
        services,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("overall: {}", report.overall);
    println!(
        "config: {}",
        if report.config_complete {
            "complete"
        } else {
            "incomplete"
        }
    );
    println!();
    println!("service         startup  current  health");
    for svc in &report.services {
        println!(
            "{:<14} {:<8} {:<7} {}",
            svc.service,
            svc.startup,
            svc.current,
            if svc.healthy { "ok" } else { "fail" }
        );
        if args.verbose {
            println!("  {}", svc.detail);
        }
    }
    Ok(())
}

fn run_show_public_url(args: ShowPublicUrlArgs) -> Result<()> {
    require_snapctl()?;
    let auth_public = snapctl_get("auth.public-url")?;
    let agents_public = snapctl_get("agents.public-url")?;
    let ui_host = non_empty_or(snapctl_get("ui.host")?, "127.0.0.1");
    let ui_port = non_empty_or(snapctl_get("ui.port")?, "3000");

    let (url, source) = match args.r#for {
        PublicUrlTarget::Auth => {
            let value = auth_public.trim();
            if value.is_empty() {
                bail!("auth.public-url is not set");
            }
            (value.to_string(), "auth.public-url".to_string())
        }
        PublicUrlTarget::Agents => {
            let value = agents_public.trim();
            if value.is_empty() {
                bail!("agents.public-url is not set");
            }
            (value.to_string(), "agents.public-url".to_string())
        }
        PublicUrlTarget::Ui => {
            let host = to_public_host(&ui_host);
            (
                format!("http://{host}:{ui_port}"),
                "derived-from-ui".to_string(),
            )
        }
        PublicUrlTarget::Auto => {
            let auth = auth_public.trim();
            if !auth.is_empty() {
                (auth.to_string(), "auth.public-url".to_string())
            } else {
                let agents = agents_public.trim();
                if !agents.is_empty() {
                    (agents.to_string(), "agents.public-url".to_string())
                } else {
                    let host = to_public_host(&ui_host);
                    (
                        format!("http://{host}:{ui_port}"),
                        "derived-from-ui".to_string(),
                    )
                }
            }
        }
    };

    if args.json {
        let report = PublicUrlReport { url, source };
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("{url}");
    if args.explain {
        println!("source: {source}");
    }
    Ok(())
}

async fn run_llm(args: LlmArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        LlmCommand::Provider(args) => run_llm_provider(args).await,
    }
}

async fn run_llm_provider(args: LlmProviderArgs) -> Result<()> {
    match args.command {
        LlmProviderCommand::List(args) => {
            let providers = load_llm_providers()?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&providers)?);
                return Ok(());
            }
            for (id, cfg) in sort_providers(&providers) {
                let key_mode = if cfg.user_provided_key {
                    "user"
                } else if cfg.api_key.trim().is_empty() {
                    "missing"
                } else {
                    "stored"
                };
                println!(
                    "{}\t{}\tmodel={}\tpriority={}\tenabled={}\tkey={}",
                    id, cfg.provider, cfg.model, cfg.priority, cfg.enabled, key_mode
                );
            }
            Ok(())
        }
        LlmProviderCommand::Show(args) => {
            let providers = load_llm_providers()?;
            let cfg = providers
                .get(&args.id)
                .ok_or_else(|| anyhow!("provider {} not found", args.id))?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(cfg)?);
            } else {
                println!("{}", serde_yaml::to_string(cfg)?);
            }
            Ok(())
        }
        LlmProviderCommand::Add(args) => run_llm_provider_add(args),
        LlmProviderCommand::Update(args) => run_llm_provider_update(args),
        LlmProviderCommand::Remove(args) => run_llm_provider_remove(args),
        LlmProviderCommand::Enable(args) => {
            run_llm_provider_enable_disable(args.id, true, args.apply)
        }
        LlmProviderCommand::Disable(args) => {
            run_llm_provider_enable_disable(args.id, false, args.apply)
        }
        LlmProviderCommand::SetPrimary(args) => run_llm_provider_set_primary(args),
        LlmProviderCommand::Reorder(args) => run_llm_provider_reorder(args),
        LlmProviderCommand::Models(args) => run_llm_provider_models(args).await,
        LlmProviderCommand::Key(args) => run_llm_provider_key(args),
        LlmProviderCommand::DetectModels(args) => run_llm_provider_detect_models(args).await,
    }
}

fn run_llm_provider_add(args: LlmProviderUpsertArgs) -> Result<()> {
    let theme = ColorfulTheme::default();
    let mut providers = load_llm_providers()?;
    let id = match args.id {
        Some(v) => v,
        None => Input::with_theme(&theme)
            .with_prompt("Provider id")
            .interact_text()?,
    };
    validate_identifier("provider id", &id)?;
    if providers.contains_key(&id) {
        bail!("provider {} already exists", id);
    }
    let provider_kind = match args.provider {
        Some(v) => v.to_provider_kind(),
        None => {
            let choices = vec!["anthropic", "gemini", "openai-compatible"];
            let pick = Select::with_theme(&theme)
                .with_prompt("Provider type")
                .items(&choices)
                .default(0)
                .interact()?;
            match pick {
                0 => ProviderKind::Anthropic,
                1 => ProviderKind::Gemini,
                _ => ProviderKind::OpenAiCompatible,
            }
        }
    };
    let model = args
        .model
        .unwrap_or_else(|| provider_kind.default_model().to_string());
    let mut user_provided_key = args.user_provided_key;
    if !user_provided_key && args.api_key.is_none() {
        user_provided_key = Confirm::with_theme(&theme)
            .with_prompt("Use user-provided API keys?")
            .default(false)
            .interact()?;
    }
    let api_key = if user_provided_key {
        String::new()
    } else {
        match args.api_key {
            Some(v) => v,
            None => Password::with_theme(&theme)
                .with_prompt("Provider API key")
                .with_confirmation("Confirm provider API key", "Keys do not match")
                .interact()?,
        }
    };
    let base_url = args
        .base_url
        .unwrap_or_else(|| provider_kind.default_base_url().to_string());
    let cfg = LlmProviderConfig {
        provider: provider_kind.as_snap_value().to_string(),
        api_key,
        user_provided_key,
        model,
        priority: args.priority.unwrap_or(next_priority(&providers)),
        base_url,
        timeout_secs: args.timeout_secs.unwrap_or(120),
        max_retries: args.max_retries.unwrap_or(3),
        display_name: args.display_name.unwrap_or_default(),
        models: args.models.unwrap_or_default(),
        expose_to_ui: args.expose_to_ui.unwrap_or(true),
        enabled: args.enabled.unwrap_or(true),
    };
    providers.insert(id.clone(), cfg.clone());
    validate_llm_providers(&providers)?;
    let settings = llm_provider_to_settings(&id, &cfg);
    mutate_settings(&settings, &args.apply)
}

fn run_llm_provider_update(args: LlmProviderUpdateArgs) -> Result<()> {
    let mut providers = load_llm_providers()?;
    let mut cfg = providers
        .get(&args.id)
        .cloned()
        .ok_or_else(|| anyhow!("provider {} not found", args.id))?;
    if let Some(provider) = args.provider {
        cfg.provider = provider.to_provider_kind().as_snap_value().to_string();
    }
    if let Some(model) = args.model {
        cfg.model = model;
    }
    if let Some(key) = args.api_key {
        cfg.api_key = key;
    }
    if args.clear_api_key {
        cfg.api_key = String::new();
    }
    if let Some(value) = args.user_provided_key {
        cfg.user_provided_key = value;
        if value {
            cfg.api_key.clear();
        }
    }
    if let Some(url) = args.base_url {
        cfg.base_url = url;
    }
    if let Some(name) = args.display_name {
        cfg.display_name = name;
    }
    if let Some(models) = args.models {
        cfg.models = normalize_models_csv(&models);
    }
    if let Some(timeout) = args.timeout_secs {
        cfg.timeout_secs = timeout;
    }
    if let Some(retries) = args.max_retries {
        cfg.max_retries = retries;
    }
    if let Some(priority) = args.priority {
        cfg.priority = priority;
    }
    if let Some(expose) = args.expose_to_ui {
        cfg.expose_to_ui = expose;
    }
    if let Some(enabled) = args.enabled {
        cfg.enabled = enabled;
    }
    providers.insert(args.id.clone(), cfg.clone());
    validate_llm_providers(&providers)?;
    let settings = llm_provider_to_settings(&args.id, &cfg);
    mutate_settings(&settings, &args.apply)
}

fn run_llm_provider_remove(args: LlmProviderRemoveArgs) -> Result<()> {
    let settings = llm_provider_clear_settings(&args.id);
    mutate_settings(&settings, &args.apply)
}

fn run_llm_provider_enable_disable(id: String, enabled: bool, apply: ApplyArgs) -> Result<()> {
    let settings = vec![(format!("llm.{id}.enabled"), enabled.to_string())];
    mutate_settings(&settings, &apply)
}

fn run_llm_provider_set_primary(args: LlmProviderSetPrimaryArgs) -> Result<()> {
    let mut providers = load_llm_providers()?;
    if !providers.contains_key(&args.id) {
        bail!("provider {} not found", args.id);
    }
    let mut ordered = sort_providers(&providers);
    ordered.retain(|(id, _)| id != &args.id);
    ordered.insert(
        0,
        (
            args.id.clone(),
            providers.get(&args.id).cloned().unwrap_or_default(),
        ),
    );
    let mut settings = Vec::new();
    for (idx, (id, cfg)) in ordered.into_iter().enumerate() {
        let mut cfg = cfg;
        cfg.priority = idx as i32;
        providers.insert(id.clone(), cfg.clone());
        settings.extend(llm_provider_to_settings(&id, &cfg));
    }
    validate_llm_providers(&providers)?;
    mutate_settings(&settings, &args.apply)
}

fn run_llm_provider_reorder(args: LlmProviderReorderArgs) -> Result<()> {
    let mut providers = load_llm_providers()?;
    let ids = provider_order_input(&providers, args.order)?;
    if ids.len() != providers.len() {
        bail!("provider reorder must include every provider exactly once");
    }
    let mut settings = Vec::new();
    for (idx, id) in ids.into_iter().enumerate() {
        let mut cfg = providers
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow!("provider {} not found", id))?;
        cfg.priority = idx as i32;
        providers.insert(id.clone(), cfg.clone());
        settings.extend(llm_provider_to_settings(&id, &cfg));
    }
    validate_llm_providers(&providers)?;
    mutate_settings(&settings, &args.apply)
}

async fn run_llm_provider_models(args: LlmProviderModelsArgs) -> Result<()> {
    match args.command {
        LlmProviderModelsCommand::Set(set) => {
            let csv = normalize_models_csv(&set.models);
            let settings = vec![(format!("llm.{}.models", args.id), csv)];
            mutate_settings(&settings, &set.apply)
        }
        LlmProviderModelsCommand::Edit(edit) => {
            let providers = load_llm_providers()?;
            let cfg = providers
                .get(&args.id)
                .ok_or_else(|| anyhow!("provider {} not found", args.id))?;
            let kind = provider_kind_from_string(&cfg.provider)?;
            let detected = if cfg.user_provided_key && cfg.api_key.trim().is_empty() {
                load_model_cache(kind, &cfg.base_url)?.unwrap_or_default()
            } else {
                let live = detect_models(kind, &cfg.base_url, &cfg.api_key)
                    .await
                    .unwrap_or_default();
                if !live.is_empty() {
                    let _ = store_model_cache(kind, &cfg.base_url, &live);
                }
                if live.is_empty() {
                    load_model_cache(kind, &cfg.base_url)?.unwrap_or_default()
                } else {
                    live
                }
            };
            if detected.is_empty() {
                bail!("no detected models available to edit");
            }
            let selected =
                select_models_allowlist(&ColorfulTheme::default(), &detected, &cfg.model)?
                    .unwrap_or_else(|| vec![cfg.model.clone()]);
            let settings = vec![(format!("llm.{}.models", args.id), selected.join(","))];
            mutate_settings(&settings, &edit.apply)
        }
        LlmProviderModelsCommand::Clear(clear) => {
            let settings = vec![(format!("llm.{}.models", args.id), String::new())];
            mutate_settings(&settings, &clear.apply)
        }
    }
}

fn run_llm_provider_key(args: LlmProviderKeyArgs) -> Result<()> {
    match args.command {
        LlmProviderKeyCommand::Set(set) => {
            let settings = vec![
                (format!("llm.{}.api-key", args.id), set.key),
                (
                    format!("llm.{}.user-provided-key", args.id),
                    "false".to_string(),
                ),
            ];
            mutate_settings(&settings, &set.apply)
        }
        LlmProviderKeyCommand::Clear(clear) => {
            let settings = vec![(format!("llm.{}.api-key", args.id), String::new())];
            mutate_settings(&settings, &clear.apply)
        }
        LlmProviderKeyCommand::UserProvided(v) => {
            let mut settings = vec![(
                format!("llm.{}.user-provided-key", args.id),
                v.value.to_string(),
            )];
            if v.value {
                settings.push((format!("llm.{}.api-key", args.id), String::new()));
            }
            mutate_settings(&settings, &v.apply)
        }
    }
}

async fn run_llm_provider_detect_models(args: LlmProviderDetectModelsArgs) -> Result<()> {
    let providers = load_llm_providers()?;
    let cfg = providers
        .get(&args.id)
        .cloned()
        .ok_or_else(|| anyhow!("provider {} not found", args.id))?;
    let kind = provider_kind_from_string(&cfg.provider)?;
    let models = if cfg.user_provided_key && cfg.api_key.trim().is_empty() {
        load_model_cache(kind, &cfg.base_url)?.unwrap_or_default()
    } else {
        let live = detect_models(kind, &cfg.base_url, &cfg.api_key)
            .await
            .unwrap_or_default();
        if !live.is_empty() {
            let _ = store_model_cache(kind, &cfg.base_url, &live);
            live
        } else {
            load_model_cache(kind, &cfg.base_url)?.unwrap_or_default()
        }
    };
    if models.is_empty() {
        println!("no models found");
        return Ok(());
    }
    println!("{}", serde_json::to_string_pretty(&models)?);
    let mut settings = Vec::new();
    if args.set_primary {
        settings.push((format!("llm.{}.model", args.id), models[0].clone()));
    }
    if args.set_allowlist {
        settings.push((format!("llm.{}.models", args.id), models.join(",")));
    }
    if settings.is_empty() {
        return Ok(());
    }
    mutate_settings(&settings, &args.apply)
}

fn run_identity(args: IdentityArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        IdentityCommand::Provider(args) => run_identity_provider(args),
    }
}

fn run_identity_provider(args: IdentityProviderArgs) -> Result<()> {
    match args.command {
        IdentityProviderCommand::List(json) => {
            let google = identity_google_state()?;
            let oidc = identity_oidc_state()?;
            let payload = serde_json::json!({"google": google, "oidc": oidc});
            if json.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("google\tenabled={}", payload["google"]["enabled"]);
                println!("oidc\tenabled={}", payload["oidc"]["enabled"]);
            }
            Ok(())
        }
        IdentityProviderCommand::Show(args) => {
            let payload = match args.provider {
                IdentityProviderKind::Google => identity_google_state()?,
                IdentityProviderKind::Oidc => identity_oidc_state()?,
            };
            if args.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", serde_yaml::to_string(&payload)?);
            }
            Ok(())
        }
        IdentityProviderCommand::Enable(args) => run_identity_provider_update(args, true),
        IdentityProviderCommand::Disable(args) => run_identity_provider_disable(args),
        IdentityProviderCommand::Update(args) => run_identity_provider_update(args, false),
    }
}

fn run_identity_provider_disable(args: IdentityProviderDisableArgs) -> Result<()> {
    let settings = match args.provider {
        IdentityProviderKind::Google => vec![
            ("auth.google.client-id".to_string(), String::new()),
            ("auth.google.client-secret".to_string(), String::new()),
            ("auth.google.redirect-uri".to_string(), String::new()),
        ],
        IdentityProviderKind::Oidc => vec![
            ("auth.oidc.issuer-url".to_string(), String::new()),
            ("auth.oidc.client-id".to_string(), String::new()),
            ("auth.oidc.client-secret".to_string(), String::new()),
            ("auth.oidc.redirect-uri".to_string(), String::new()),
            ("auth.oidc.display-name".to_string(), String::new()),
        ],
    };
    mutate_settings(&settings, &args.apply)
}

fn run_identity_provider_update(
    args: IdentityProviderUpdateArgs,
    force_validate: bool,
) -> Result<()> {
    let mut settings = Vec::new();
    match args.provider {
        IdentityProviderKind::Google => {
            let client_id = args
                .client_id
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.google.client-id").unwrap_or_default());
            let client_secret = args
                .client_secret
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.google.client-secret").unwrap_or_default());
            let redirect_uri = args
                .redirect_uri
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.google.redirect-uri").unwrap_or_default());
            if let Some(v) = args.client_id {
                settings.push(("auth.google.client-id".to_string(), v));
            }
            if let Some(v) = args.client_secret {
                settings.push(("auth.google.client-secret".to_string(), v));
            }
            if let Some(v) = args.redirect_uri {
                settings.push(("auth.google.redirect-uri".to_string(), v));
            }
            if force_validate {
                if client_id.trim().is_empty()
                    || client_secret.trim().is_empty()
                    || redirect_uri.trim().is_empty()
                {
                    bail!("google requires client-id, client-secret, redirect-uri");
                }
            }
        }
        IdentityProviderKind::Oidc => {
            let issuer_url = args
                .issuer_url
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.oidc.issuer-url").unwrap_or_default());
            let client_id = args
                .client_id
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.oidc.client-id").unwrap_or_default());
            let client_secret = args
                .client_secret
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.oidc.client-secret").unwrap_or_default());
            let redirect_uri = args
                .redirect_uri
                .clone()
                .unwrap_or_else(|| snapctl_get("auth.oidc.redirect-uri").unwrap_or_default());
            if let Some(v) = args.issuer_url {
                settings.push(("auth.oidc.issuer-url".to_string(), v));
            }
            if let Some(v) = args.client_id {
                settings.push(("auth.oidc.client-id".to_string(), v));
            }
            if let Some(v) = args.client_secret {
                settings.push(("auth.oidc.client-secret".to_string(), v));
            }
            if let Some(v) = args.redirect_uri {
                settings.push(("auth.oidc.redirect-uri".to_string(), v));
            }
            if let Some(v) = args.display_name {
                settings.push(("auth.oidc.display-name".to_string(), v));
            }
            if force_validate {
                if issuer_url.trim().is_empty()
                    || client_id.trim().is_empty()
                    || client_secret.trim().is_empty()
                    || redirect_uri.trim().is_empty()
                {
                    bail!("oidc requires issuer-url, client-id, client-secret, redirect-uri");
                }
            }
        }
    }
    if settings.is_empty() {
        return Ok(());
    }
    mutate_settings(&settings, &args.apply)
}

fn run_auth(args: AuthArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        AuthCommand::LocalLogin(args) => match args.command {
            AuthLocalLoginCommand::Get(json) => {
                let value = snapctl_get("auth.allow-local-login")?;
                if json.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({"value": value}))?
                    );
                } else {
                    println!("{}", non_empty_or(value, "true"));
                }
                Ok(())
            }
            AuthLocalLoginCommand::Set(set) => mutate_settings(
                &[("auth.allow-local-login".to_string(), set.value.to_string())],
                &set.apply,
            ),
        },
        AuthCommand::Jwt(args) => match args.command {
            AuthJwtCommand::Rotate(rotate) => {
                let value = if let Some(v) = rotate.value {
                    v
                } else if rotate.auto || rotate.value.is_none() {
                    generate_secret(64)
                } else {
                    bail!("provide --auto or --value");
                };
                mutate_settings(&[("auth.jwt-secret".to_string(), value)], &rotate.apply)
            }
        },
        AuthCommand::PublicUrl(args) => match args.command {
            AuthPublicUrlCommand::Get(json) => {
                let value = snapctl_get("auth.public-url")?;
                if json.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({"value": value}))?
                    );
                } else {
                    println!("{value}");
                }
                Ok(())
            }
            AuthPublicUrlCommand::Set(set) => {
                mutate_settings(&[("auth.public-url".to_string(), set.value)], &set.apply)
            }
        },
    }
}

fn run_security(args: SecurityArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        SecurityCommand::UserKeyEncryptionKey(args) => match args.command {
            SecurityUserKeyCommand::Set(set) => {
                validate_user_key_encryption_key(&set.value)?;
                mutate_settings(
                    &[("security.user-key-encryption-key".to_string(), set.value)],
                    &set.apply,
                )
            }
            SecurityUserKeyCommand::Rotate(rotate) => mutate_settings(
                &[(
                    "security.user-key-encryption-key".to_string(),
                    generate_hex_secret(64),
                )],
                &rotate.apply,
            ),
            SecurityUserKeyCommand::Clear(clear) => {
                let providers = load_llm_providers().unwrap_or_default();
                if providers.values().any(|p| p.user_provided_key) {
                    bail!("cannot clear encryption key while user-provided-key providers exist");
                }
                mutate_settings(
                    &[(
                        "security.user-key-encryption-key".to_string(),
                        String::new(),
                    )],
                    &clear.apply,
                )
            }
        },
    }
}

fn run_database(args: DatabaseArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        DatabaseCommand::Get(json) => {
            let payload = serde_json::json!({
                "url": snapctl_get("database.url")?,
                "pool_size": non_empty_or(snapctl_get("database.pool-size")?, "16"),
                "ca_file": snapctl_get("database.ca-file")?,
            });
            if json.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", serde_yaml::to_string(&payload)?);
            }
            Ok(())
        }
        DatabaseCommand::SetUrl(set) => {
            validate_database_url(&set.value)?;
            mutate_settings(&[("database.url".to_string(), set.value)], &set.apply)
        }
        DatabaseCommand::SetPoolSize(set) => mutate_settings(
            &[("database.pool-size".to_string(), set.value.to_string())],
            &set.apply,
        ),
        DatabaseCommand::SetCaFile(set) => {
            mutate_settings(&[("database.ca-file".to_string(), set.value)], &set.apply)
        }
    }
}

fn run_server(args: ServerArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        ServerCommand::Get(json) => {
            let payload = serde_json::json!({
                "host": non_empty_or(snapctl_get("server.host")?, "127.0.0.1"),
                "port": non_empty_or(snapctl_get("server.port")?, "8080"),
            });
            if json.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", serde_yaml::to_string(&payload)?);
            }
            Ok(())
        }
        ServerCommand::SetHost(set) => {
            mutate_settings(&[("server.host".to_string(), set.value)], &set.apply)
        }
        ServerCommand::SetPort(set) => mutate_settings(
            &[("server.port".to_string(), set.value.to_string())],
            &set.apply,
        ),
    }
}

fn run_ui(args: UiArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        UiCommand::Get(json) => {
            let payload = serde_json::json!({
                "host": non_empty_or(snapctl_get("ui.host")?, "127.0.0.1"),
                "port": non_empty_or(snapctl_get("ui.port")?, "3000"),
                "mode": non_empty_or(snapctl_get("ui.mode")?, "proxy"),
                "enable_docs": non_empty_or(snapctl_get("ui.enable-docs")?, "false"),
            });
            if json.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", serde_yaml::to_string(&payload)?);
            }
            Ok(())
        }
        UiCommand::SetHost(set) => {
            mutate_settings(&[("ui.host".to_string(), set.value)], &set.apply)
        }
        UiCommand::SetPort(set) => mutate_settings(
            &[("ui.port".to_string(), set.value.to_string())],
            &set.apply,
        ),
        UiCommand::SetMode(set) => {
            mutate_settings(&[("ui.mode".to_string(), set.value)], &set.apply)
        }
        UiCommand::EnableDocs(set) => mutate_settings(
            &[("ui.enable-docs".to_string(), set.value.to_string())],
            &set.apply,
        ),
    }
}

fn run_agents(args: AgentsArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        AgentsCommand::Get(json) => {
            let payload = serde_json::json!({"public_url": snapctl_get("agents.public-url")?});
            if json.json {
                println!("{}", serde_json::to_string_pretty(&payload)?);
            } else {
                println!("{}", serde_yaml::to_string(&payload)?);
            }
            Ok(())
        }
        AgentsCommand::SetPublicUrl(set) => {
            mutate_settings(&[("agents.public-url".to_string(), set.value)], &set.apply)
        }
    }
}

fn run_config(args: ConfigArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        ConfigCommand::Get(args) => {
            let value = snapctl_get(&args.key)?;
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({args.key: value}))?
                );
            } else {
                println!("{value}");
            }
            Ok(())
        }
        ConfigCommand::Set(args) => {
            let mut settings = Vec::new();
            for pair in args.entries {
                let (k, v) = pair
                    .split_once('=')
                    .ok_or_else(|| anyhow!("entry must be key=value: {}", pair))?;
                settings.push((k.to_string(), v.to_string()));
            }
            mutate_settings(&settings, &args.apply)
        }
        ConfigCommand::Unset(args) => {
            let settings = args
                .keys
                .into_iter()
                .map(|k| (k, String::new()))
                .collect::<Vec<_>>();
            mutate_settings(&settings, &args.apply)
        }
        ConfigCommand::List(args) => {
            let map = load_snap_map(args.prefix.as_deref())?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&map)?);
            } else {
                for (k, v) in map {
                    println!("{k}={v}");
                }
            }
            Ok(())
        }
        ConfigCommand::Export(args) => {
            let map = load_snap_map(None)?;
            match args.format {
                ExportFormat::Json => println!("{}", serde_json::to_string_pretty(&map)?),
                ExportFormat::Yaml => println!("{}", serde_yaml::to_string(&map)?),
            }
            Ok(())
        }
        ConfigCommand::Import(args) => {
            let content = std::fs::read_to_string(&args.file)?;
            let map: HashMap<String, String> =
                if args.file.extension().and_then(|e| e.to_str()) == Some("json") {
                    serde_json::from_str(&content)?
                } else {
                    serde_yaml::from_str(&content)?
                };
            let settings = map.into_iter().collect::<Vec<_>>();
            if args.apply.dry_run {
                for (k, v) in settings {
                    println!("{k}={v}");
                }
                return Ok(());
            }
            mutate_settings(&settings, &args.apply)
        }
        ConfigCommand::Diff(args) => {
            let content = std::fs::read_to_string(&args.file)?;
            let desired: HashMap<String, String> =
                if args.file.extension().and_then(|e| e.to_str()) == Some("json") {
                    serde_json::from_str(&content)?
                } else {
                    serde_yaml::from_str(&content)?
                };
            let current = load_snap_map(None)?;
            let mut keys = BTreeSet::new();
            for k in desired.keys() {
                keys.insert(k.clone());
            }
            for k in current.keys() {
                keys.insert(k.clone());
            }
            for key in keys {
                let a = current.get(&key).cloned().unwrap_or_default();
                let b = desired.get(&key).cloned().unwrap_or_default();
                if a != b {
                    println!("{}\n  current: {}\n  desired: {}", key, a, b);
                }
            }
            Ok(())
        }
        ConfigCommand::Apply(args) => {
            run_configure_hook()?;
            if !args.no_restart {
                let _ = run_snapctl(["restart", "harvest.server"]);
                let _ = run_snapctl(["restart", "harvest.ui"]);
                let _ = run_snapctl(["start", "--enable", "harvest.server"]);
                let _ = run_snapctl(["start", "--enable", "harvest.ui"]);
            }
            Ok(())
        }
        ConfigCommand::Validate(args) => {
            let providers = load_llm_providers().unwrap_or_default();
            let mut errors = validate_llm_providers_collect(&providers);
            if providers.values().any(|p| p.user_provided_key) {
                let key = snapctl_get("security.user-key-encryption-key").unwrap_or_default();
                if key.trim().is_empty() {
                    errors.push("security.user-key-encryption-key is required when any provider uses user-provided-key=true".to_string());
                } else if let Err(e) = validate_user_key_encryption_key(&key) {
                    errors.push(e.to_string());
                }
            }
            let report = ValidationReport {
                ok: errors.is_empty(),
                errors,
            };
            if args.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if report.ok {
                println!("ok");
            } else {
                for e in &report.errors {
                    println!("error: {e}");
                }
            }
            if report.ok {
                Ok(())
            } else {
                bail!("validation failed")
            }
        }
    }
}

fn mutate_settings(settings: &[(String, String)], apply: &ApplyArgs) -> Result<()> {
    enforce_user_key_encryption_guard(settings)?;
    if apply.dry_run {
        for (k, v) in settings {
            if k.contains("secret") || k.contains("api-key") {
                println!("{k}=<redacted>");
            } else {
                println!("{k}={v}");
            }
        }
        return Ok(());
    }
    if !running_as_root() {
        bail!("this operation requires root inside the snap environment");
    }
    apply_snap_settings(settings)?;
    if apply.apply_now {
        run_configure_hook()?;
        if !apply.no_restart {
            let _ = run_snapctl(["restart", "harvest.server"]);
            let _ = run_snapctl(["restart", "harvest.ui"]);
            let _ = run_snapctl(["start", "--enable", "harvest.server"]);
            let _ = run_snapctl(["start", "--enable", "harvest.ui"]);
        }
    }
    Ok(())
}

fn enforce_user_key_encryption_guard(pending: &[(String, String)]) -> Result<()> {
    let mut providers = load_llm_providers().unwrap_or_default();
    for (k, v) in pending {
        if let Some(rest) = k.strip_prefix("llm.") {
            if let Some((id, field)) = rest.split_once('.') {
                let cfg = providers.entry(id.to_string()).or_default();
                match field {
                    "user-provided-key" => {
                        cfg.user_provided_key = v.trim().eq_ignore_ascii_case("true") || v == "1";
                    }
                    "api-key" => cfg.api_key = v.clone(),
                    _ => {}
                }
            }
        }
    }
    let mut encryption_key = snapctl_get("security.user-key-encryption-key").unwrap_or_default();
    for (k, v) in pending {
        if k == "security.user-key-encryption-key" {
            encryption_key = v.clone();
        }
    }
    let uses_user_keys = providers.values().any(|p| p.user_provided_key);
    if uses_user_keys && encryption_key.trim().is_empty() {
        bail!(
            "security.user-key-encryption-key is required when any provider has user-provided-key=true"
        );
    }
    if uses_user_keys {
        validate_user_key_encryption_key(&encryption_key)?;
    }
    Ok(())
}

fn load_llm_providers() -> Result<HashMap<String, LlmProviderConfig>> {
    let raw = snapctl_get_json("llm")?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({"llm":{}}));
    let node = value
        .get("llm")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let mut out: HashMap<String, LlmProviderConfig> =
        serde_json::from_value(node).unwrap_or_default();
    for cfg in out.values_mut() {
        if cfg.timeout_secs == 0 {
            cfg.timeout_secs = 120;
        }
        if cfg.max_retries == 0 {
            cfg.max_retries = 3;
        }
        if cfg.provider.is_empty() {
            cfg.provider = "anthropic".to_string();
        }
    }
    Ok(out)
}

fn sort_providers(
    providers: &HashMap<String, LlmProviderConfig>,
) -> Vec<(String, LlmProviderConfig)> {
    let mut items: Vec<(String, LlmProviderConfig)> = providers
        .iter()
        .map(|(id, cfg)| (id.clone(), cfg.clone()))
        .collect();
    items.sort_by(|a, b| a.1.priority.cmp(&b.1.priority).then(a.0.cmp(&b.0)));
    items
}

fn next_priority(providers: &HashMap<String, LlmProviderConfig>) -> i32 {
    providers.values().map(|p| p.priority).max().unwrap_or(-1) + 1
}

fn llm_provider_to_settings(id: &str, cfg: &LlmProviderConfig) -> Vec<(String, String)> {
    vec![
        (format!("llm.{id}.provider"), cfg.provider.clone()),
        (format!("llm.{id}.api-key"), cfg.api_key.clone()),
        (
            format!("llm.{id}.user-provided-key"),
            cfg.user_provided_key.to_string(),
        ),
        (format!("llm.{id}.model"), cfg.model.clone()),
        (format!("llm.{id}.priority"), cfg.priority.to_string()),
        (format!("llm.{id}.base-url"), cfg.base_url.clone()),
        (
            format!("llm.{id}.timeout-secs"),
            cfg.timeout_secs.to_string(),
        ),
        (format!("llm.{id}.max-retries"), cfg.max_retries.to_string()),
        (format!("llm.{id}.display-name"), cfg.display_name.clone()),
        (
            format!("llm.{id}.models"),
            normalize_models_csv(&cfg.models),
        ),
        (
            format!("llm.{id}.expose-to-ui"),
            cfg.expose_to_ui.to_string(),
        ),
        (format!("llm.{id}.enabled"), cfg.enabled.to_string()),
    ]
}

fn llm_provider_clear_settings(id: &str) -> Vec<(String, String)> {
    vec![
        (format!("llm.{id}.provider"), String::new()),
        (format!("llm.{id}.api-key"), String::new()),
        (format!("llm.{id}.user-provided-key"), String::new()),
        (format!("llm.{id}.model"), String::new()),
        (format!("llm.{id}.priority"), String::new()),
        (format!("llm.{id}.base-url"), String::new()),
        (format!("llm.{id}.timeout-secs"), String::new()),
        (format!("llm.{id}.max-retries"), String::new()),
        (format!("llm.{id}.display-name"), String::new()),
        (format!("llm.{id}.models"), String::new()),
        (format!("llm.{id}.expose-to-ui"), String::new()),
        (format!("llm.{id}.enabled"), String::new()),
    ]
}

fn validate_llm_providers(providers: &HashMap<String, LlmProviderConfig>) -> Result<()> {
    let errors = validate_llm_providers_collect(providers);
    if errors.is_empty() {
        Ok(())
    } else {
        bail!(errors.join("; "))
    }
}

fn validate_llm_providers_collect(providers: &HashMap<String, LlmProviderConfig>) -> Vec<String> {
    let mut errors = Vec::new();
    for (id, cfg) in providers {
        if cfg.provider.trim().is_empty() {
            errors.push(format!("llm.{id}.provider is required"));
        }
        if !cfg.user_provided_key && cfg.api_key.trim().is_empty() {
            errors.push(format!(
                "llm.{id}.api-key is required when user-provided-key=false"
            ));
        }
        if cfg.model.trim().is_empty() {
            errors.push(format!("llm.{id}.model is required"));
        }
        let allow = parse_models_csv(&cfg.models);
        if !allow.is_empty() && !allow.iter().any(|m| m == &cfg.model) {
            errors.push(format!("llm.{id}.models must include llm.{id}.model"));
        }
    }
    errors
}

fn parse_models_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize_models_csv(raw: &str) -> String {
    parse_models_csv(raw).join(",")
}

fn provider_kind_from_string(value: &str) -> Result<ProviderKind> {
    match value {
        "anthropic" => Ok(ProviderKind::Anthropic),
        "gemini" => Ok(ProviderKind::Gemini),
        "openai-compatible" => Ok(ProviderKind::OpenAiCompatible),
        _ => bail!("unsupported provider kind {}", value),
    }
}

fn provider_order_input(
    providers: &HashMap<String, LlmProviderConfig>,
    order: Option<String>,
) -> Result<Vec<String>> {
    if let Some(value) = order {
        let parsed = value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        return Ok(parsed);
    }
    let default = sort_providers(providers)
        .into_iter()
        .map(|(id, _)| id)
        .collect::<Vec<_>>()
        .join(",");
    let value: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("Provider order (comma-separated ids, first is primary)")
        .default(default)
        .interact_text()?;
    Ok(value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn identity_google_state() -> Result<serde_json::Value> {
    let client_id = snapctl_get("auth.google.client-id")?;
    let client_secret = snapctl_get("auth.google.client-secret")?;
    let redirect_uri = snapctl_get("auth.google.redirect-uri")?;
    Ok(serde_json::json!({
        "enabled": !client_id.is_empty() && !client_secret.is_empty() && !redirect_uri.is_empty(),
        "client_id": client_id,
        "client_secret": if client_secret.is_empty() { "" } else { "<redacted>" },
        "redirect_uri": redirect_uri,
    }))
}

fn identity_oidc_state() -> Result<serde_json::Value> {
    let issuer_url = snapctl_get("auth.oidc.issuer-url")?;
    let client_id = snapctl_get("auth.oidc.client-id")?;
    let client_secret = snapctl_get("auth.oidc.client-secret")?;
    let redirect_uri = snapctl_get("auth.oidc.redirect-uri")?;
    let display_name = snapctl_get("auth.oidc.display-name")?;
    Ok(serde_json::json!({
        "enabled": !issuer_url.is_empty() && !client_id.is_empty() && !client_secret.is_empty() && !redirect_uri.is_empty(),
        "issuer_url": issuer_url,
        "client_id": client_id,
        "client_secret": if client_secret.is_empty() { "" } else { "<redacted>" },
        "redirect_uri": redirect_uri,
        "display_name": display_name,
    }))
}

fn load_snap_map(prefix: Option<&str>) -> Result<HashMap<String, String>> {
    if let Some(p) = prefix {
        return read_prefix_map(p);
    }
    let roots = [
        "database", "server", "ui", "auth", "security", "agent", "agents", "cluster", "llm", "lxd",
        "docs", "log", "config",
    ];
    let mut merged = HashMap::new();
    for root in roots {
        if let Ok(map) = read_prefix_map(root) {
            merged.extend(map);
        }
    }
    Ok(merged)
}

fn read_prefix_map(prefix: &str) -> Result<HashMap<String, String>> {
    let raw = snapctl_get_json(prefix)?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|_| serde_json::json!({}));
    let node = value
        .get(prefix)
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let mut map = HashMap::new();
    flatten_json(prefix, &node, &mut map);
    Ok(map)
}

fn flatten_json(prefix: &str, value: &serde_json::Value, out: &mut HashMap<String, String>) {
    match value {
        serde_json::Value::Object(obj) => {
            for (k, v) in obj {
                let key = format!("{prefix}.{k}");
                flatten_json(&key, v, out);
            }
        }
        serde_json::Value::Null => {
            out.insert(prefix.to_string(), String::new());
        }
        serde_json::Value::Bool(v) => {
            out.insert(prefix.to_string(), v.to_string());
        }
        serde_json::Value::Number(v) => {
            out.insert(prefix.to_string(), v.to_string());
        }
        serde_json::Value::String(v) => {
            out.insert(prefix.to_string(), v.to_string());
        }
        serde_json::Value::Array(v) => {
            let joined = v
                .iter()
                .filter_map(|x| x.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
                .join(",");
            out.insert(prefix.to_string(), joined);
        }
    }
}

async fn run_cluster(args: ClusterArgs) -> Result<()> {
    require_snapctl()?;
    match args.command {
        ClusterCommand::Backend(args) => run_cluster_backend(args),
        ClusterCommand::Status(args) => run_cluster_status(args).await,
        ClusterCommand::List(args) => run_cluster_list(args),
        ClusterCommand::Token(args) => run_cluster_token(args),
        ClusterCommand::Init(args) => run_cluster_init(args),
        ClusterCommand::Join(args) => run_cluster_join(args),
        ClusterCommand::Preseed(args) => run_cluster_preseed(args),
        ClusterCommand::Remove(args) => run_cluster_remove(args),
        ClusterCommand::Evacuate(args) => run_cluster_evacuate(args),
        ClusterCommand::Restore(args) => run_cluster_restore(args),
        ClusterCommand::RotateSecrets => run_cluster_rotate_secrets(),
    }
}

fn is_cluster_managed() -> Result<bool> {
    Ok(snapctl_get("config.managed")?
        .trim()
        .eq_ignore_ascii_case("true"))
}

fn ensure_cluster_mutable() -> Result<()> {
    if is_cluster_managed()? {
        bail!(
            "cluster is charm-managed. Use juju add-unit/remove-unit/actions for cluster changes"
        );
    }
    Ok(())
}

async fn run_cluster_status(args: ClusterStatusArgs) -> Result<()> {
    let managed = is_cluster_managed()?;
    let backend = current_cluster_backend()?.as_snap_value().to_string();
    let node_name = snapctl_get("cluster.node-name")?;
    let internal_listen = non_empty_or(snapctl_get("cluster.internal-listen")?, "0.0.0.0:8081");
    let internal_url = snapctl_get("cluster.internal-url")?;
    let server_host = non_empty_or(snapctl_get("server.host")?, "127.0.0.1");
    let server_port = non_empty_or(snapctl_get("server.port")?, "8080");
    let host = to_local_probe_host(&server_host);
    let healthy = check_http_ok(&format!("http://{host}:{server_port}/health/ready"))
        .await
        .unwrap_or(false);
    let members = parse_cluster_members(&snapctl_get("cluster.members")?);
    let report = ClusterStatusReport {
        managed,
        backend,
        node_name,
        internal_listen,
        internal_url,
        healthy,
        members,
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!(
        "managed={} backend={} healthy={} node={} internal_url={}",
        report.managed, report.backend, report.healthy, report.node_name, report.internal_url
    );
    Ok(())
}

fn run_cluster_backend(args: ClusterBackendArgs) -> Result<()> {
    match args.command {
        ClusterBackendCommand::Get(json) => {
            let value = current_cluster_backend()?.as_snap_value().to_string();
            if json.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({"value": value}))?
                );
            } else {
                println!("{value}");
            }
            Ok(())
        }
        ClusterBackendCommand::Set(set) => mutate_settings(
            &[(
                "cluster.backend".to_string(),
                set.value.as_snap_value().to_string(),
            )],
            &set.apply,
        ),
    }
}

fn run_cluster_list(args: ClusterListArgs) -> Result<()> {
    let members = parse_cluster_members(&snapctl_get("cluster.members")?);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&members)?);
        return Ok(());
    }
    for member in members {
        println!("{}\t{}", member.name, member.url);
    }
    Ok(())
}

fn run_cluster_token(args: ClusterTokenArgs) -> Result<()> {
    match args.command {
        ClusterTokenCommand::Create(args) => run_cluster_token_create(args),
        ClusterTokenCommand::List(args) => run_cluster_token_list(args),
        ClusterTokenCommand::Revoke => run_cluster_token_revoke(),
    }
}

fn run_cluster_token_create(args: ClusterTokenCreateArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let token = generate_secret(48);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let expires_unix = now.saturating_add(args.ttl_secs.max(1));
    apply_snap_settings(&[
        ("cluster.join-token".to_string(), token.clone()),
        (
            "cluster.join-token-expiry".to_string(),
            expires_unix.to_string(),
        ),
    ])?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ClusterTokenReport {
                token,
                expires_unix,
            })?
        );
        return Ok(());
    }
    println!("{token}");
    Ok(())
}

fn run_cluster_token_list(args: ClusterTokenListArgs) -> Result<()> {
    let token = snapctl_get("cluster.join-token")?;
    let expiry = snapctl_get("cluster.join-token-expiry")?
        .parse::<u64>()
        .unwrap_or(0);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&ClusterTokenReport {
                token,
                expires_unix: expiry,
            })?
        );
        return Ok(());
    }
    if token.is_empty() {
        println!("no token");
        return Ok(());
    }
    println!("{token}\texpires_unix={expiry}");
    Ok(())
}

fn run_cluster_token_revoke() -> Result<()> {
    ensure_cluster_mutable()?;
    apply_snap_settings(&[
        ("cluster.join-token".to_string(), String::new()),
        ("cluster.join-token-expiry".to_string(), "0".to_string()),
    ])?;
    Ok(())
}

fn run_cluster_init(args: ClusterInitArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let node_name = cluster_node_name(args.node_name)?;
    let internal_url = cluster_internal_url(args.internal_url)?;
    let shared_secret = args.shared_secret.unwrap_or_else(|| generate_secret(48));
    let backend = resolve_cluster_backend(args.backend)?;
    let mut settings = vec![
        ("cluster.node-name".to_string(), node_name.clone()),
        (
            "cluster.internal-listen".to_string(),
            args.internal_listen.clone(),
        ),
        ("cluster.internal-url".to_string(), internal_url.clone()),
        ("cluster.shared-secret".to_string(), shared_secret),
        (
            "cluster.backend".to_string(),
            backend.as_snap_value().to_string(),
        ),
        (
            "cluster.members".to_string(),
            format_cluster_members(&[ClusterMember {
                name: node_name,
                url: internal_url,
            }]),
        ),
    ];
    settings.sort_by(|a, b| a.0.cmp(&b.0));
    if args.dry_run {
        for (k, v) in settings {
            println!("{k}={v}");
        }
        return Ok(());
    }
    apply_snap_settings(&settings)?;
    run_configure_hook()?;
    Ok(())
}

fn run_cluster_join(args: ClusterJoinArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let token = args.token.unwrap_or_default();
    let stored_token = snapctl_get("cluster.join-token")?;
    let expiry = snapctl_get("cluster.join-token-expiry")?
        .parse::<u64>()
        .unwrap_or(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if !stored_token.is_empty() {
        if token != stored_token {
            bail!("join token does not match cluster token");
        }
        if expiry > 0 && now > expiry {
            bail!("join token has expired");
        }
    }
    let node_name = cluster_node_name(args.node_name)?;
    let internal_url = cluster_internal_url(args.internal_url)?;
    let backend = resolve_cluster_backend(args.backend)?;
    let secret = match args.shared_secret {
        Some(value) => value,
        None => snapctl_get("cluster.shared-secret")?,
    };
    if secret.trim().is_empty() {
        bail!("cluster.shared-secret is required to join");
    }
    let mut members = parse_cluster_members(&snapctl_get("cluster.members")?);
    upsert_cluster_member(
        &mut members,
        ClusterMember {
            name: node_name.clone(),
            url: internal_url.clone(),
        },
    );
    let settings = vec![
        ("cluster.node-name".to_string(), node_name),
        (
            "cluster.internal-listen".to_string(),
            args.internal_listen.clone(),
        ),
        ("cluster.internal-url".to_string(), internal_url),
        ("cluster.shared-secret".to_string(), secret),
        (
            "cluster.backend".to_string(),
            backend.as_snap_value().to_string(),
        ),
        (
            "cluster.members".to_string(),
            format_cluster_members(&members),
        ),
    ];
    if args.dry_run {
        for (k, v) in settings {
            println!("{k}={v}");
        }
        return Ok(());
    }
    apply_snap_settings(&settings)?;
    run_configure_hook()?;
    Ok(())
}

fn run_cluster_preseed(args: ClusterPreseedArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let input = match args.file {
        Some(path) => std::fs::read_to_string(path)?,
        None => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
    };
    let parsed: ClusterPreseed = serde_yaml::from_str(&input)?;
    let node_name = cluster_node_name(None)?;
    let systems = parsed.systems.unwrap_or_default();
    let system = systems
        .iter()
        .find(|s| s.name == node_name)
        .ok_or_else(|| anyhow!("preseed does not include local node {node_name}"))?;
    let internal_url = system
        .address
        .clone()
        .ok_or_else(|| anyhow!("preseed entry for {node_name} is missing address"))?;
    let cluster = parsed
        .cluster
        .ok_or_else(|| anyhow!("preseed is missing cluster section"))?;
    let shared_secret = cluster
        .shared_secret
        .ok_or_else(|| anyhow!("preseed cluster.shared_secret is required"))?;
    let backend = resolve_cluster_backend(cluster.backend)?;
    let internal_listen = cluster
        .internal_listen
        .unwrap_or_else(|| "0.0.0.0:8081".to_string());
    let members = systems
        .into_iter()
        .filter_map(|s| s.address.map(|url| ClusterMember { name: s.name, url }))
        .collect::<Vec<_>>();
    let settings = vec![
        ("cluster.node-name".to_string(), node_name),
        ("cluster.internal-listen".to_string(), internal_listen),
        ("cluster.internal-url".to_string(), internal_url),
        ("cluster.shared-secret".to_string(), shared_secret),
        (
            "cluster.backend".to_string(),
            backend.as_snap_value().to_string(),
        ),
        (
            "cluster.members".to_string(),
            format_cluster_members(&members),
        ),
    ];
    if args.dry_run {
        for (k, v) in settings {
            println!("{k}={v}");
        }
        return Ok(());
    }
    apply_snap_settings(&settings)?;
    run_configure_hook()?;
    Ok(())
}

fn run_cluster_remove(args: ClusterNodeArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let mut members = parse_cluster_members(&snapctl_get("cluster.members")?);
    members.retain(|m| m.name != args.node);
    apply_snap_settings(&[(
        "cluster.members".to_string(),
        format_cluster_members(&members),
    )])?;
    Ok(())
}

fn run_cluster_evacuate(args: ClusterNodeArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let key = format!("cluster.evacuated.{}", args.node);
    apply_snap_settings(&[(key, "true".to_string())])?;
    Ok(())
}

fn run_cluster_restore(args: ClusterNodeArgs) -> Result<()> {
    ensure_cluster_mutable()?;
    let key = format!("cluster.evacuated.{}", args.node);
    apply_snap_settings(&[(key, "false".to_string())])?;
    Ok(())
}

fn run_cluster_rotate_secrets() -> Result<()> {
    ensure_cluster_mutable()?;
    apply_snap_settings(&[("cluster.shared-secret".to_string(), generate_secret(48))])?;
    run_configure_hook()?;
    Ok(())
}

fn current_cluster_backend() -> Result<ClusterBackend> {
    let raw = snapctl_get("cluster.backend").unwrap_or_default();
    ClusterBackend::from_snap_value(&raw)
}

fn resolve_cluster_backend(value: Option<ClusterBackend>) -> Result<ClusterBackend> {
    match value {
        Some(v) => Ok(v),
        None => current_cluster_backend(),
    }
}

fn cluster_node_name(input: Option<String>) -> Result<String> {
    if let Some(value) = input {
        validate_identifier("cluster node name", &value)?;
        return Ok(value);
    }
    if let Ok(value) = snapctl_get("cluster.node-name") {
        if !value.trim().is_empty() {
            return Ok(value);
        }
    }
    if let Ok(value) = std::env::var("HOSTNAME") {
        let normalized = value.replace('-', "_");
        validate_identifier("cluster node name", &normalized)?;
        return Ok(normalized);
    }
    Ok("harvest_node".to_string())
}

fn cluster_internal_url(input: Option<String>) -> Result<String> {
    if let Some(value) = input {
        validate_cluster_url(&value)?;
        return Ok(value);
    }
    let existing = snapctl_get("cluster.internal-url")?;
    if !existing.trim().is_empty() {
        validate_cluster_url(&existing)?;
        return Ok(existing);
    }
    bail!("cluster internal URL is required (pass --internal-url)")
}

fn validate_cluster_url(value: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(value).context("invalid cluster URL")?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        bail!("cluster URL must be http or https");
    }
    if parsed.host_str().is_none() {
        bail!("cluster URL must include a host");
    }
    Ok(())
}

fn parse_cluster_members(raw: &str) -> Vec<ClusterMember> {
    raw.split(',')
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            let (name, url) = trimmed.split_once('@')?;
            if name.is_empty() || url.is_empty() {
                return None;
            }
            Some(ClusterMember {
                name: name.to_string(),
                url: url.to_string(),
            })
        })
        .collect()
}

fn format_cluster_members(members: &[ClusterMember]) -> String {
    members
        .iter()
        .map(|m| format!("{}@{}", m.name, m.url))
        .collect::<Vec<_>>()
        .join(",")
}

fn upsert_cluster_member(members: &mut Vec<ClusterMember>, member: ClusterMember) {
    if let Some(existing) = members.iter_mut().find(|m| m.name == member.name) {
        *existing = member;
        return;
    }
    members.push(member);
    members.sort_by(|a, b| a.name.cmp(&b.name));
}

fn apply_snap_settings(settings: &[(String, String)]) -> Result<()> {
    let mut args = vec!["set".to_string()];
    for (k, v) in settings {
        args.push(format!("{k}={v}"));
    }
    let status = Command::new("snapctl")
        .args(args)
        .status()
        .context("running snapctl set")?;
    if !status.success() {
        bail!("snapctl set failed with status {status}");
    }
    Ok(())
}

fn run_configure_hook() -> Result<()> {
    let snap = std::env::var("SNAP").context("SNAP environment variable is missing")?;
    let hook = format!("{snap}/meta/hooks/configure");
    let status = Command::new(hook)
        .status()
        .context("running configure hook")?;
    if !status.success() {
        bail!("configure hook failed with status {status}");
    }
    Ok(())
}

fn running_as_root() -> bool {
    std::env::var("SNAP_EUID")
        .map(|value| value == "0")
        .unwrap_or(false)
}

fn snapctl_get(key: &str) -> Result<String> {
    let output = Command::new("snapctl")
        .args(["get", key])
        .output()
        .with_context(|| format!("running snapctl get {key}"))?;
    if !output.status.success() {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn snapctl_get_json(key: &str) -> Result<String> {
    let output = Command::new("snapctl")
        .args(["get", "-d", key])
        .output()
        .with_context(|| format!("running snapctl get -d {key}"))?;
    if !output.status.success() {
        return Ok("{}".to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_snapctl<const N: usize>(args: [&str; N]) -> Result<()> {
    let status = Command::new("snapctl").args(args).status()?;
    if !status.success() {
        bail!("snapctl {:?} failed", args);
    }
    Ok(())
}

fn snapctl_services() -> Result<String> {
    let output = Command::new("snapctl")
        .arg("services")
        .output()
        .context("running snapctl services")?;
    if !output.status.success() {
        bail!("snapctl services failed with status {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn parse_services_table(raw: &str) -> HashMap<String, (String, String)> {
    let mut out = HashMap::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        if lower.starts_with("service") {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 3 {
            continue;
        }
        out.insert(
            cols[0].to_string(),
            (cols[1].to_string(), cols[2].to_string()),
        );
    }
    out
}

fn find_service_state(
    states: &HashMap<String, (String, String)>,
    full: &str,
    short: &str,
) -> (String, String) {
    states
        .get(full)
        .cloned()
        .or_else(|| states.get(short).cloned())
        .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()))
}

fn service_report(
    name: &str,
    state: (String, String),
    health: Result<bool>,
    health_url: String,
) -> ServiceReport {
    match health {
        Ok(healthy) => ServiceReport {
            service: name.to_string(),
            startup: state.0,
            current: state.1,
            healthy,
            detail: format!("health probe {health_url}"),
        },
        Err(e) => ServiceReport {
            service: name.to_string(),
            startup: state.0,
            current: state.1,
            healthy: false,
            detail: format!("health probe {health_url} failed: {e}"),
        },
    }
}

async fn check_http_ok(url: &str) -> Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    let resp = client.get(url).send().await?;
    Ok(resp.status().is_success())
}

fn require_snapctl() -> Result<()> {
    let status = Command::new("sh")
        .args(["-c", "command -v snapctl >/dev/null 2>&1"])
        .status()
        .context("checking snapctl")?;
    if !status.success() {
        bail!("snapctl was not found. Run this command from inside the installed harvest snap.");
    }
    Ok(())
}

fn non_empty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

fn to_local_probe_host(host: &str) -> &str {
    if host == "0.0.0.0" || host == "::" || host == "" {
        "127.0.0.1"
    } else {
        host
    }
}

fn to_public_host(host: &str) -> String {
    if host == "0.0.0.0" || host == "::" || host == "" || host == "127.0.0.1" {
        "localhost".to_string()
    } else {
        host.to_string()
    }
}

fn validate_database_url(url: &str) -> Result<()> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        bail!("database.url cannot be empty");
    }
    if !trimmed.starts_with("postgres://") && !trimmed.starts_with("postgresql://") {
        bail!("database.url must start with postgres:// or postgresql://");
    }
    Ok(())
}

fn validate_identifier(name: &str, value: &str) -> Result<()> {
    let mut chars = value.chars();
    let first = chars
        .next()
        .ok_or_else(|| anyhow!("{name} cannot be empty"))?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        bail!("{name} must start with a letter or underscore");
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("{name} can only contain letters, digits, and underscores");
    }
    Ok(())
}

fn validate_shell_safe(name: &str, value: &str) -> Result<()> {
    if value.contains('\'') || value.contains('\n') || value.contains('\r') {
        bail!("{name} contains unsupported characters");
    }
    Ok(())
}

fn generate_secret(len: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn generate_hex_secret(len: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut rng = thread_rng();
    (0..len)
        .map(|_| HEX[rng.gen_range(0..16)] as char)
        .collect()
}

fn validate_user_key_encryption_key(value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.len() != 64 {
        bail!("security.user-key-encryption-key must be exactly 64 hex characters");
    }
    if !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("security.user-key-encryption-key must contain only hex characters");
    }
    Ok(())
}

fn model_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SNAP_COMMON") {
        return PathBuf::from(dir).join("model-cache");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("harvest")
            .join("model-cache");
    }
    std::env::temp_dir().join("harvest-model-cache")
}

fn model_cache_path(kind: ProviderKind, base_url: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    kind.as_snap_value().hash(&mut hasher);
    base_url.trim().to_lowercase().hash(&mut hasher);
    let key = hasher.finish();
    model_cache_dir().join(format!("{key:016x}.json"))
}

fn load_model_cache(kind: ProviderKind, base_url: &str) -> Result<Option<Vec<String>>> {
    let path = model_cache_path(kind, base_url);
    if !path.exists() {
        return Ok(None);
    }
    let data =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let parsed: ModelCacheEntry =
        serde_json::from_str(&data).with_context(|| format!("parsing {}", path.display()))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(parsed.fetched_unix) > 86_400 {
        return Ok(None);
    }
    Ok(Some(parsed.models))
}

fn store_model_cache(kind: ProviderKind, base_url: &str, models: &[String]) -> Result<()> {
    let dir = model_cache_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = model_cache_path(kind, base_url);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = ModelCacheEntry {
        fetched_unix: now,
        models: models.to_vec(),
    };
    let encoded = serde_json::to_vec(&entry)?;
    std::fs::write(&path, encoded).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_services_works() {
        let raw = "Service         Startup  Current\nharvest.server  enabled  active\nharvest.ui      enabled  inactive\n";
        let parsed = parse_services_table(raw);
        let server = parsed.get("harvest.server").expect("server row");
        assert_eq!(server.0, "enabled");
        assert_eq!(server.1, "active");
    }

    #[test]
    fn public_host_maps_local_aliases() {
        assert_eq!(to_public_host("127.0.0.1"), "localhost");
        assert_eq!(to_public_host("0.0.0.0"), "localhost");
        assert_eq!(to_public_host("10.0.0.7"), "10.0.0.7");
    }

    #[test]
    fn validate_identifier_accepts_expected_values() {
        validate_identifier("x", "harvest_1").expect("valid identifier");
        assert!(validate_identifier("x", "1bad").is_err());
        assert!(validate_identifier("x", "bad-name").is_err());
    }

    #[test]
    fn model_cache_roundtrip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("SNAP_COMMON", tmp.path());
        let models = vec!["m1".to_string(), "m2".to_string()];
        store_model_cache(
            ProviderKind::OpenAiCompatible,
            "https://example.com/v1",
            &models,
        )
        .expect("store");
        let loaded = load_model_cache(ProviderKind::OpenAiCompatible, "https://example.com/v1")
            .expect("load")
            .expect("models");
        assert_eq!(loaded, models);
        std::env::remove_var("SNAP_COMMON");
    }

    #[test]
    fn cluster_members_roundtrip() {
        let members = vec![
            ClusterMember {
                name: "n1".to_string(),
                url: "http://10.0.0.1:8081".to_string(),
            },
            ClusterMember {
                name: "n2".to_string(),
                url: "http://10.0.0.2:8081".to_string(),
            },
        ];
        let encoded = format_cluster_members(&members);
        let decoded = parse_cluster_members(&encoded);
        assert_eq!(decoded, members);
    }

    #[test]
    fn cluster_member_upsert_replaces_existing() {
        let mut members = vec![ClusterMember {
            name: "n1".to_string(),
            url: "http://10.0.0.1:8081".to_string(),
        }];
        upsert_cluster_member(
            &mut members,
            ClusterMember {
                name: "n1".to_string(),
                url: "http://10.0.0.11:8081".to_string(),
            },
        );
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].url, "http://10.0.0.11:8081");
    }

    #[test]
    fn cluster_member_upsert_adds_and_sorts() {
        let mut members = vec![ClusterMember {
            name: "n2".to_string(),
            url: "http://10.0.0.2:8081".to_string(),
        }];
        upsert_cluster_member(
            &mut members,
            ClusterMember {
                name: "n1".to_string(),
                url: "http://10.0.0.1:8081".to_string(),
            },
        );
        assert_eq!(members[0].name, "n1");
        assert_eq!(members[1].name, "n2");
    }

    #[test]
    fn cluster_url_validation_rejects_invalid_schemes() {
        assert!(validate_cluster_url("ftp://example.com").is_err());
        assert!(validate_cluster_url("http://example.com:8081").is_ok());
    }

    #[test]
    fn cluster_preseed_parses_yaml() {
        let yaml = r#"
cluster:
  backend: "postgres"
  internal_listen: "0.0.0.0:8081"
  shared_secret: "secret"
systems:
  - name: n1
    address: "http://10.0.0.1:8081"
"#;
        let parsed: ClusterPreseed = serde_yaml::from_str(yaml).expect("preseed");
        assert_eq!(
            parsed
                .cluster
                .expect("cluster")
                .shared_secret
                .expect("secret"),
            "secret"
        );
        assert_eq!(parsed.systems.expect("systems").len(), 1);
    }

    #[test]
    fn cluster_backend_parsing_defaults_to_postgres() {
        assert_eq!(
            ClusterBackend::from_snap_value("").expect("default backend"),
            ClusterBackend::Postgres
        );
        assert_eq!(
            ClusterBackend::from_snap_value("postgres").expect("postgres backend"),
            ClusterBackend::Postgres
        );
        assert_eq!(
            ClusterBackend::from_snap_value("quorium").expect("quorium backend"),
            ClusterBackend::Quorium
        );
        assert!(ClusterBackend::from_snap_value("invalid").is_err());
    }
}
