use serde_json::Value;

use super::images::quote;
use super::project_facts::{Probe, ProjectFacts};

const PRELUDE: &str = "set -e
export DEBIAN_FRONTEND=noninteractive PIP_BREAK_SYSTEM_PACKAGES=1 PIP_IGNORE_INSTALLED=1 PIP_ROOT_USER_ACTION=ignore PIP_DISABLE_PIP_VERSION_CHECK=1
apt_ready=
install_packages() { if [ -z \"$apt_ready\" ]; then apt-get update -qq; apt_ready=1; fi; apt-get install -y -qq \"$@\" >/dev/null; }";
const PYTHON_TOOLS: &[&str] = &["pytest", "ruff", "mypy", "pre-commit"];
const PYTHON_EXTRAS: &[&str] = &["test", "tests", "testing", "dev"];
const PYTHON_GROUPS: &[&str] = &["test", "tests", "testing", "typing", "lint", "dev", "pre-commit"];
const PYTHON_REQUIREMENTS: &[&str] = &["requirements.txt", "requirements-dev.txt"];

pub trait Toolchain {
    fn name(&self) -> &'static str;
    fn steps(&self, probe: &Probe, facts: &ProjectFacts) -> Vec<String>;
}

pub fn toolchains() -> Vec<Box<dyn Toolchain>> {
    vec![Box::new(Python), Box::new(Node), Box::new(Rust), Box::new(Go), Box::new(Make)]
}

pub fn prepare_script(probe: &Probe, facts: &ProjectFacts) -> Option<String> {
    let sections: Vec<String> = toolchains()
        .iter()
        .filter_map(|toolchain| {
            let steps = toolchain.steps(probe, facts);
            (!steps.is_empty()).then(|| format!("echo '== {}'\n{}", toolchain.name(), steps.join("\n")))
        })
        .collect();
    (!sections.is_empty()).then(|| format!("{PRELUDE}\n{}\necho '== ready'", sections.join("\n")))
}

fn ensure_command(command: &str, packages: &str) -> String {
    format!("command -v {command} >/dev/null 2>&1 || install_packages {packages}")
}

fn uses(facts: &ProjectFacts, tool: &str) -> bool {
    facts
        .tests
        .iter()
        .chain(&facts.checks)
        .any(|command| command.split_whitespace().any(|word| word == tool))
}

pub struct Python;

impl Toolchain for Python {
    fn name(&self) -> &'static str {
        "python"
    }

    fn steps(&self, probe: &Probe, facts: &ProjectFacts) -> Vec<String> {
        let pyproject = probe.files.get("pyproject.toml").and_then(|text| text.parse::<toml::Table>().ok());
        let requirements: Vec<&str> = PYTHON_REQUIREMENTS
            .iter()
            .copied()
            .filter(|file| probe.files.contains_key(*file))
            .collect();
        let installable = probe.files.contains_key("setup.py")
            || pyproject.as_ref().is_some_and(|table| table.contains_key("build-system") || table.contains_key("project"));
        if pyproject.is_none() && requirements.is_empty() && !installable {
            return Vec::new();
        }
        let mut steps = vec![ensure_command("pip3", "python3-pip python3-venv")];
        let tools: Vec<&str> = PYTHON_TOOLS.iter().copied().filter(|tool| uses(facts, tool)).collect();
        if !tools.is_empty() {
            steps.push(format!("pip3 install -q {}", tools.join(" ")));
        }
        if installable {
            let extras = pyproject.as_ref().map(optional_extras).unwrap_or_default();
            let target = if extras.is_empty() { ".".to_string() } else { format!(".[{}]", extras.join(",")) };
            steps.push(format!("pip3 install -q -e {}", quote(&target)));
        }
        steps.extend(requirements.iter().map(|file| format!("pip3 install -q -r {file}")));
        let groups = pyproject.as_ref().map(dependency_groups).unwrap_or_default();
        if !groups.is_empty() {
            let quoted: Vec<String> = groups.iter().map(|g| quote(g)).collect();
            steps.push(format!("pip3 install -q {}", quoted.join(" ")));
        }
        if uses(facts, "pre-commit") {
            steps.push("pre-commit install-hooks".into());
        }
        steps
    }
}

fn optional_extras(pyproject: &toml::Table) -> Vec<String> {
    let extras = pyproject
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|o| o.as_table());
    PYTHON_EXTRAS
        .iter()
        .filter(|name| extras.is_some_and(|table| table.contains_key(**name)))
        .map(|name| name.to_string())
        .collect()
}

fn dependency_groups(pyproject: &toml::Table) -> Vec<String> {
    let Some(groups) = pyproject.get("dependency-groups").and_then(|g| g.as_table()) else {
        return Vec::new();
    };
    let mut requirements: Vec<String> = groups
        .iter()
        .filter(|(name, _)| PYTHON_GROUPS.contains(&name.as_str()))
        .filter_map(|(_, group)| group.as_array())
        .flatten()
        .filter_map(|entry| entry.as_str().map(String::from))
        .collect();
    requirements.sort();
    requirements.dedup();
    requirements
}

pub struct Node;

