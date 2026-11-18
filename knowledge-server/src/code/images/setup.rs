use anyhow::Result;

use super::state::SETUP_TIMEOUT;
use super::{tail, REPO_SETUP_FILE};
use crate::code::projects::CodeProject;
use crate::code::{preparation, CodeEnv, WORKSPACE};
use crate::lxd::containers::Command;

const LOG_TAIL_CHARS: usize = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Toolchain,
    Repository,
    Project,
}

impl Layer {
    fn file_name(self) -> &'static str {
        match self {
            Self::Toolchain => "toolchain",
            Self::Repository => "repository",
            Self::Project => "project",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Toolchain => "The automatic toolchain setup",
            Self::Repository => "The repository's .harvest/setup.sh",
            Self::Project => "The project's setup script",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub layer: Layer,
    pub script: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepFailure {
    pub layer: Layer,
    pub exit_code: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupReport {
    pub log: String,
    pub warning: Option<String>,
}

pub async fn plan_for(env: &CodeEnv, project: &CodeProject, container: &str) -> Result<Vec<Step>> {
    let toolchain = preparation::script_for(env, container)
        .await?
        .map(|script| Step { layer: Layer::Toolchain, script });
    let custom = custom_step(env, project, container).await?;
    Ok(toolchain.into_iter().chain(custom).collect())
}

async fn custom_step(env: &CodeEnv, project: &CodeProject, container: &str) -> Result<Option<Step>> {
    let from_repository = env
        .lxd
        .run(container, Command::shell(&format!("cat {REPO_SETUP_FILE}")).in_dir(WORKSPACE))
        .await?;
    if from_repository.succeeded() && !from_repository.stdout.trim().is_empty() {
        return Ok(Some(Step { layer: Layer::Repository, script: from_repository.stdout }));
    }
    Ok(project
        .setup_script
        .clone()
        .filter(|script| !script.trim().is_empty())
        .map(|script| Step { layer: Layer::Project, script }))
}

pub async fn run(env: &CodeEnv, container: &str, steps: &[Step]) -> Result<SetupReport> {
    let mut log = String::new();
    let mut failures = Vec::new();
    for step in steps {
        let path = format!("/tmp/harvest-setup-{}.sh", step.layer.file_name());
        env.lxd.push_file(container, &path, step.script.as_bytes().to_vec(), 0o700).await?;
        let output = env
            .lxd
            .run(container, Command::shell(&format!("bash {path}")).in_dir(WORKSPACE).timeout(SETUP_TIMEOUT))
            .await?;
        log.push_str(&format!("== {}\n{}{}\n", step.layer.description(), output.stdout, output.stderr));
        if !output.succeeded() {
            failures.push(StepFailure { layer: step.layer, exit_code: output.exit_code });
        }
    }
    Ok(SetupReport { log: tail(&log, LOG_TAIL_CHARS), warning: warning_for(&failures) })
}

pub fn warning_for(failures: &[StepFailure]) -> Option<String> {
    let sentences: Vec<String> = failures
        .iter()
        .map(|failure| format!("{} failed with exit code {}.", failure.layer.description(), failure.exit_code))
        .collect();
    (!sentences.is_empty()).then(|| {
        format!(
            "{} Containers start without the steps that did not finish; the log shows where it stopped.",
            sentences.join(" ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_setup_has_no_warning() {
        assert_eq!(warning_for(&[]), None);
    }

    #[test]
    fn the_warning_names_each_layer_that_failed() {
        let warning = warning_for(&[
            StepFailure { layer: Layer::Toolchain, exit_code: 100 },
            StepFailure { layer: Layer::Project, exit_code: 1 },
        ])
        .unwrap();
        assert!(warning.starts_with("The automatic toolchain setup failed with exit code 100. The project's setup script failed with exit code 1."));
    }
}
