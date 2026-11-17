use anyhow::{Context, Result};

use super::images::quote;
use super::jobs::shell::{self, Spec};
use super::jobs::{Job, Kind};
use super::{preparation, projects, sessions, CodeEnv, WORKSPACE};
use crate::lxd::containers::Command;

pub const REPO_CHECKS_FILE: &str = ".harvest/checks";
const COPY_DIR: &str = "/tmp/harvest-checks";
const LABEL_CHARS: usize = 80;

#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    Repository,
    Project,
    Detected,
}

#[derive(Debug, PartialEq, Eq)]
pub struct DefinitionOfDone {
    pub source: Source,
    pub commands: Vec<String>,
}

pub fn definition_of_done(
    repository_file: Option<&str>,
    project_setting: Option<&str>,
    detected: impl IntoIterator<Item = String>,
) -> DefinitionOfDone {
    let lines = |text: &str| -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(String::from)
            .collect()
    };
    if let Some(commands) = repository_file.map(lines).filter(|c| !c.is_empty()) {
        return DefinitionOfDone { source: Source::Repository, commands };
    }
    if let Some(commands) = project_setting.map(lines).filter(|c| !c.is_empty()) {
        return DefinitionOfDone { source: Source::Project, commands };
    }
    DefinitionOfDone {
        source: Source::Detected,
        commands: detected.into_iter().collect(),
    }
}

pub fn label(commands: &[String]) -> String {
    let joined = commands.join(", ");
    if joined.chars().count() <= LABEL_CHARS {
        return joined;
    }
    format!("{}…", joined.chars().take(LABEL_CHARS).collect::<String>())
}

pub fn checks_script(commands: &[String]) -> String {
    let mut lines = vec![
        format!("rm -rf {COPY_DIR} && cp -a {WORKSPACE} {COPY_DIR} && cd {COPY_DIR} || exit 1"),
        "failed=0".to_string(),
    ];
    for command in commands {
        lines.push(format!("echo {}", quote(&format!("$ {command}"))));
        lines.push(format!(
            "if ( {command} ); then echo '-> passed'; else echo '-> failed'; failed=1; fi"
        ));
    }
    lines.push("exit $failed".into());
    lines.join("\n")
}

pub async fn definition_for(env: &CodeEnv, session_id: &str) -> Result<DefinitionOfDone> {
    let session = sessions::get(env, session_id).await?.context("the session no longer exists")?;
    let project = projects::get(&env.db, &session.project_id)
        .await?
        .context("the project no longer exists")?;
    let container = sessions::active_container(env, session_id).await?;
    let repository_file = env
        .lxd
        .run(&container, Command::shell(&format!("cat {REPO_CHECKS_FILE} 2>/dev/null")).in_dir(WORKSPACE))
        .await?
        .stdout;
    let (_, facts) = preparation::probe(env, &container).await?;
    Ok(definition_of_done(
        Some(repository_file.as_str()),
        project.checks.as_deref(),
        facts.tests.into_iter().chain(facts.checks),
    ))
}

pub async fn start(env: &CodeEnv, session_id: &str, chat_id: Option<&str>) -> Result<Option<Job>> {
    let definition = definition_for(env, session_id).await?;
    if definition.commands.is_empty() {
        return Ok(None);
    }
    let job = shell::start(
        env,
        Spec {
            session_id,
            chat_id,
            kind: Kind::Checks,
            label: &label(&definition.commands),
            script: &checks_script(&definition.commands),
        },
    )
    .await?;
    Ok(Some(job))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detected() -> Vec<String> {
        vec!["npm test".into()]
    }

    #[test]
    fn the_repository_file_wins_then_the_project_then_detection() {
        let repo = definition_of_done(Some("# checks\nmake lint\n\nmake test\n"), Some("pytest"), detected());
        assert_eq!(repo, DefinitionOfDone { source: Source::Repository, commands: vec!["make lint".into(), "make test".into()] });
        let project = definition_of_done(Some(""), Some("pytest -q"), detected());
        assert_eq!(project.source, Source::Project);
        assert_eq!(project.commands, ["pytest -q"]);
        let fallback = definition_of_done(None, Some("  "), detected());
        assert_eq!(fallback.source, Source::Detected);
        assert_eq!(fallback.commands, ["npm test"]);
    }

    #[test]
    fn the_label_lists_the_commands() {
        assert_eq!(label(&["npm test".into(), "npm run lint".into()]), "npm test, npm run lint");
        assert_eq!(label(&["x".repeat(100)]).chars().count(), LABEL_CHARS + 1);
    }

    #[test]
    fn every_command_runs_on_a_copy_and_any_failure_fails_the_job() {
        let script = checks_script(&["npm test".into(), "npm run lint".into()]);
        assert!(script.starts_with("rm -rf /tmp/harvest-checks && cp -a /workspace /tmp/harvest-checks && cd /tmp/harvest-checks || exit 1"));
        assert!(script.contains("echo '$ npm test'\nif ( npm test ); then echo '-> passed'; else echo '-> failed'; failed=1; fi"));
        assert!(script.contains("if ( npm run lint ); then"));
        assert!(script.ends_with("exit $failed"));
    }
}
