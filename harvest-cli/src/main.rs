use std::collections::{hash_map::DefaultHasher, BTreeSet, HashMap};
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Password, Select};
use indicatif::{ProgressBar, ProgressStyle};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init(args) => run_init(args).await,
        Commands::PrepareDbScript(args) => run_prepare_db_script(args),
        Commands::Status(args) => run_status(args).await,
        Commands::ShowPublicUrl(args) => run_show_public_url(args),
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

    let database_url = if let Some(url) = args.from_db_url {
        validate_database_url(&url)?;
        url
    } else {
        let mut entries = vec!["Use existing database.url".to_string()];
        entries.push("Use harvest prepare-db-script workflow".to_string());
        let choice = Select::with_theme(&theme)
            .with_prompt("Database setup")
            .items(&entries)
            .default(0)
            .interact()?;

        if choice == 0 {
            ask_database_url(
                &theme,
                if current_db.is_empty() {
                    None
                } else {
                    Some(current_db)
                },
            )?
        } else {
            println!("Run this command in another terminal and complete it:");
            println!("  harvest prepare-db-script > /tmp/harvest-db-setup.sh");
            println!("  sudo bash -x /tmp/harvest-db-setup.sh");
            println!("Then paste the printed database.url value.");
            ask_database_url(&theme, None)?
        }
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
    let provider_api_key = Password::with_theme(&theme)
        .with_prompt("Provider API key")
        .with_confirmation("Confirm provider API key", "Keys do not match")
        .interact()?;

    let detected_models = if let Some(cached) = load_model_cache(provider_kind, &provider_url)? {
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
        select_models_multi(&theme, &detected_models, &model)?
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
        ("llm.primary.api-key".to_string(), provider_api_key),
        ("llm.primary.model".to_string(), model),
        ("llm.primary.priority".to_string(), "0".to_string()),
    ];

    if let ProviderKind::OpenAiCompatible = provider_kind {
        settings.push(("llm.primary.base-url".to_string(), provider_url));
    }

    if let Some(models) = models_allowlist {
        settings.push(("llm.primary.models".to_string(), models.join(",")));
    }

    println!();
    println!("Pending configuration:");
    for (k, v) in &settings {
        if k.contains("secret") || k.contains("api-key") {
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

fn select_models_multi(
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
END \$\$;
SQL"

if ! $ROOT_CMD bash -c "${{PSQL}} -U postgres -d postgres -tAc \"SELECT 1 FROM pg_database WHERE datname = '$DB_NAME'\"" | grep -q '^1$'; then
  $ROOT_CMD bash -c "${{PSQL}} -v ON_ERROR_STOP=1 -U postgres -d postgres -c 'CREATE DATABASE \"'$DB_NAME'\" OWNER \"'$DB_USER'\";'"
fi

$ROOT_CMD bash -c "${{PSQL}} -v ON_ERROR_STOP=1 -U postgres -d postgres -c 'GRANT ALL PRIVILEGES ON DATABASE \"'$DB_NAME'\" TO \"'$DB_USER'\";'"

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
}
