mod config;
mod explore;
mod harvest;
mod llm;
mod report;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

#[derive(Parser)]
#[command(name = "squall", about = "Autonomous QA testing utility for Harvest")]
struct Cli {
    #[arg(long, short, env = "SQUALL_CONFIG", default_value = "./squall.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Login,
    SmokeTest,
    Explore {
        #[arg(long)]
        objective: String,
        #[arg(long)]
        max_iterations: Option<usize>,
        #[arg(long)]
        no_infra: bool,
        #[arg(long)]
        out: Option<String>,
    },
}

async fn connect(cfg: &config::HarvestConfig) -> Result<harvest::Client> {
    let token = harvest::login(&cfg.base_url, &cfg.email, &cfg.password)
        .await
        .context("logging into Harvest")?;
    harvest::Client::new(&cfg.base_url, token)
}

async fn run_login(cfg: &config::Config) -> Result<()> {
    let client = connect(&cfg.harvest).await?;
    let me = client.me().await.context("fetching current user")?;
    info!(email = %me["email"], role = %me["role"], "login succeeded");
    println!("logged in as {} ({})", me["email"], me["role"]);
    Ok(())
}

async fn resolve_group_id(client: &harvest::Client, cfg: &config::HarvestConfig) -> Result<String> {
    if let Some(id) = &cfg.group_id {
        return Ok(id.clone());
    }
    let groups = client.list_groups().await?;
    let first = groups
        .first()
        .ok_or_else(|| anyhow::anyhow!("account has no groups and harvest.group_id is not set in config"))?;
    Ok(first["id"].as_str().unwrap_or_default().to_string())
}

async fn run_smoke_test(cfg: &config::Config) -> Result<()> {
    let client = connect(&cfg.harvest).await?;
    let group_id = resolve_group_id(&client, &cfg.harvest).await?;

    println!("[1/8] create_project");
    let project = client.create_project("squall-smoke-test", Some("created by squall smoke-test"), &group_id).await?;
    let project_id = project["id"].as_str().context("create_project response missing id")?;

    println!("[2/8] create_conversation");
    let conversation = client.create_conversation(project_id, Some("smoke test")).await?;
    let conversation_id = conversation["id"].as_str().context("create_conversation response missing id")?;

    println!("[3/8] send_chat_message");
    let query_resp = client.send_chat_message(project_id, "hello", Some(conversation_id)).await?;
    println!("      agent answered ({} chars)", query_resp.answer.len());

    println!("[4/8] create_artifact");
    let artifact = client
        .create_artifact(project_id, "smoke-test-note", "markdown", &serde_json::json!("# smoke test"))
        .await?;
    println!("      artifact id: {}", artifact["id"]);

    println!("[5/8] get_deployment");
    let deployment = client.get_deployment(project_id).await?;
    let deployment_id = deployment["id"].as_str().context("get_deployment response missing id")?;

    println!("[6/8] generate_design");
    client.generate_design(project_id, deployment_id, None).await?;

    println!("[7/8] add_context_artifact");
    client
        .add_context_artifact(project_id, deployment_id, "smoke-test-context", "markdown", &serde_json::json!("context"))
        .await?;

    println!("[8/8] done — all non-destructive Harvest actions succeeded");
    println!("project id: {project_id}");
    Ok(())
}

async fn run_explore(cfg: &config::Config, objective: String, max_iterations: Option<usize>, no_infra: bool, out: Option<String>) -> Result<()> {
    let client = Arc::new(connect(&cfg.harvest).await?);
    let allow_infra = cfg.exploration.allow_infra && !no_infra;
    let max_iterations = max_iterations.unwrap_or(cfg.exploration.max_iterations);
    let output_dir = out.unwrap_or_else(|| cfg.exploration.output_dir.clone());

    let llm = llm::from_config(&cfg.llm);
    let pricing = llm::pricing::PricingTable::from_configs(&cfg.llm);
    let tools = explore::HarvestTool::build_toolset(client, allow_infra);

    info!(objective = %objective, max_iterations, allow_infra, "starting exploration");
    let result = explore::run(llm, tools, &pricing, &objective, max_iterations).await?;

    let dir = report::run_dir(&output_dir, &objective);
    report::write(&dir, &result).await?;

    println!("exploration finished in {} iterations, {} tool calls, {}", result.transcript.len(), result.tool_calls_made, llm::pricing::format_cost(result.estimated_cost_microusd));
    println!("report written to {}", dir.display());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let cfg = config::Config::from_file(&cli.config)?;

    match cli.command {
        Command::Login => run_login(&cfg).await,
        Command::SmokeTest => run_smoke_test(&cfg).await,
        Command::Explore { objective, max_iterations, no_infra, out } => {
            run_explore(&cfg, objective, max_iterations, no_infra, out).await
        }
    }
}
