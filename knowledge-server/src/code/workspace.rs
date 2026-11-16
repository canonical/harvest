use std::time::Duration;

use anyhow::{bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::images::{quote, tail};
use super::{projects, sessions, CodeEnv, WORKSPACE};
use crate::agent::tool::Tool;
use crate::llm::types::ToolDefinition;
use crate::lxd::containers::Command;
use crate::toolsets::{Toolset, TurnContext};

const OUTPUT_CHARS: usize = 12_000;
const READ_MAX_LINES: usize = 400;
const DEFAULT_TIMEOUT_SECS: u64 = 300;
const MAX_TIMEOUT_SECS: u64 = 1_800;

pub fn workspace_path(path: &str) -> Result<String> {
    let path = path.trim();
    if path.is_empty() {
        bail!("a path is required");
    }
    if path.starts_with('/') {
        Ok(path.to_string())
    } else {
        Ok(format!("{WORKSPACE}/{}", path.trim_start_matches("./")))
    }
}

pub fn numbered(text: &str, start: usize, end: Option<usize>) -> String {
    let first = start.max(1);
    let last = end.unwrap_or(first + READ_MAX_LINES - 1).min(first + READ_MAX_LINES - 1);
    text.lines()
        .enumerate()
        .skip(first - 1)
        .take_while(|(i, _)| *i < last)
        .map(|(i, line)| format!("{:>6}  {line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn replace_once(text: &str, old: &str, new: &str) -> Result<String> {
    match text.matches(old).count() {
        0 => bail!("the text to replace was not found; read the file again and copy it exactly"),
        1 => Ok(text.replacen(old, new, 1)),
        n => bail!("the text to replace appears {n} times; include more surrounding lines so it is unique"),
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Run,
    Read,
    Write,
    Replace,
    List,
    Changes,
    SaveSetup,
}

const OPERATIONS: [Operation; 7] = [
    Operation::Run,
    Operation::Read,
    Operation::Write,
    Operation::Replace,
    Operation::List,
    Operation::Changes,
    Operation::SaveSetup,
];

impl Operation {
    fn definition(self) -> ToolDefinition {
        let (name, description, parameters) = match self {
            Self::Run => (
                "run_command",
                "Run a shell command in the session's container, from the repository root. Use it to build, run tests, install packages and inspect the repository with git.",
                json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                        "timeout_seconds": { "type": "integer", "description": "Defaults to 300, at most 1800" }
                    },
                    "required": ["command"]
                }),
            ),
            Self::Read => (
                "read_file",
                "Read a file from the repository with line numbers. Paths are relative to the repository root.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "start_line": { "type": "integer" },
                        "end_line": { "type": "integer" }
                    },
                    "required": ["path"]
                }),
            ),
            Self::Write => (
                "write_file",
                "Create or overwrite a file in the repository with the given content.",
                json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" }, "content": { "type": "string" } },
                    "required": ["path", "content"]
                }),
            ),
            Self::Replace => (
                "replace_in_file",
                "Replace one exact, unique piece of text in a file. Prefer this over rewriting whole files.",
                json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_text": { "type": "string" },
                        "new_text": { "type": "string" }
                    },
                    "required": ["path", "old_text", "new_text"]
                }),
            ),
            Self::List => (
                "list_files",
                "List files tracked in the repository, optionally under a directory.",
                json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": []
                }),
            ),
            Self::Changes => (
                "show_changes",
                "Show the session's changes against its base branch: status, summary and diff.",
                json!({ "type": "object", "properties": {}, "required": [] }),
            ),
            Self::SaveSetup => (
                "save_setup_script",
                "Save the shell script that installs this repository's dependencies, once it has worked in this container. Harvest runs it when preparing new containers for the project, unless the repository has its own .harvest/setup.sh.",
                json!({
                    "type": "object",
                    "properties": { "script": { "type": "string" } },
                    "required": ["script"]
                }),
            ),
        };
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

struct WorkspaceTool {
    env: CodeEnv,
    session_id: String,
    operation: Operation,
}

impl WorkspaceTool {
    async fn container(&self) -> Result<String> {
        let container = sessions::active_container(&self.env, &self.session_id).await?;
        sessions::touch(&self.env, &self.session_id).await?;
        Ok(container)
    }

    async fn shell(&self, container: &str, script: &str, timeout: Duration) -> Result<String> {
        let output = self
            .env
            .lxd
            .run(container, Command::shell(script).in_dir(WORKSPACE).timeout(timeout))
            .await?;
        let mut text = format!("exit code {}\n", output.exit_code);
        if !output.stdout.is_empty() {
            text.push_str(&tail(&output.stdout, OUTPUT_CHARS));
        }
        if !output.stderr.is_empty() {
            text.push_str("\n--- stderr ---\n");
            text.push_str(&tail(&output.stderr, OUTPUT_CHARS / 2));
        }
        Ok(text)
    }

