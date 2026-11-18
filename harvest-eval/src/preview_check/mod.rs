pub mod runner;
pub mod scenario;

use std::path::Path;

use anyhow::Result;

use crate::code_bench::client::Harvest;
use crate::session_containers::SessionContainers;
use runner::{Check, Outcome};

pub struct Options<'a> {
    pub config: &'a Path,
    pub server: &'a str,
    pub token: &'a str,
    pub scenarios: &'a Path,
    pub only: Option<&'a str>,
    pub keep_sessions: bool,
}

pub async fn run(options: Options<'_>) -> Result<Vec<Outcome>> {
    let scenarios = scenario::load_dir(options.scenarios, options.only)?;
    let check = Check {
        harvest: Harvest::new(options.server, options.token)?,
        containers: SessionContainers::from_config(options.config).await?,
        keep_sessions: options.keep_sessions,
    };
    let mut outcomes = Vec::new();
    for scenario in &scenarios {
        tracing::info!(scenario = %scenario.name, "checking");
        outcomes.push(check.run(scenario).await);
    }
    println!("{}", table(&outcomes));
    Ok(outcomes)
}

pub fn table(outcomes: &[Outcome]) -> String {
    let mut lines = vec![format!("{:<28} {:>8} {:>9} {:>9}  {}", "scenario", "image s", "session s", "preview s", "result")];
    for outcome in outcomes {
        let result = match &outcome.error {
            None => "passed".to_string(),
            Some(error) => format!("failed: {}", error.lines().next().unwrap_or_default()),
        };
        lines.push(format!(
            "{:<28} {:>8} {:>9} {:>9}  {result}",
            outcome.scenario, outcome.image_secs, outcome.session_secs, outcome.preview_secs
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_shows_timings_and_the_first_line_of_a_failure() {
        let outcomes = [
            Outcome { scenario: "flaskr-web".into(), image_secs: 90, session_secs: 5, preview_secs: 12, passed: true, error: None },
            Outcome { scenario: "amafu-cli".into(), error: Some("expected \"1.99.1\"\ncargo 1.75.0".into()), ..Outcome::default() },
        ];
        let table = table(&outcomes);
        assert!(table.contains("flaskr-web                         90         5        12  passed"));
        assert!(table.contains("amafu-cli") && table.ends_with("failed: expected \"1.99.1\""));
    }
}
