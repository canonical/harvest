use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

const FILE_MARK: &str = "@@file ";
const DIR_MARK: &str = "@@dir ";
const TOOL_MARK: &str = "@@tool ";
const MARKER_FILES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "pyproject.toml",
    "setup.py",
    "requirements.txt",
    "requirements-dev.txt",
    ".pre-commit-config.yaml",
    "Cargo.toml",
    "rust-toolchain.toml",
    "rust-toolchain",
    "go.mod",
    "Makefile",
];
const TEST_DIRS: &[&str] = &["test", "tests"];
const TOOLS: &[&str] = &[
    "node", "npm", "pnpm", "yarn", "python3", "ruff", "mypy", "pre-commit", "cargo", "go", "make",
];
const MAKE_TEST_TARGETS: &[&str] = &["test"];
const MAKE_CHECK_TARGETS: &[&str] = &["lint", "check"];
const NPM_CHECK_SCRIPTS: &[&str] = &["lint", "typecheck", "check"];
const NPM_PLACEHOLDER_TEST: &str = "no test specified";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Probe {
    pub files: BTreeMap<String, String>,
    pub dirs: BTreeSet<String>,
    pub tools: BTreeSet<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ProjectFacts {
    pub tests: Vec<String>,
    pub checks: Vec<String>,
    pub missing_tools: Vec<String>,
}

pub fn probe_script() -> String {
    let files = MARKER_FILES.join(" ");
    let dirs = TEST_DIRS.join(" ");
    let tools = TOOLS.join(" ");
    format!(
        "for f in {files}; do if [ -f \"$f\" ]; then echo \"{FILE_MARK}$f\"; head -c 20000 \"$f\"; echo; fi; done\n\
         for d in {dirs}; do if [ -d \"$d\" ]; then echo \"{DIR_MARK}$d\"; fi; done\n\
         for t in {tools}; do if command -v \"$t\" >/dev/null 2>&1; then echo \"{TOOL_MARK}$t\"; fi; done\n\
         if python3 -c 'import pytest' >/dev/null 2>&1; then echo \"{TOOL_MARK}pytest\"; fi\n\
         true"
    )
}

pub fn parse_probe(output: &str) -> Probe {
    let mut probe = Probe::default();
    let mut current: Option<(String, Vec<&str>)> = None;
    let flush = |current: &mut Option<(String, Vec<&str>)>, probe: &mut Probe| {
        if let Some((name, lines)) = current.take() {
            probe.files.insert(name, lines.join("\n"));
        }
    };
    for line in output.lines() {
        if let Some(name) = line.strip_prefix(FILE_MARK) {
            flush(&mut current, &mut probe);
            current = Some((name.to_string(), Vec::new()));
        } else if let Some(dir) = line.strip_prefix(DIR_MARK) {
            flush(&mut current, &mut probe);
            probe.dirs.insert(dir.to_string());
        } else if let Some(tool) = line.strip_prefix(TOOL_MARK) {
            flush(&mut current, &mut probe);
            probe.tools.insert(tool.to_string());
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    flush(&mut current, &mut probe);
    probe
}

pub fn detect(probe: &Probe) -> ProjectFacts {
    let mut facts = ProjectFacts::default();
    if let Some(manifest) = probe.files.get("package.json") {
        node_commands(manifest, package_runner(probe), &probe.tools, &mut facts);
    }
    if let Some(pyproject) = probe.files.get("pyproject.toml") {
        python_commands(pyproject, probe, &mut facts);
    }
    if probe.files.contains_key(".pre-commit-config.yaml") {
        facts.checks.push("pre-commit run --all-files".into());
    }
    if probe.files.contains_key("Cargo.toml") {
        facts.tests.push("cargo test".into());
        facts.checks.push("cargo fmt --check".into());
        facts.checks.push("cargo clippy --all-targets".into());
    }
    if probe.files.contains_key("go.mod") {
        facts.tests.push("go test ./...".into());
        facts.checks.push("go vet ./...".into());
    }
    if let Some(makefile) = probe.files.get("Makefile") {
        make_commands(makefile, &mut facts);
    }
    facts.missing_tools = missing_tools(&facts, &probe.tools);
    facts
}

pub fn render(facts: &ProjectFacts) -> Option<String> {
    if facts.tests.is_empty() && facts.checks.is_empty() {
        return None;
    }
    let mut lines = vec!["## This repository's commands".to_string()];
    if !facts.tests.is_empty() {
        lines.push(format!("Tests: {}.", backticked(&facts.tests)));
    }
    if !facts.checks.is_empty() {
        lines.push(format!(
            "Checks (formatting, linting, types): {}. Run them with the tests before every commit; CI runs them too.",
            backticked(&facts.checks)
        ));
    }
    if !facts.missing_tools.is_empty() {
        lines.push(format!(
            "Not installed in this container yet: {}. Install what you need once and record it with save_setup_script.",
            backticked(&facts.missing_tools)
        ));
    }
    Some(lines.join("\n"))
}

fn backticked(items: &[String]) -> String {
    items.iter().map(|i| format!("`{i}`")).collect::<Vec<_>>().join(", ")
}

fn package_runner(probe: &Probe) -> &'static str {
    if probe.files.contains_key("pnpm-lock.yaml") {
        "pnpm"
    } else if probe.files.contains_key("yarn.lock") {
        "yarn"
    } else {
        "npm"
    }
}

fn node_commands(manifest: &str, runner: &str, installed: &BTreeSet<String>, facts: &mut ProjectFacts) {
    let Ok(json) = serde_json::from_str::<Value>(manifest) else {
        return;
    };
    let scripts = &json["scripts"];
    let command = |name: &str, via_runner: String| {
        let body = scripts[name].as_str().filter(|body| !body.contains(NPM_PLACEHOLDER_TEST))?;
        let direct = body.split_whitespace().next().is_some_and(|tool| installed.contains(tool));
        Some(if installed.contains(runner) || !direct { via_runner } else { body.to_string() })
    };
    if let Some(test) = command("test", format!("{runner} test")) {
        facts.tests.push(test);
    }
    for name in NPM_CHECK_SCRIPTS {
        if let Some(check) = command(name, format!("{runner} run {name}")) {
            facts.checks.push(check);
        }
    }
}

fn python_commands(pyproject: &str, probe: &Probe, facts: &mut ProjectFacts) {
    let tool = pyproject
        .parse::<toml::Table>()
        .ok()
        .and_then(|t| t.get("tool").and_then(|v| v.as_table()).cloned())
        .unwrap_or_default();
    let has_tests = tool.contains_key("pytest") || TEST_DIRS.iter().any(|d| probe.dirs.contains(*d));
    if has_tests {
        facts.tests.push("python3 -m pytest".into());
    }
    let uses_pre_commit = probe.files.contains_key(".pre-commit-config.yaml");
    if tool.contains_key("ruff") && !uses_pre_commit {
        facts.checks.push("ruff format --check .".into());
        facts.checks.push("ruff check .".into());
    }
    if tool.contains_key("mypy") {
        facts.checks.push("mypy".into());
    }
}

fn make_commands(makefile: &str, facts: &mut ProjectFacts) {
    let targets: BTreeSet<&str> = makefile
        .lines()
        .filter_map(|line| line.split_once(':').map(|(target, _)| target.trim()))
        .filter(|target| !target.is_empty() && !target.contains(char::is_whitespace))
        .collect();
    for target in MAKE_TEST_TARGETS.iter().filter(|t| targets.contains(*t)) {
        facts.tests.push(format!("make {target}"));
    }
    for target in MAKE_CHECK_TARGETS.iter().filter(|t| targets.contains(*t)) {
        facts.checks.push(format!("make {target}"));
    }
}

fn missing_tools(facts: &ProjectFacts, installed: &BTreeSet<String>) -> Vec<String> {
    let needed: BTreeSet<String> = facts
        .tests
        .iter()
        .chain(&facts.checks)
        .filter_map(|command| tool_of(command))
        .collect();
    needed.into_iter().filter(|tool| !installed.contains(tool)).collect()
}

fn tool_of(command: &str) -> Option<String> {
    let mut words = command.split_whitespace();
    let first = words.next()?;
    match (first, words.next(), words.next()) {
        ("python3", Some("-m"), Some(module)) => Some(module.to_string()),
        _ => Some(first.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(files: &[(&str, &str)], dirs: &[&str], tools: &[&str]) -> Probe {
        Probe {
            files: files.iter().map(|(n, c)| (n.to_string(), c.to_string())).collect(),
            dirs: dirs.iter().map(|d| d.to_string()).collect(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn the_probe_output_is_split_into_files_dirs_and_tools() {
        let output = "@@file package.json\n{\n  \"name\": \"a\"\n}\n@@file go.mod\nmodule x\n\n@@dir tests\n@@tool node\n@@tool pytest\n";
        let parsed = parse_probe(output);
        assert_eq!(parsed.files["package.json"], "{\n  \"name\": \"a\"\n}");
        assert_eq!(parsed.files["go.mod"], "module x\n");
        assert!(parsed.dirs.contains("tests"));
        assert_eq!(parsed.tools.len(), 2);
    }

    #[test]
    fn npm_scripts_become_commands_and_a_missing_npm_is_reported() {
        let manifest = r#"{"scripts": {"test": "vitest", "lint": "eslint ."}}"#;
        let facts = detect(&probe(&[("package.json", manifest)], &[], &["node"]));
        assert_eq!(facts.tests, ["npm test"]);
        assert_eq!(facts.checks, ["npm run lint"]);
        assert_eq!(facts.missing_tools, ["npm"]);
    }

    #[test]
    fn without_npm_a_script_that_only_needs_node_runs_directly() {
        let manifest = r#"{"scripts": {"test": "node --test", "lint": "eslint ."}}"#;
        let facts = detect(&probe(&[("package.json", manifest)], &[], &["node"]));
        assert_eq!(facts.tests, ["node --test"]);
        assert_eq!(facts.checks, ["npm run lint"]);
        assert_eq!(facts.missing_tools, ["npm"]);
    }

    #[test]
    fn the_npm_placeholder_test_script_is_ignored() {
        let manifest = r#"{"scripts": {"test": "echo \"Error: no test specified\" && exit 1"}}"#;
        assert_eq!(detect(&probe(&[("package.json", manifest)], &[], &[])), ProjectFacts::default());
    }

    #[test]
    fn the_lockfile_picks_the_package_runner() {
        let manifest = r#"{"scripts": {"test": "vitest"}}"#;
        let facts = detect(&probe(&[("package.json", manifest), ("pnpm-lock.yaml", "")], &[], &["pnpm"]));
        assert_eq!(facts.tests, ["pnpm test"]);
        assert!(facts.missing_tools.is_empty());
    }

    #[test]
    fn pre_commit_replaces_individual_python_linters() {
        let pyproject = "[tool.ruff]\nline-length = 88\n[tool.pytest.ini_options]\ntestpaths = [\"tests\"]\n";
        let facts = detect(&probe(
            &[("pyproject.toml", pyproject), (".pre-commit-config.yaml", "repos: []")],
            &[],
            &["python3"],
        ));
        assert_eq!(facts.tests, ["python3 -m pytest"]);
        assert_eq!(facts.checks, ["pre-commit run --all-files"]);
        assert_eq!(facts.missing_tools, ["pre-commit", "pytest"]);
    }

    #[test]
    fn ruff_runs_directly_without_pre_commit() {
        let facts = detect(&probe(&[("pyproject.toml", "[tool.ruff]\n")], &["tests"], &["python3", "pytest", "ruff"]));
        assert_eq!(facts.checks, ["ruff format --check .", "ruff check ."]);
        assert!(facts.missing_tools.is_empty());
    }

    #[test]
    fn makefile_targets_rust_and_go_are_detected() {
        let facts = detect(&probe(
            &[("Makefile", "test:\n\tgo test ./...\nlint: deps\n\tgolangci-lint run\n"), ("go.mod", "module x")],
            &[],
            &["go", "make"],
        ));
        assert_eq!(facts.tests, ["go test ./...", "make test"]);
        assert_eq!(facts.checks, ["go vet ./...", "make lint"]);
    }

    #[test]
    fn nothing_detected_renders_nothing() {
        assert_eq!(render(&ProjectFacts::default()), None);
    }

    #[test]
    fn the_rendering_names_commands_and_missing_tools() {
        let facts = ProjectFacts {
            tests: vec!["npm test".into()],
            checks: vec!["npm run lint".into()],
            missing_tools: vec!["npm".into()],
        };
        let text = render(&facts).unwrap();
        assert!(text.contains("Tests: `npm test`."));
        assert!(text.contains("`npm run lint`"));
        assert!(text.contains("Not installed in this container yet: `npm`"));
    }
}
