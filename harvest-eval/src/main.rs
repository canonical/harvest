use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use harvest_db::Db;
use harvest_search::fallback::PostgresEngine;
use harvest_search::index::pg::PgCorpus;
use harvest_search::llm::LanguageModel;
use harvest_search::{Scope, SearchEngine};
use knowledge_server::config::{Config, SearchConfig};
use knowledge_server::search::legacy_engine::LegacyLoopEngine;
use knowledge_server::search::llm_adapter::ProviderModel;
use knowledge_server::search::wiring;

use harvest_eval::gold::{self, Split};
use harvest_eval::judge::{Answerer, Judge};
use harvest_eval::report::{compare, summarize, Gates, Report};
use harvest_eval::run::{run, Run};
use harvest_eval::{calibrate, distill, extract, synth};

#[derive(Parser)]
#[command(
    name = "harvest-eval",
    about = "Measure the quality and latency of Harvest code search"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum EngineKind {
    Fallback,
    Tier1,
    Tiered,
    Legacy,
}

impl EngineKind {
    fn name(self) -> &'static str {
        match self {
            Self::Fallback => "fallback",
            Self::Tier1 => "tier1",
            Self::Tiered => "tiered",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(clap::Args)]
struct Connection {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, env = "HARVEST_DATABASE_URL")]
    database_url: Option<String>,
    #[arg(long)]
    reranker: Option<PathBuf>,
    #[arg(long)]
    gate: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    Synth {
        #[command(flatten)]
        connection: Connection,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        version: String,
        #[arg(long, default_value_t = 25)]
        per_kind: usize,
        #[arg(long)]
        out: PathBuf,
    },
    Extract {
        #[command(flatten)]
        connection: Connection,
        #[arg(long)]
        out: PathBuf,
    },
    Run {
        #[command(flatten)]
        connection: Connection,
        #[arg(long)]
        gold: PathBuf,
        #[arg(long, value_enum, default_value = "dev")]
        split: Split,
        #[arg(long, value_enum)]
        engine: EngineKind,
        #[arg(long, default_value_t = 10_000)]
        budget_ms: u64,
        #[arg(long)]
        token_budget: Option<usize>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        label_gate: bool,
        #[arg(long)]
        answers: bool,
        #[arg(long)]
        judge: bool,
        #[arg(long)]
        references: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    Distill {
        #[command(flatten)]
        connection: Connection,
        #[arg(long)]
        gold: PathBuf,
        #[arg(long, value_enum, default_value = "all")]
        split: Split,
        #[arg(long, value_enum, default_value = "legacy")]
        engine: EngineKind,
        #[arg(long, default_value_t = 600_000)]
        budget_ms: u64,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        cache: bool,
        #[arg(long)]
        out: PathBuf,
    },
    Compare {
        base: PathBuf,
        new: PathBuf,
        #[arg(long)]
        gates: Option<PathBuf>,
    },
    Show {
        report: PathBuf,
    },
    CalibrateGate {
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    TrainReranker {
        #[command(flatten)]
        connection: Connection,
        #[arg(long)]
        gold: PathBuf,
        #[arg(long, value_enum, default_value = "train")]
        split: Split,
        #[arg(long)]
        out: PathBuf,
    },
}

struct Env {
    config: Option<Config>,
    db: Arc<Db>,
    reranker: Option<PathBuf>,
    gate: Option<PathBuf>,
}

async fn connect(connection: &Connection) -> Result<Env> {
    let config = connection
        .config
        .as_deref()
        .map(Config::from_file)
        .transpose()?;
    let url = connection
        .database_url
        .clone()
        .or_else(|| config.as_ref().map(|c| c.database.url.clone()))
        .context("pass --database-url or --config")?;
    Ok(Env {
        config,
        db: Arc::new(Db::connect(&url).await?),
        reranker: connection.reranker.clone(),
        gate: connection.gate.clone(),
    })
}

fn search_config(context: &Env) -> SearchConfig {
    let mut config = context
        .config
        .as_ref()
        .map(|c| c.search.clone())
        .unwrap_or_default();
    if let Some(path) = &context.reranker {
        config.reranker = Some(path.clone());
    }
    if let Some(path) = &context.gate {
        config.gate = Some(path.clone());
    }
    config
}

fn model(context: &Env, fast: bool) -> Result<Arc<dyn LanguageModel>> {
    let config = context
        .config
        .as_ref()
        .context("this needs --config with an [[llm]] provider")?;
    let llm = knowledge_server::llm::from_config(&config.llm);
    let selection = fast
        .then(|| config.search.fast_model.clone())
        .flatten()
        .map(|m| knowledge_server::llm::types::ProviderSelection {
            provider_id: m.provider_id,
            model: m.model,
            cache_breakpoint_index: None,
        });
    Ok(ProviderModel::new(llm, selection))
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn read_report(path: &Path) -> Result<Report> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(serde_json::from_str(&text)?)
}

fn commit() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn print_summary(report: &Report) {
    let s = &report.summary;
    println!(
        "engine {} on {} ({} split), {} queries, {} errors",
        report.engine, report.gold, report.split, s.queries, s.errors
    );
    println!(
        "  evidence recall   essential {:.3}  all {:.3}  pool {}",
        s.recall_essential,
        s.recall_all,
        s.pool_recall.map_or("-".into(), |p| format!("{p:.3}"))
    );
    println!(
        "  ranking           nDCG@10 {:.3}  precision {:.3}",
        s.ndcg, s.precision
    );
    println!(
        "  latency ms        p50 {:.0}  p90 {:.0}  p99 {:.0}  max {:.0}",
        s.latency.p50, s.latency.p90, s.latency.p99, s.latency.max
    );
    println!("  within budget     {:.1}%", s.budget_compliance * 100.0);
    for (tier, latency) in &s.latency_by_tier {
        println!(
            "  {tier:<17} {:>5.1}% of queries, p50 {:.0} ms, p99 {:.0} ms, at most {} model calls",
            s.escalation.get(tier).copied().unwrap_or(0.0) * 100.0,
            latency.p50,
            latency.p99,
            s.max_llm_calls_by_tier.get(tier).copied().unwrap_or(0)
        );
    }
    for (reason, share) in &s.degraded {
        println!("  degraded          {reason}: {:.1}%", share * 100.0);
    }
    if let Some(missed) = s.missed_insufficiency {
        println!(
            "  gate              missed insufficiency {:.1}%  unneeded escalation {}",
            missed * 100.0,
            s.unneeded_escalation
                .map_or("-".into(), |u| format!("{:.1}%", u * 100.0))
        );
    }
    for (kind, recall) in &s.recall_by_kind {
        println!("  recall by kind    {kind}: {recall:.3}");
    }
    if let Some(judge) = &s.judge {
        println!(
            "  answers           {} wins, {} ties, {} losses, margin {:+.3} ({:+.3}..{:+.3})",
            judge.wins,
            judge.ties,
            judge.losses,
            judge.margin,
            judge.margin_ci.0,
            judge.margin_ci.1
        );
    }
    let mut slow: Vec<_> = s.stage_p99.iter().collect();
    slow.sort_by(|a, b| b.1.total_cmp(a.1));
    for (stage, ms) in slow.into_iter().take(8) {
        println!("  stage p99         {stage}: {ms:.1} ms");
    }
}

async fn warm(parts: &wiring::Parts, queries: &[gold::GoldQuery]) -> Result<BTreeMap<String, f64>> {
    let scopes: BTreeSet<Scope> = queries
        .iter()
        .map(|q| Scope::new(&q.repo, &q.version))
        .collect();
    let mut index_load_ms = BTreeMap::new();
    for scope in scopes {
        let started = Instant::now();
        parts.registry.ensure(&scope).await?;
        index_load_ms.insert(
            format!("{}@{}", scope.repo, scope.version),
            started.elapsed().as_secs_f64() * 1000.0,
        );
    }
    Ok(index_load_ms)
}

async fn build_engine(
    context: &Env,
    engine: EngineKind,
    config: &SearchConfig,
    parts: &wiring::Parts,
) -> Result<Box<dyn SearchEngine>> {
    Ok(match engine {
        EngineKind::Fallback => Box::new(PostgresEngine::new(Arc::clone(&context.db))),
        EngineKind::Tier1 => Box::new(wiring::tiered(config, parts, true)),
        EngineKind::Tiered => Box::new(wiring::tiered(config, parts, false)),
        EngineKind::Legacy => {
            let cfg = context
                .config
                .as_ref()
                .context("the legacy engine needs --config")?;
            knowledge_server::agent::graph_tools::set_run_sql_available(
                knowledge_server::agent::graph_tools::probe_run_sql(&context.db).await,
            );
            let agent = knowledge_server::agent::Agent::new(
                knowledge_server::llm::from_config(&cfg.llm),
                knowledge_server::agent::graph_tools::all_tools(Arc::clone(&context.db)),
                cfg.agent.max_iterations,
            );
            Box::new(LegacyLoopEngine::new(Arc::new(agent)))
        }
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber_init();
    match Cli::parse().command {
        Command::Synth {
            connection,
            repo,
            version,
            per_kind,
            out,
        } => {
            let context = connect(&connection).await?;
            let queries = synth::Synthesizer::new(&context.db, &repo, &version)
                .generate(per_kind)
                .await?;
            gold::save(&out, &queries)?;
            println!(
                "wrote {} synthetic queries to {}",
                queries.len(),
                out.display()
            );
        }
        Command::Extract { connection, out } => {
            let context = connect(&connection).await?;
            let queries = extract::extract(&context.db).await?;
            gold::save(&out, &queries)?;
            println!(
                "wrote {} queries from conversation logs to {}",
                queries.len(),
                out.display()
            );
        }
        Command::Run {
            connection,
            gold: gold_path,
            split,
            engine,
            budget_ms,
            token_budget,
            limit,
            label_gate,
            answers,
            judge,
            references,
            out,
        } => {
            let context = connect(&connection).await?;
            let mut queries: Vec<_> = gold::load(&gold_path)?
                .into_iter()
                .filter(|q| q.in_split(split))
                .collect();
            if let Some(limit) = limit {
                queries.truncate(limit);
            }
            if queries.is_empty() {
                bail!(
                    "no gold queries in the {split:?} split of {}",
                    gold_path.display()
                );
            }
            let config = search_config(&context);
            let token_budget = token_budget.unwrap_or(config.token_budget);
            let fast_model = match engine {
                EngineKind::Tiered => Some(model(&context, true)?),
                _ => None,
            };
            let parts = wiring::parts(&config, PgCorpus::new(Arc::clone(&context.db)), fast_model)?;
            let index_load_ms = if matches!(engine, EngineKind::Tier1 | EngineKind::Tiered) {
                warm(&parts, &queries).await?
            } else {
                BTreeMap::new()
            };
            let tier1 = wiring::tiered(&config, &parts, true);
            let engine_box = build_engine(&context, engine, &config, &parts).await?;
            let answerer = if answers || judge {
                Some(Answerer::new(model(&context, false)?))
            } else {
                None
            };
            let judging = if judge {
                Some(Judge::new(model(&context, false)?))
            } else {
                None
            };
            let references = match references {
                Some(path) => read_report(&path)?
                    .records
                    .into_iter()
                    .filter_map(|r| r.answer.map(|a| (r.id, a)))
                    .collect(),
                None => BTreeMap::new(),
            };
            let records = run(
                &queries,
                &Run {
                    engine: engine_box.as_ref(),
                    tier1: (label_gate && engine == EngineKind::Tiered)
                        .then_some(&tier1 as &dyn SearchEngine),
                    budget_ms,
                    token_budget,
                    answerer: answerer.as_ref(),
                    judge: judging.as_ref(),
                    references,
                },
            )
            .await;
            let report = Report {
                engine: engine.name().into(),
                split: format!("{split:?}").to_lowercase(),
                gold: gold_path.display().to_string(),
                commit: commit(),
                created_at: chrono::Utc::now().to_rfc3339(),
                budget_ms,
                index_load_ms,
                summary: summarize(&records, budget_ms),
                records,
            };
            write_json(&out, &report)?;
            print_summary(&report);
            println!("report written to {}", out.display());
        }
        Command::Distill {
            connection,
            gold: gold_path,
            split,
            engine,
            budget_ms,
            limit,
            cache,
            out,
        } => {
            let context = connect(&connection).await?;
            let mut queries: Vec<_> = gold::load(&gold_path)?
                .into_iter()
                .filter(|q| q.in_split(split))
                .collect();
            if let Some(limit) = limit {
                queries.truncate(limit);
            }
            let config = search_config(&context);
            let fast_model = match engine {
                EngineKind::Tiered => Some(model(&context, true)?),
                _ => None,
            };
            let parts = wiring::parts(&config, PgCorpus::new(Arc::clone(&context.db)), fast_model)?;
            if matches!(engine, EngineKind::Tier1 | EngineKind::Tiered) {
                warm(&parts, &queries).await?;
            }
            let engine_box = build_engine(&context, engine, &config, &parts).await?;
            let mut distilled = Vec::new();
            for query in &queries {
                let result = engine_box
                    .search(
                        &harvest_eval::run::query_for(query, config.token_budget),
                        harvest_search::Deadline::after(std::time::Duration::from_millis(
                            budget_ms,
                        )),
                    )
                    .await;
                let Some(gold) = result
                    .ok()
                    .and_then(|r| distill::gold_from_result(query, &r, engine.name()))
                else {
                    continue;
                };
                if cache {
                    distill::store_cache(&context.db, &gold).await?;
                }
                distilled.push(gold);
            }
            gold::save(&out, &distilled)?;
            println!(
                "distilled {} of {} questions into {}{}",
                distilled.len(),
                queries.len(),
                out.display(),
                if cache { " and the evidence cache" } else { "" }
            );
        }
        Command::Compare { base, new, gates } => {
            let gates: Gates = match gates {
                Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
                None => Gates::default(),
            };
            let (base, new) = (read_report(&base)?, read_report(&new)?);
            let results = compare(&base, &new, &gates);
            for result in &results {
                println!(
                    "{} {:<22} {}",
                    if result.passed { "PASS" } else { "FAIL" },
                    result.name,
                    result.detail
                );
            }
            if results.iter().any(|r| !r.passed) {
                std::process::exit(1);
            }
        }
        Command::Show { report } => print_summary(&read_report(&report)?),
        Command::CalibrateGate { report, out } => {
            let report = read_report(&report)?;
            let score = calibrate::calibrate_gate(&report.records)
                .context("the report needs tier 1 signals with both sufficient and insufficient examples (run --engine tier1 or --label-gate)")?;
            write_json(&out, &score)?;
            println!(
                "gate calibration written to {} (accept ≥ {:.3}, reject ≤ {:.3})",
                out.display(),
                score.accept,
                score.reject
            );
        }
        Command::TrainReranker {
            connection,
            gold: gold_path,
            split,
            out,
        } => {
            let context = connect(&connection).await?;
            let queries: Vec<_> = gold::load(&gold_path)?
                .into_iter()
                .filter(|q| q.in_split(split))
                .collect();
            let parts = wiring::parts(
                &search_config(&context),
                PgCorpus::new(Arc::clone(&context.db)),
                None,
            )?;
            let rounds = wiring::rounds(&parts);
            let (rows, labels) =
                calibrate::reranker_rows(&rounds, &parts.registry, &queries).await?;
            let positives = labels.iter().filter(|l| **l).count();
            if positives == 0 {
                bail!("no candidate matched the gold evidence; check the gold set and the index");
            }
            let reranker = calibrate::train_reranker(&rows, &labels);
            write_json(&out, &reranker)?;
            println!(
                "reranker trained on {} candidates ({positives} relevant) written to {}",
                rows.len(),
                out.display()
            );
        }
    }
    Ok(())
}

fn tracing_subscriber_init() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("harvest_eval=info,warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
