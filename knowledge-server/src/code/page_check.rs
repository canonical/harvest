use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use super::images::quote;

pub const BROWSER_HOME: &str = "/opt/harvest-browser";
const TEXT_CHARS: usize = 6_000;
const LISTED: usize = 20;
const DEFAULT_WAIT_MS: u64 = 1_000;
const MAX_WAIT_MS: u64 = 15_000;
const MAX_STEPS: usize = 30;

const PROGRAM: &str = r#"
import json, sys
from playwright.sync_api import sync_playwright

request = json.loads(sys.argv[1])
report = {"errors": [], "failed_requests": []}
with sync_playwright() as p:
    browser = p.chromium.launch()
    page = browser.new_page(viewport={"width": 1280, "height": 800})
    page.on("console", lambda m: report["errors"].append(m.text) if m.type == "error" else None)
    page.on("pageerror", lambda e: report["errors"].append(str(e)))
    page.on("requestfailed", lambda r: report["failed_requests"].append(f"{r.method} {r.url}: {r.failure}"))
    page.on("response", lambda r: report["failed_requests"].append(f"{r.request.method} {r.url}: HTTP {r.status}") if r.status >= 400 else None)
    try:
        response = page.goto(request["url"], wait_until="load", timeout=30000)
        report["status"] = response.status if response else None
        for step in request["steps"]:
            kind = step["action"]
            if kind == "click":
                page.click(step["selector"], timeout=5000)
            elif kind == "fill":
                page.fill(step["selector"], step.get("value", ""), timeout=5000)
            elif kind == "press":
                page.press(step.get("selector") or "body", step["key"], timeout=5000)
            elif kind == "wait":
                page.wait_for_timeout(min(int(step.get("ms", 500)), 5000))
        page.wait_for_timeout(request["wait_ms"])
        report["title"] = page.title()
        report["url"] = page.url
        report["text"] = page.inner_text("body")
    except Exception as e:
        report["failure"] = str(e)
    browser.close()
print(json.dumps(report))
"#;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Step {
    Click { selector: String },
    Fill { selector: String, value: String },
    Press { key: String, #[serde(default)] selector: Option<String> },
    Wait { ms: u64 },
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub url: String,
    pub steps: Vec<Step>,
    pub wait_ms: u64,
}

impl Request {
    pub fn from_params(params: &Value) -> Result<Self> {
        let port = params["port"].as_u64().filter(|p| (1..=65_535).contains(p));
        let Some(port) = port else {
            bail!("port must be the port the app listens on in this container (1-65535)");
        };
        let path = params["path"].as_str().unwrap_or("/");
        let path = if path.starts_with('/') { path.to_string() } else { format!("/{path}") };
        let steps: Vec<Step> = match &params["steps"] {
            Value::Null => Vec::new(),
            steps => serde_json::from_value(steps.clone())
                .map_err(|e| anyhow::anyhow!("steps must be click, fill, press or wait actions: {e}"))?,
        };
        if steps.len() > MAX_STEPS {
            bail!("at most {MAX_STEPS} steps per check");
        }
        Ok(Self {
            url: format!("http://127.0.0.1:{port}{path}"),
            steps,
            wait_ms: params["wait_ms"].as_u64().unwrap_or(DEFAULT_WAIT_MS).min(MAX_WAIT_MS),
        })
    }

    fn as_json(&self) -> Value {
        let steps: Vec<Value> = self
            .steps
            .iter()
            .map(|step| match step {
                Step::Click { selector } => json!({ "action": "click", "selector": selector }),
                Step::Fill { selector, value } => json!({ "action": "fill", "selector": selector, "value": value }),
                Step::Press { key, selector } => json!({ "action": "press", "key": key, "selector": selector }),
                Step::Wait { ms } => json!({ "action": "wait", "ms": ms }),
            })
            .collect();
        json!({ "url": self.url, "steps": steps, "wait_ms": self.wait_ms })
    }
}

pub fn install_script() -> String {
    format!(
        "set -e\n\
         if [ -x {home}/bin/python ] && {home}/bin/python -c 'import playwright' 2>/dev/null; then exit 0; fi\n\
         export DEBIAN_FRONTEND=noninteractive\n\
         if ! python3 -m venv --help >/dev/null 2>&1 || ! python3 -c 'import ensurepip' 2>/dev/null; then apt-get update -qq && apt-get install -y -qq python3-venv >/dev/null; fi\n\
         python3 -m venv {home}\n\
         {home}/bin/pip install -q playwright\n\
         {home}/bin/python -m playwright install --with-deps chromium >/dev/null",
        home = BROWSER_HOME
    )
}

