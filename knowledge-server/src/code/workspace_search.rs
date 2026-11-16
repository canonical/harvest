use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::Value;

use super::images::quote;

pub const TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MATCHES: usize = 200;
const MAX_PER_FILE: usize = 20;
const MAX_LINE_CHARS: usize = 300;
const NO_MATCH: i64 = 1;

#[derive(Debug, PartialEq, Eq)]
pub struct Search {
    pattern: String,
    path: Option<String>,
    fixed: bool,
    ignore_case: bool,
}

impl Search {
    pub fn from_params(params: &Value) -> Result<Self> {
        let pattern = params["pattern"].as_str().unwrap_or_default();
        if pattern.is_empty() {
            bail!("a pattern is required");
        }
        Ok(Self {
            pattern: pattern.to_string(),
            path: params["path"]
                .as_str()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(String::from),
            fixed: params["fixed"].as_bool().unwrap_or(false),
            ignore_case: params["ignore_case"].as_bool().unwrap_or(false),
        })
    }

    pub fn script(&self) -> String {
        let syntax = if self.fixed { "-F" } else { "-E" };
        let case = if self.ignore_case { " -i" } else { "" };
        let scope = match &self.path {
            Some(path) => format!(" -- {}", quote(path)),
            None => String::new(),
        };
        format!(
            "git grep -n -I --untracked --no-color --max-count={MAX_PER_FILE} {syntax}{case} -e {}{scope}",
            quote(&self.pattern)
        )
    }

    pub fn report(&self, exit_code: i64, stdout: &str, stderr: &str) -> Result<String> {
        match exit_code {
            0 => Ok(self.matches(stdout)),
            NO_MATCH if stderr.trim().is_empty() => Ok(format!("No matches for {}.", self.describe())),
            _ => bail!("{}", first_line_or(stderr, "git grep failed")),
        }
    }

    fn matches(&self, stdout: &str) -> String {
        let lines: Vec<&str> = stdout.lines().collect();
        let mut out: Vec<String> = lines.iter().take(MAX_MATCHES).map(|line| clip(line)).collect();
        if lines.len() > MAX_MATCHES {
            out.push(format!(
                "Showing {MAX_MATCHES} of {} matches; narrow the search with path or a more specific pattern.",
                lines.len()
            ));
        }
        out.join("\n")
    }

    fn describe(&self) -> String {
        match &self.path {
            Some(path) => format!("'{}' in {path}", self.pattern),
            None => format!("'{}'", self.pattern),
        }
    }
}

fn clip(line: &str) -> String {
    match line.char_indices().nth(MAX_LINE_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.to_string(),
    }
}

fn first_line_or<'a>(text: &'a str, fallback: &'a str) -> &'a str {
    text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn search(params: Value) -> Search {
        Search::from_params(&params).unwrap()
    }

    #[test]
    fn a_pattern_is_required() {
        assert!(Search::from_params(&json!({ "pattern": "" })).is_err());
        assert!(Search::from_params(&json!({})).is_err());
    }

    #[test]
    fn the_script_quotes_the_pattern_and_the_path() {
        let script = search(json!({ "pattern": "a'b; rm -rf /", "path": "src/*.rs" })).script();
        assert!(script.starts_with("git grep -n -I --untracked --no-color --max-count=20 -E -e "));
        assert!(script.contains(&quote("a'b; rm -rf /")));
        assert!(script.ends_with(&format!(" -- {}", quote("src/*.rs"))));
    }

    #[test]
    fn fixed_and_case_insensitive_searches_change_the_flags() {
        let script = search(json!({ "pattern": "x", "fixed": true, "ignore_case": true })).script();
        assert!(script.contains(" -F -i -e "));
        assert!(!script.contains(" -- "));
    }

    #[test]
    fn exit_code_one_without_errors_means_no_matches() {
        let s = search(json!({ "pattern": "zzz", "path": "src" }));
        assert_eq!(s.report(1, "", "").unwrap(), "No matches for 'zzz' in src.");
    }

    #[test]
    fn git_errors_are_reported() {
        let s = search(json!({ "pattern": "(" }));
        let error = s.report(128, "", "fatal: command line, '(': Unmatched ( or \\(\n").unwrap_err();
        assert_eq!(error.to_string(), "fatal: command line, '(': Unmatched ( or \\(");
    }

    #[test]
    fn long_results_are_capped_and_long_lines_clipped() {
        let s = search(json!({ "pattern": "x" }));
        let stdout: String = (0..250).map(|i| format!("a.rs:{i}:x\n")).collect();
        let report = s.report(0, &stdout, "").unwrap();
        assert_eq!(report.lines().count(), MAX_MATCHES + 1);
        assert!(report.ends_with("Showing 200 of 250 matches; narrow the search with path or a more specific pattern."));
        let long = format!("a.rs:1:{}", "y".repeat(400));
        assert!(s.report(0, &long, "").unwrap().ends_with('…'));
    }
}
