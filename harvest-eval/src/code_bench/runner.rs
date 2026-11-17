use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use harvest_db::Db;
use knowledge_server::lxd::containers::Command;
use knowledge_server::lxd::LxdClient;
use serde_json::json;

use super::client::Harvest;
use super::report::RunResult;
use super::task::{acceptance_script, checks_script, Task};

const WORKSPACE: &str = "/workspace";
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ACCEPTANCE_DIR: &str = "/tmp/harvest-bench-acceptance";
const CHECKS_DIR: &str = "/tmp/harvest-bench-checks";
const TEST_PATH_MARKERS: &[&str] = &["test", "spec"];

pub struct Bench {
    pub harvest: Harvest,
    pub lxd: LxdClient,
    pub db: Arc<Db>,
    pub keep_sessions: bool,
}

impl Bench {
    pub async fn run(&self, task: &Task, run: usize) -> RunResult {
        let mut result = RunResult {
            task: task.name.clone(),
            run,
            ..Default::default()
        };
        if let Err(e) = self.attempt(task, run, &mut result).await {
            result.error = Some(format!("{e:#}"));
        }
        result
    }

    async fn attempt(&self, task: &Task, run: usize, result: &mut RunResult) -> Result<()> {
        let project = self.harvest.open_project(&task.repository).await?;
        self.harvest.wait_for_image(&project).await?;
        let session = self
            .harvest
            .start_session(&project, &branch_name(&task.name, run), &task.base)
            .await?;
        let outcome = self.exercise(task, &session, result).await;
        if !self.keep_sessions {
            if let Err(e) = self.harvest.end_session(&session).await {
                tracing::warn!(session, error = %e, "could not end a benchmark session");
            }
        }
        outcome
    }

    async fn exercise(&self, task: &Task, session: &str, result: &mut RunResult) -> Result<()> {
        let container = self.container(session).await?;
        for command in &task.setup {
            let output = self.exec(&container, command).await?;
            if !output.0 {
                bail!("setup command failed: {command}\n{}", output.1);
            }
        }
        let chat = self.harvest.new_chat(session).await?;
        for prompt in &task.prompts {
            result.turns.push(self.harvest.send(session, &chat, prompt).await?);
        }
        if let Some(acceptance) = &task.acceptance {
            result.acceptance = Some(self.exec(&container, &acceptance_script(acceptance, ACCEPTANCE_DIR)).await?.0);
        }
        if let Some(checks) = &task.checks {
            result.checks = Some(self.exec(&container, &checks_script(checks, CHECKS_DIR)).await?.0);
        }
        let numstat = self
            .exec(&container, &format!("git diff --numstat origin/{} --", task.base))
            .await?;
        result.test_lines_added = test_lines_added(&numstat.1);
        Ok(())
    }

    async fn container(&self, session: &str) -> Result<String> {
        let rows = self
            .db
            .query("SELECT container FROM code_sessions WHERE id = $id", json!({ "id": session }))
            .await?;
        rows.first()
            .and_then(|row| row["container"].as_str())
            .map(String::from)
            .context("the session has no container")
    }

    async fn exec(&self, container: &str, script: &str) -> Result<(bool, String)> {
        let output = self
            .lxd
            .run(container, Command::shell(script).in_dir(WORKSPACE).timeout(SETUP_TIMEOUT))
            .await?;
        Ok((output.succeeded(), format!("{}{}", output.stdout, output.stderr)))
    }
}

fn branch_name(task: &str, run: usize) -> String {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    format!("bench/{task}-{run}-{stamp}")
}

pub fn test_lines_added(numstat: &str) -> u64 {
    numstat
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let added = fields.next()?.parse::<u64>().ok()?;
            let path = fields.nth(1)?.to_lowercase();
            TEST_PATH_MARKERS.iter().any(|m| path.contains(m)).then_some(added)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_lines_added_to_test_files_count() {
        let numstat = "12\t3\tsrc/board.js\n40\t0\ttest/history.test.js\n8\t1\ttests/test_timed.py\n-\t-\tlogo.png\n";
        assert_eq!(test_lines_added(numstat), 48);
    }

    #[test]
    fn branch_names_are_unique_per_run() {
        let name = branch_name("tweak", 2);
        assert!(name.starts_with("bench/tweak-2-"));
    }
}
