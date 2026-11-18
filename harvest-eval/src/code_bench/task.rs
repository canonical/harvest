use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Task {
    pub name: String,
    pub kind: String,
    pub repository: String,
    #[serde(default = "default_base")]
    pub base: String,
    #[serde(default)]
    pub setup: Vec<String>,
    pub prompts: Vec<String>,
    #[serde(default)]
    pub acceptance: Option<Acceptance>,
    #[serde(default)]
    pub checks: Option<Checks>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Checks {
    #[serde(default)]
    pub prepare: Vec<String>,
    pub commands: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Acceptance {
    #[serde(default)]
    pub prepare: Vec<String>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    pub command: String,
}

fn default_base() -> String {
    "main".into()
}

impl Task {
    pub fn parse(text: &str) -> Result<Self> {
        let task: Task = toml::from_str(text)?;
        if task.prompts.is_empty() {
            bail!("task {} has no prompts", task.name);
        }
        Ok(task)
    }
}

pub fn load_dir(dir: &Path, only: Option<&str>) -> Result<Vec<Task>> {
    let mut tasks = Vec::new();
    for (path, text) in crate::toml_dir::read(dir)? {
        let task = Task::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        if only.is_none_or(|name| name == task.name) {
            tasks.push(task);
        }
    }
    if tasks.is_empty() {
        bail!("no matching tasks in {}", dir.display());
    }
    Ok(tasks)
}

fn on_a_copy(copy_dir: &str, prepare: &[String]) -> Vec<String> {
    let mut lines = vec![
        "set -e".to_string(),
        format!("rm -rf {copy_dir}"),
        format!("cp -a /workspace {copy_dir}"),
        format!("cd {copy_dir}"),
    ];
    lines.extend(prepare.iter().cloned());
    lines
}

pub fn checks_script(checks: &Checks, copy_dir: &str) -> String {
    let mut lines = on_a_copy(copy_dir, &checks.prepare);
    lines.extend(checks.commands.iter().cloned());
    lines.join("\n")
}

pub fn acceptance_script(acceptance: &Acceptance, copy_dir: &str) -> String {
    let mut lines = on_a_copy(copy_dir, &acceptance.prepare);
    for (path, content) in &acceptance.files {
        lines.push(format!("mkdir -p \"$(dirname '{path}')\""));
        lines.push(format!("cat > '{path}' <<'HARVEST_BENCH_FILE'\n{content}\nHARVEST_BENCH_FILE"));
    }
    lines.push(acceptance.command.clone());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = r#"
name = "tweak"
kind = "tweak"
repository = "https://example.com/r.git"
prompts = ["Change it."]

[acceptance]
prepare = ["pip install pytest"]
command = "pytest -q tests/test_hidden.py"
[acceptance.files]
"tests/test_hidden.py" = "def test_x():\n    assert True"
"#;

    #[test]
    fn a_task_parses_with_defaults() {
        let task = Task::parse(TASK).unwrap();
        assert_eq!(task.base, "main");
        assert!(task.setup.is_empty());
        assert_eq!(task.acceptance.unwrap().files.len(), 1);
    }

    #[test]
    fn the_bundled_tasks_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tasks/code");
        let tasks = load_dir(&dir, None).unwrap();
        assert!(tasks.len() >= 4);
        assert!(tasks.iter().all(|t| t.acceptance.is_some()));
    }

    #[test]
    fn a_task_needs_a_prompt() {
        let text = TASK.replace("prompts = [\"Change it.\"]", "prompts = []");
        assert!(Task::parse(&text).is_err());
    }

    #[test]
    fn checks_run_every_command_on_a_copy() {
        let checks = Checks { prepare: vec!["pip install ruff".into()], commands: vec!["ruff check .".into(), "ruff format --check .".into()] };
        let script = checks_script(&checks, "/tmp/checks");
        assert!(script.contains("cp -a /workspace /tmp/checks\ncd /tmp/checks\npip install ruff\nruff check .\nruff format --check ."));
    }

    #[test]
    fn acceptance_runs_on_a_copy_with_the_hidden_files() {
        let task = Task::parse(TASK).unwrap();
        let script = acceptance_script(task.acceptance.as_ref().unwrap(), "/tmp/accept");
        assert!(script.starts_with("set -e\nrm -rf /tmp/accept\ncp -a /workspace /tmp/accept\ncd /tmp/accept\npip install pytest"));
        assert!(script.contains("cat > 'tests/test_hidden.py' <<'HARVEST_BENCH_FILE'\ndef test_x():\n    assert True\nHARVEST_BENCH_FILE"));
        assert!(script.ends_with("pytest -q tests/test_hidden.py"));
    }
}