    async fn perform(&self, params: &Value) -> Result<String> {
        let text = |key: &str| params[key].as_str().unwrap_or_default();
        if let Operation::SaveSetup = self.operation {
            let session = sessions::get(&self.env, &self.session_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("the session no longer exists"))?;
            projects::set_setup_script(&self.env.db, &session.project_id, Some(text("script"))).await?;
            return Ok("Saved. New containers for this project will run this script.".into());
        }
        let container = self.container().await?;
        let lxd = &self.env.lxd;
        match self.operation {
            Operation::Run => {
                let secs = params["timeout_seconds"]
                    .as_u64()
                    .unwrap_or(DEFAULT_TIMEOUT_SECS)
                    .clamp(1, MAX_TIMEOUT_SECS);
                self.shell(&container, text("command"), Duration::from_secs(secs)).await
            }
            Operation::Read => {
                let bytes = lxd.pull_file(&container, &workspace_path(text("path"))?).await?;
                let content = String::from_utf8_lossy(&bytes);
                let start = params["start_line"].as_u64().unwrap_or(1) as usize;
                let end = params["end_line"].as_u64().map(|n| n as usize);
                Ok(numbered(&content, start, end))
            }
            Operation::Write => {
                let path = workspace_path(text("path"))?;
                self.shell(&container, &format!("mkdir -p \"$(dirname {})\"", quote(&path)), Duration::from_secs(30))
                    .await?;
                lxd.push_file(&container, &path, text("content").as_bytes().to_vec(), 0o644)
                    .await?;
                Ok(format!("Wrote {path}."))
            }
            Operation::Replace => {
                let path = workspace_path(text("path"))?;
                let current = String::from_utf8_lossy(&lxd.pull_file(&container, &path).await?).into_owned();
                let updated = replace_once(&current, text("old_text"), text("new_text"))?;
                lxd.push_file(&container, &path, updated.into_bytes(), 0o644).await?;
                Ok(format!("Updated {path}."))
            }
            Operation::List => {
                let scope = match text("path") {
                    "" => String::new(),
                    path => format!(" -- {}", quote(path)),
                };
                self.shell(&container, &format!("git ls-files{scope} | head -n 500"), Duration::from_secs(60))
                    .await
            }
            Operation::Changes => {
                let session = sessions::get(&self.env, &self.session_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("the session no longer exists"))?;
                let base = quote(&format!("refs/remotes/origin/{}", session.base_branch));
                let script = format!(
                    "git status --short && echo && git diff --stat {base} && echo && git diff {base}"
                );
                self.shell(&container, &script, Duration::from_secs(60)).await
            }
            Operation::SaveSetup => unreachable!(),
        }
    }
}

#[async_trait]
impl Tool for WorkspaceTool {
    fn definition(&self) -> ToolDefinition {
        self.operation.definition()
    }

    async fn execute(&self, params: Value) -> Result<String> {
        match self.perform(&params).await {
            Ok(text) => Ok(text),
            Err(e) => Ok(format!("error: {e}")),
        }
    }

    fn reads_code(&self) -> bool {
        matches!(self.operation, Operation::Read)
    }
}

pub struct WorkspaceToolset {
    pub env: CodeEnv,
}

impl Toolset for WorkspaceToolset {
    fn tools(&self, turn: &TurnContext) -> Vec<Box<dyn Tool>> {
        let Some(session_id) = turn.space.code_session_id() else {
            return Vec::new();
        };
        OPERATIONS
            .iter()
            .map(|operation| {
                Box::new(WorkspaceTool {
                    env: self.env.clone(),
                    session_id: session_id.to_string(),
                    operation: *operation,
                }) as Box<dyn Tool>
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_inside_the_repository() {
        assert_eq!(workspace_path("src/main.rs").unwrap(), "/workspace/src/main.rs");
        assert_eq!(workspace_path("./README.md").unwrap(), "/workspace/README.md");
        assert_eq!(workspace_path("/etc/hosts").unwrap(), "/etc/hosts");
        assert!(workspace_path("  ").is_err());
    }

    #[test]
    fn reading_numbers_lines_from_the_requested_start() {
        let text = "a\nb\nc\nd";
        assert_eq!(numbered(text, 2, Some(3)), "     2  b\n     3  c");
        assert_eq!(numbered(text, 1, None).lines().count(), 4);
    }

    #[test]
    fn a_replacement_must_match_exactly_once() {
        assert_eq!(replace_once("let x = 1;", "1", "2").unwrap(), "let x = 2;");
        assert!(replace_once("a a", "a", "b").is_err());
        assert!(replace_once("abc", "z", "y").is_err());
    }

    #[test]
    fn every_operation_has_a_distinct_tool_name() {
        let mut names: Vec<_> = OPERATIONS.iter().map(|o| o.definition().name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), OPERATIONS.len());
    }
}
