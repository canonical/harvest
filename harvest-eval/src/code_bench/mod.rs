pub mod client;
pub mod report;
pub mod runner;
pub mod task;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use harvest_db::Db;
use knowledge_server::config::Config;

use client::Harvest;
use report::{summarize, table, RunResult, TaskSummary};
use runner::Bench;

pub struct Options<'a> {
    pub config: &'a Path,
    pub server: &'a str,
    pub token: &'a str,
    pub tasks: &'a Path,
    pub only: Option<&'a str>,
    pub runs: usize,
    pub keep_sessions: bool,
}

#[derive(serde::Serialize)]
pub struct Report {
    pub summaries: Vec<TaskSummary>,
    pub runs: Vec<RunResult>,
}

pub async fn run(options: Options<'_>) -> Result<Report> {
    let tasks = task::load_dir(options.tasks, options.only)?;
    let config = Config::from_file(options.config)?;
    let db = Arc::new(Db::connect(&config.database.url).await?);
    let lxd_config = config.lxd.as_ref().context("the server configuration has no [lxd] section")?;
    let lxd = knowledge_server::lxd::resolve_client(lxd_config, &db)
        .await?
        .context("Harvest has no trusted LXD identity yet")?;
    let bench = Bench {
        harvest: Harvest::new(options.server, options.token)?,
        lxd,
        db,
        keep_sessions: options.keep_sessions,
    };
    let mut runs = Vec::new();
    let mut summaries = Vec::new();
    for task in &tasks {
        let mut task_runs = Vec::new();
        for run in 1..=options.runs {
            tracing::info!(task = %task.name, run, "running");
            let result = bench.run(task, run).await;
            if let Some(error) = &result.error {
                tracing::warn!(task = %task.name, run, error, "run failed");
            }
            task_runs.push(result);
        }
        summaries.push(summarize(&task.name, &task_runs));
        runs.extend(task_runs);
    }
    println!("{}", table(&summaries));
    Ok(Report { summaries, runs })
}