pub fn run_script(request: &Request) -> String {
    format!(
        "cat > {home}/check.py <<'HARVEST_PAGE_CHECK'\n{PROGRAM}\nHARVEST_PAGE_CHECK\n{home}/bin/python {home}/check.py {}",
        quote(&request.as_json().to_string()),
        home = BROWSER_HOME
    )
}

pub fn describe(stdout: &str) -> Result<String> {
    let line = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or_default();
    let report: Value = serde_json::from_str(line).map_err(|_| anyhow::anyhow!("the browser check printed no report: {}", stdout.trim()))?;
    let mut out = Vec::new();
    if let Some(failure) = report["failure"].as_str() {
        out.push(format!("The check stopped early: {failure}"));
    }
    if let Some(status) = report["status"].as_u64() {
        out.push(format!("HTTP {status}"));
    }
    if let Some(title) = report["title"].as_str() {
        out.push(format!("Title: {title}"));
    }
    if let Some(url) = report["url"].as_str() {
        out.push(format!("Final URL: {url}"));
    }
    out.push(list("Console errors", &report["errors"]));
    out.push(list("Failed requests", &report["failed_requests"]));
    if let Some(text) = report["text"].as_str() {
        let shown: String = text.chars().take(TEXT_CHARS).collect();
        let more = if text.chars().count() > TEXT_CHARS { "\n(text truncated)" } else { "" };
        out.push(format!("Visible text:\n{shown}{more}"));
    }
    Ok(out.join("\n"))
}

fn list(label: &str, items: &Value) -> String {
    let items: Vec<&str> = items.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if items.is_empty() {
        return format!("{label}: none");
    }
    let mut lines = vec![format!("{label} ({}):", items.len())];
    lines.extend(items.iter().take(LISTED).map(|i| format!("- {i}")));
    if items.len() > LISTED {
        lines.push(format!("- and {} more", items.len() - LISTED));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_targets_the_container_and_parses_steps() {
        let request = Request::from_params(&json!({
            "port": 8000,
            "path": "board?x=1",
            "steps": [
                { "action": "fill", "selector": "#title", "value": "A" },
                { "action": "press", "key": "Enter", "selector": "#title" },
                { "action": "click", "selector": "text=Undo" },
                { "action": "wait", "ms": 200 }
            ]
        }))
        .unwrap();
        assert_eq!(request.url, "http://127.0.0.1:8000/board?x=1");
        assert_eq!(request.steps.len(), 4);
        assert_eq!(request.wait_ms, DEFAULT_WAIT_MS);
        assert_eq!(request.as_json()["steps"][1]["key"], "Enter");
    }

    #[test]
    fn invalid_requests_are_explained() {
        assert!(Request::from_params(&json!({})).is_err());
        assert!(Request::from_params(&json!({ "port": 70000 })).is_err());
        let bad_step = Request::from_params(&json!({ "port": 80, "steps": [{ "action": "drag" }] }));
        assert!(bad_step.unwrap_err().to_string().contains("click, fill, press or wait"));
    }

    #[test]
    fn the_wait_is_capped() {
        let request = Request::from_params(&json!({ "port": 80, "wait_ms": 999_999 })).unwrap();
        assert_eq!(request.wait_ms, MAX_WAIT_MS);
    }

    #[test]
    fn the_report_is_described_for_the_agent() {
        let stdout = "noise\n{\"status\": 200, \"title\": \"Kanban\", \"url\": \"http://127.0.0.1:8000/\", \"errors\": [\"TypeError: x is undefined\"], \"failed_requests\": [], \"text\": \"To do\\n3 cards\"}\n";
        let text = describe(stdout).unwrap();
        assert!(text.contains("HTTP 200"));
        assert!(text.contains("Title: Kanban"));
        assert!(text.contains("Console errors (1):\n- TypeError: x is undefined"));
        assert!(text.contains("Failed requests: none"));
        assert!(text.ends_with("Visible text:\nTo do\n3 cards"));
    }

    #[test]
    fn a_missing_report_is_an_error() {
        assert!(describe("Traceback: boom").is_err());
    }
}
