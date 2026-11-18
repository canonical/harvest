use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use serde::Serialize;

use super::scenario::{Scenario, Verification};
use crate::code_bench::client::Harvest;
use crate::session_containers::SessionContainers;

const PAGE_ROUNDS: [&str; 2] = ["after starting", "after a restart"];

#[derive(Debug, Default, Serialize)]
pub struct Outcome {
    pub scenario: String,
    pub image_secs: u64,
    pub session_secs: u64,
    pub preview_secs: u64,
    pub passed: bool,
    pub error: Option<String>,
}

pub struct Check {
    pub harvest: Harvest,
    pub containers: SessionContainers,
    pub keep_sessions: bool,
}

impl Check {
    pub async fn run(&self, scenario: &Scenario) -> Outcome {
        let mut outcome = Outcome { scenario: scenario.name.clone(), ..Outcome::default() };
        if let Err(error) = self.attempt(scenario, &mut outcome).await {
            outcome.error = Some(format!("{error:#}"));
        }
        outcome
    }

    async fn attempt(&self, scenario: &Scenario, outcome: &mut Outcome) -> Result<()> {
        let started = Instant::now();
        let project = self.harvest.open_project(&scenario.repository).await?;
        self.harvest.wait_for_image(&project).await?;
        outcome.image_secs = started.elapsed().as_secs();

        let started = Instant::now();
        let session = self.harvest.start_session(&project, &branch_name(&scenario.name), &scenario.base).await?;
        outcome.session_secs = started.elapsed().as_secs();

        let exercised = self.exercise(scenario, &session, outcome).await;
        if !self.keep_sessions {
            if let Err(error) = self.harvest.end_session(&session).await {
                tracing::warn!(session, error = %error, "could not end a preview check session");
            }
        }
        exercised
    }

    async fn exercise(&self, scenario: &Scenario, session: &str, outcome: &mut Outcome) -> Result<()> {
        let container = self.containers.of_session(session).await?;
        for command in &scenario.prepare {
            let ran = self.containers.run(&container, command).await?;
            if !ran.succeeded {
                bail!("preparing failed: {command}\n{}", ran.output);
            }
        }
        let started = Instant::now();
        let preview = self.harvest.create_preview(session, scenario.preview.clone()).await?;
        match &scenario.verification {
            Verification::Page { port, path, contains } => {
                for round in PAGE_ROUNDS {
                    self.harvest.restart_preview(&preview).await?;
                    let page = self.containers.run(&container, &page_request(*port, path)).await?;
                    expect(page.succeeded, &page.output, contains, round)?;
                }
            }
            Verification::Command { command, contains } => {
                let result = self.harvest.run_in_preview(&preview, command).await?;
                let output = format!("{}{}", result["stdout"].as_str().unwrap_or_default(), result["stderr"].as_str().unwrap_or_default());
                expect(result["exit_code"].as_i64() == Some(0), &output, contains, "running the command")?;
            }
        }
        outcome.preview_secs = started.elapsed().as_secs();
        outcome.passed = true;
        Ok(())
    }
}

fn page_request(port: u16, path: &str) -> String {
    format!("curl -sS --max-time 30 'http://127.0.0.1:{port}{path}'")
}

fn expect(succeeded: bool, output: &str, contains: &str, when: &str) -> Result<()> {
    if !succeeded || !output.contains(contains) {
        bail!("expected {contains:?} {when}, got:\n{}", output.chars().take(2_000).collect::<String>());
    }
    Ok(())
}

fn branch_name(scenario: &str) -> String {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    format!("preview-check/{scenario}-{stamp}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_requested_inside_the_session_container() {
        assert_eq!(page_request(5000, "/auth/login"), "curl -sS --max-time 30 'http://127.0.0.1:5000/auth/login'");
    }

    #[test]
    fn an_expectation_needs_success_and_the_expected_text() {
        assert!(expect(true, "<h1>Log In</h1>", "Log In", "after starting").is_ok());
        assert!(expect(false, "<h1>Log In</h1>", "Log In", "after starting").is_err());
        let missing = expect(true, "Internal Server Error", "Log In", "after a restart").unwrap_err();
        assert!(missing.to_string().contains("expected \"Log In\" after a restart"));
    }
}
