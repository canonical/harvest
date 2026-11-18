pub mod client;
pub mod report;
pub mod runner;
pub mod task;

use std::path::Path;

use anyhow::Result;

use client::Harvest;
use report::{summarize, table, RunResult, TaskSummary};
use runner::Bench;

use crate::session_containers::SessionContainers;

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
    let bench = Bench {
        harvest: Harvest::new(options.server, options.token)?,
        containers: SessionContainers::from_config(options.config).await?,
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
