use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub name: String,
    pub repository: String,
    pub base: String,
    pub prepare: Vec<String>,
    pub preview: Value,
    pub verification: Verification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    Page { port: u16, path: String, contains: String },
    Command { command: String, contains: String },
}

#[derive(Deserialize)]
struct Definition {
    name: String,
    repository: String,
    #[serde(default = "default_base")]
    base: String,
    #[serde(default)]
    prepare: Vec<String>,
    preview: PreviewDefinition,
    expect: Expectation,
}

#[derive(Deserialize)]
struct PreviewDefinition {
    kind: String,
    port: Option<u16>,
    start_command: Option<String>,
}

#[derive(Deserialize)]
struct Expectation {
    path: Option<String>,
    command: Option<String>,
    contains: String,
}

fn default_base() -> String {
    "main".into()
}

impl Scenario {
    pub fn parse(text: &str) -> Result<Self> {
        let definition: Definition = toml::from_str(text)?;
        let verification = verification_for(&definition)?;
        Ok(Self {
            name: definition.name,
            repository: definition.repository,
            base: definition.base,
            prepare: definition.prepare,
            preview: json!({
                "kind": definition.preview.kind,
                "port": definition.preview.port,
                "start_command": definition.preview.start_command,
            }),
            verification,
        })
    }
}

fn verification_for(definition: &Definition) -> Result<Verification> {
    let contains = definition.expect.contains.clone();
    match (definition.preview.kind.as_str(), definition.preview.port, &definition.expect) {
        ("web" | "api", Some(port), Expectation { path: Some(path), .. }) => {
            Ok(Verification::Page { port, path: path.clone(), contains })
        }
        ("cli", _, Expectation { command: Some(command), .. }) => {
            Ok(Verification::Command { command: command.clone(), contains })
        }
        ("web" | "api", _, _) => bail!("{}: web and api previews need a port and an expected path", definition.name),
        ("cli", _, _) => bail!("{}: cli previews need an expected command", definition.name),
        (kind, _, _) => bail!("{}: unknown preview kind {kind}", definition.name),
    }
}

pub fn load_dir(dir: &Path, only: Option<&str>) -> Result<Vec<Scenario>> {
    let mut scenarios = Vec::new();
    for (path, text) in crate::toml_dir::read(dir)? {
        let scenario = Scenario::parse(&text).with_context(|| format!("parsing {}", path.display()))?;
        if only.is_none_or(|name| name == scenario.name) {
            scenarios.push(scenario);
        }
    }
    if scenarios.is_empty() {
        bail!("no matching scenarios in {}", dir.display());
    }
    Ok(scenarios)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_scenario_checks_a_page_of_the_running_app() {
        let scenario = Scenario::parse(
            r#"
name = "flaskr-web"
repository = "https://github.com/pallets/flask"
prepare = ["pip install -q -e examples/tutorial"]
[preview]
kind = "web"
port = 5000
start_command = "flask --app flaskr run --port $PORT"
[expect]
path = "/auth/login"
contains = "Log In"
"#,
        )
        .unwrap();
        assert_eq!(scenario.base, "main");
        assert_eq!(scenario.preview["port"], 5000);
        assert_eq!(
            scenario.verification,
            Verification::Page { port: 5000, path: "/auth/login".into(), contains: "Log In".into() }
        );
    }

    #[test]
    fn a_cli_scenario_runs_a_command_in_the_preview() {
        let scenario = Scenario::parse(
            r#"
name = "amafu-cli"
repository = "https://github.com/canonical/amafu"
[preview]
kind = "cli"
[expect]
command = "cargo --version"
contains = "1.99.1"
"#,
        )
        .unwrap();
        assert_eq!(scenario.verification, Verification::Command { command: "cargo --version".into(), contains: "1.99.1".into() });
    }

    #[test]
    fn the_bundled_scenarios_all_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tasks/preview");
        let names: Vec<String> = load_dir(&dir, None).unwrap().into_iter().map(|scenario| scenario.name).collect();
        assert_eq!(names, ["amafu-cli", "flaskr-web", "itsdangerous-cli", "kanban-static"]);
    }

    #[test]
    fn a_web_scenario_without_a_page_to_check_is_refused() {
        let error = Scenario::parse(
            "name = \"x\"\nrepository = \"r\"\n[preview]\nkind = \"web\"\nport = 80\n[expect]\ncontains = \"a\"\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("need a port and an expected path"));
    }
}