impl Toolchain for Node {
    fn name(&self) -> &'static str {
        "node"
    }

    fn steps(&self, probe: &Probe, _facts: &ProjectFacts) -> Vec<String> {
        let Some(manifest) = probe.files.get("package.json") else {
            return Vec::new();
        };
        let mut steps = vec![ensure_command("npm", "nodejs npm")];
        if has_dependencies(manifest) {
            steps.push(if probe.files.contains_key("pnpm-lock.yaml") {
                "npm install -g -s pnpm && pnpm install --frozen-lockfile".into()
            } else if probe.files.contains_key("yarn.lock") {
                "npm install -g -s yarn && yarn install --frozen-lockfile".into()
            } else if probe.files.contains_key("package-lock.json") {
                "npm ci --no-audit --no-fund".into()
            } else {
                "npm install --no-audit --no-fund".into()
            });
        }
        steps
    }
}

fn has_dependencies(manifest: &str) -> bool {
    let Ok(json) = serde_json::from_str::<Value>(manifest) else {
        return false;
    };
    ["dependencies", "devDependencies"]
        .iter()
        .any(|key| json[key].as_object().is_some_and(|deps| !deps.is_empty()))
}

pub struct Rust;

impl Toolchain for Rust {
    fn name(&self) -> &'static str {
        "rust"
    }

    fn steps(&self, probe: &Probe, _facts: &ProjectFacts) -> Vec<String> {
        if !probe.files.contains_key("Cargo.toml") {
            return Vec::new();
        }
        vec![ensure_command("cargo", "cargo"), "cargo fetch".into()]
    }
}

pub struct Go;

impl Toolchain for Go {
    fn name(&self) -> &'static str {
        "go"
    }

    fn steps(&self, probe: &Probe, _facts: &ProjectFacts) -> Vec<String> {
        if !probe.files.contains_key("go.mod") {
            return Vec::new();
        }
        vec![ensure_command("go", "golang-go"), "go mod download".into()]
    }
}

pub struct Make;

impl Toolchain for Make {
    fn name(&self) -> &'static str {
        "make"
    }

    fn steps(&self, _probe: &Probe, facts: &ProjectFacts) -> Vec<String> {
        if !uses(facts, "make") {
            return Vec::new();
        }
        vec![ensure_command("make", "make")]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::project_facts::detect;

    fn probe(files: &[(&str, &str)]) -> Probe {
        Probe {
            files: files.iter().map(|(n, c)| (n.to_string(), c.to_string())).collect(),
            ..Default::default()
        }
    }

    fn script_for(files: &[(&str, &str)]) -> Option<String> {
        let probe = probe(files);
        prepare_script(&probe, &detect(&probe))
    }

    #[test]
    fn a_project_without_known_files_needs_no_preparation() {
        assert_eq!(script_for(&[("README.md", "hi")]), None);
    }

    #[test]
    fn a_python_project_gets_pip_its_tools_its_package_and_its_groups() {
        let pyproject = r#"
[project]
name = "x"
[project.optional-dependencies]
dev = ["black"]
[build-system]
requires = ["flit_core"]
[dependency-groups]
tests = ["pytest", "freezegun"]
typing = ["mypy", { include-group = "tests" }]
docs = ["sphinx"]
[tool.pytest.ini_options]
testpaths = ["tests"]
"#;
        let script = script_for(&[("pyproject.toml", pyproject), (".pre-commit-config.yaml", "repos: []")]).unwrap();
        assert!(script.starts_with("set -e\nexport DEBIAN_FRONTEND=noninteractive"));
        assert!(script.contains("PIP_IGNORE_INSTALLED=1"));
        assert!(script.contains("command -v pip3 >/dev/null 2>&1 || install_packages python3-pip python3-venv"));
        assert!(script.contains("pip3 install -q pytest pre-commit"));
        assert!(script.contains("pip3 install -q -e '.[dev]'"));
        assert!(script.contains("pip3 install -q 'freezegun' 'mypy' 'pytest'"));
        assert!(!script.contains("sphinx"));
        assert!(script.contains("pre-commit install-hooks"));
        assert!(script.ends_with("echo '== ready'"));
    }

    #[test]
    fn requirements_files_are_installed_without_a_package() {
        let script = script_for(&[("requirements.txt", "flask")]).unwrap();
        assert!(script.contains("pip3 install -q -r requirements.txt"));
        assert!(!script.contains("-e "));
    }

    #[test]
    fn node_installs_with_the_project_lockfile() {
        let manifest = r#"{"devDependencies": {"vitest": "1"}, "scripts": {"test": "vitest"}}"#;
        let lock = script_for(&[("package.json", manifest), ("package-lock.json", "{}")]).unwrap();
        assert!(lock.contains("command -v npm >/dev/null 2>&1 || install_packages nodejs npm"));
        assert!(lock.contains("npm ci --no-audit --no-fund"));
        let pnpm = script_for(&[("package.json", manifest), ("pnpm-lock.yaml", "")]).unwrap();
        assert!(pnpm.contains("pnpm install --frozen-lockfile"));
    }

    #[test]
    fn a_package_without_dependencies_only_needs_npm() {
        let script = script_for(&[("package.json", r#"{"scripts": {"test": "node --test"}}"#)]).unwrap();
        assert!(script.contains("install_packages nodejs npm"));
        assert!(!script.contains("npm install --"));
    }

    #[test]
    fn rust_go_and_make_are_detected() {
        let script = script_for(&[("Cargo.toml", ""), ("go.mod", "module x"), ("Makefile", "test:\n\tgo test")]).unwrap();
        assert!(script.contains("echo '== rust'\ncommand -v cargo >/dev/null 2>&1 || install_packages cargo\ncargo fetch"));
        assert!(script.contains("go mod download"));
        assert!(script.contains("install_packages make"));
    }
}
