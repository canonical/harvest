use serde_json::{json, Value};

use super::images::quote;
use super::workspace::workspace_path;
use super::{sessions, CodeEnv, WORKSPACE};

const FILE_MARK: &str = "@@working-set ";
const FILE_TOOLS: &[&str] = &["read_file", "write_file", "replace_in_file"];
const MAX_FILES: usize = 12;

#[derive(Debug, PartialEq, Eq)]
pub struct WorkingFile {
    pub path: String,
    pub size: usize,
    pub content: String,
}

pub async fn for_chat(env: &CodeEnv, session_id: &str, chat_id: &str) -> Option<String> {
    let budget = env.settings.working_set_chars;
    if budget == 0 || chat_id.is_empty() {
        return None;
    }
    let chain = last_answer_chain(env, chat_id).await?;
    let paths = touched_files(&chain);
    if paths.is_empty() {
        return None;
    }
    let container = sessions::active_container(env, session_id).await.ok()?;
    match sessions::run_checked(env, &container, &read_script(&paths, budget)).await {
        Ok(output) => render(&parse(&output), budget),
        Err(e) => {
            tracing::warn!(session = session_id, error = %e, "could not read the previous turn's files");
            None
        }
    }
}

async fn last_answer_chain(env: &CodeEnv, chat_id: &str) -> Option<Vec<Value>> {
    let rows = env
        .db
        .query(
            "SELECT payload->'chain' AS chain FROM conversation_messages
             WHERE conv_id = $cid AND payload->>'role' = 'assistant'
             ORDER BY seq DESC LIMIT 1",
            json!({ "cid": chat_id }),
        )
        .await
        .map_err(|e| tracing::warn!(chat = chat_id, error = %e, "could not load the previous answer"))
        .ok()?;
    rows.into_iter().next()?["chain"].as_array().cloned()
}

pub fn touched_files(chain: &[Value]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for step in chain.iter().rev() {
        let is_file_tool = step["name"].as_str().is_some_and(|name| FILE_TOOLS.contains(&name));
        let Some(path) = step["input"]["path"].as_str().filter(|_| is_file_tool) else {
            continue;
        };
        let Ok(absolute) = workspace_path(path) else {
            continue;
        };
        if !paths.contains(&absolute) {
            paths.push(absolute);
        }
    }
    paths.truncate(MAX_FILES);
    paths
}

pub fn read_script(paths: &[String], limit: usize) -> String {
    let mut script: Vec<String> = paths
        .iter()
        .map(|path| {
            let p = quote(path);
            format!("if [ -f {p} ]; then echo \"{FILE_MARK}$(wc -c < {p}) \"{p}; head -c {limit} {p}; echo; fi")
        })
        .collect();
    script.push("true".into());
    script.join("\n")
}

pub fn parse(output: &str) -> Vec<WorkingFile> {
    let mut files: Vec<WorkingFile> = Vec::new();
    for line in output.split_inclusive('\n') {
        if let Some(header) = line.strip_prefix(FILE_MARK) {
            let (size, path) = header.trim_end().split_once(' ').unwrap_or(("0", ""));
            files.push(WorkingFile {
                path: path.to_string(),
                size: size.parse().unwrap_or(0),
                content: String::new(),
            });
        } else if let Some(file) = files.last_mut() {
            file.content.push_str(line);
        }
    }
    for file in &mut files {
        if file.content.ends_with('\n') {
            file.content.pop();
        }
    }
    files
}

pub fn render(files: &[WorkingFile], budget: usize) -> Option<String> {
    let mut remaining = budget;
    let mut shown = Vec::new();
    let mut left_out = Vec::new();
    for file in files {
        if remaining == 0 {
            left_out.push(display_path(&file.path));
            continue;
        }
        let excerpt: String = file.content.chars().take(remaining).collect();
        remaining -= excerpt.chars().count();
        let partial = excerpt.len() < file.size;
        let note = if partial {
            format!(" (first {} of {} bytes; read_file for the rest)", excerpt.len(), file.size)
        } else {
            String::new()
        };
        shown.push(format!("### {}{note}\n```\n{excerpt}\n```", display_path(&file.path)));
    }
    if shown.is_empty() {
        return None;
    }
    let mut text = vec![
        "## Files from your previous turn\nTheir current contents, read just now: use them instead of reading these files again. Read a file again only after changing it with run_command.".to_string(),
    ];
    text.extend(shown);
    if !left_out.is_empty() {
        text.push(format!("Also touched, not shown: {}.", left_out.join(", ")));
    }
    Some(text.join("\n\n"))
}

fn display_path(path: &str) -> String {
    path.strip_prefix(WORKSPACE)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(name: &str, path: &str) -> Value {
        json!({ "type": "tool", "name": name, "input": { "path": path } })
    }

    #[test]
    fn files_read_or_edited_last_are_listed_first_once() {
        let chain = vec![
            step("read_file", "src/app.js"),
            step("run_command", "ignored"),
            step("read_file", "src/board.js"),
            step("replace_in_file", "src/app.js"),
            json!({ "type": "thinking", "text": "…" }),
        ];
        assert_eq!(touched_files(&chain), ["/workspace/src/app.js", "/workspace/src/board.js"]);
    }

    #[test]
    fn the_read_script_lists_sizes_and_quotes_paths() {
        let script = read_script(&["/workspace/a b.js".into()], 100);
        assert!(script.contains("wc -c < '/workspace/a b.js'"));
        assert!(script.contains("head -c 100 '/workspace/a b.js'"));
    }

    #[test]
    fn parsed_files_keep_their_exact_content() {
        let output = "@@working-set 12 /workspace/src/a.js\nconst a = 1;\n@@working-set 3 /workspace/b.txt\nhi\n\n";
        let files = parse(output);
        assert_eq!(files[0], WorkingFile { path: "/workspace/src/a.js".into(), size: 12, content: "const a = 1;".into() });
        assert_eq!(files[1].content, "hi\n");
    }

    #[test]
    fn the_budget_truncates_and_then_leaves_files_out() {
        let files = vec![
            WorkingFile { path: "/workspace/a.js".into(), size: 10, content: "0123456789".into() },
            WorkingFile { path: "/workspace/b.js".into(), size: 10, content: "abcdefghij".into() },
            WorkingFile { path: "/workspace/c.js".into(), size: 3, content: "xyz".into() },
        ];
        let text = render(&files, 14).unwrap();
        assert!(text.contains("### a.js\n```\n0123456789\n```"));
        assert!(text.contains("### b.js (first 4 of 10 bytes; read_file for the rest)\n```\nabcd\n```"));
        assert!(text.ends_with("Also touched, not shown: c.js."));
    }

    #[test]
    fn nothing_to_show_renders_nothing() {
        assert_eq!(render(&[], 100), None);
    }
}
