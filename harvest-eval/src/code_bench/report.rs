use knowledge_server::llm::types::Usage;
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct TurnMetrics {
    pub seconds: f64,
    pub llm_calls: u64,
    pub tool_calls: u64,
    pub usage: Usage,
    pub cost_microusd: i64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RunResult {
    pub task: String,
    pub run: usize,
    pub turns: Vec<TurnMetrics>,
    pub acceptance: Option<bool>,
    pub checks: Option<bool>,
    pub test_lines_added: u64,
    pub error: Option<String>,
}

impl RunResult {
    pub fn seconds(&self) -> f64 {
        self.turns.iter().map(|t| t.seconds).sum()
    }

    pub fn usage(&self) -> Usage {
        self.turns.iter().fold(Usage::default(), |sum, t| sum + t.usage.clone())
    }

    pub fn cost_microusd(&self) -> i64 {
        self.turns.iter().map(|t| t.cost_microusd).sum()
    }

    pub fn llm_calls(&self) -> u64 {
        self.turns.iter().map(|t| t.llm_calls).sum()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Spread {
    pub median: f64,
    pub p90: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TaskSummary {
    pub task: String,
    pub runs: usize,
    pub failed_runs: usize,
    pub seconds: Spread,
    pub llm_calls: Spread,
    pub input_tokens: Spread,
    pub cache_read_tokens: Spread,
    pub output_tokens: Spread,
    pub cost_usd: Spread,
    pub acceptance_rate: Option<f64>,
    pub checks_rate: Option<f64>,
    pub mean_test_lines_added: f64,
}

pub fn spread(values: &[f64]) -> Spread {
    if values.is_empty() {
        return Spread { median: 0.0, p90: 0.0 };
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
    Spread { median: at(0.5), p90: at(0.9) }
}

fn rate(outcomes: impl Iterator<Item = Option<bool>>) -> Option<f64> {
    let known: Vec<bool> = outcomes.flatten().collect();
    (!known.is_empty()).then(|| known.iter().filter(|ok| **ok).count() as f64 / known.len() as f64)
}

pub fn summarize(task: &str, runs: &[RunResult]) -> TaskSummary {
    let completed: Vec<&RunResult> = runs.iter().filter(|r| r.error.is_none()).collect();
    let measure = |f: &dyn Fn(&RunResult) -> f64| spread(&completed.iter().map(|r| f(r)).collect::<Vec<_>>());
    TaskSummary {
        task: task.to_string(),
        runs: runs.len(),
        failed_runs: runs.len() - completed.len(),
        seconds: measure(&|r| r.seconds()),
        llm_calls: measure(&|r| r.llm_calls() as f64),
        input_tokens: measure(&|r| r.usage().input_tokens as f64),
        cache_read_tokens: measure(&|r| r.usage().cache_read_tokens as f64),
        output_tokens: measure(&|r| r.usage().output_tokens as f64),
        cost_usd: measure(&|r| r.cost_microusd() as f64 / 1_000_000.0),
        acceptance_rate: rate(completed.iter().map(|r| r.acceptance)),
        checks_rate: rate(completed.iter().map(|r| r.checks)),
        mean_test_lines_added: if completed.is_empty() {
            0.0
        } else {
            completed.iter().map(|r| r.test_lines_added as f64).sum::<f64>() / completed.len() as f64
        },
    }
}

pub fn table(summaries: &[TaskSummary]) -> String {
    let percent = |r: Option<f64>| r.map(|v| format!("{:.0}%", v * 100.0)).unwrap_or_else(|| "-".into());
    let mut lines = vec![format!(
        "{:<28} {:>4} {:>14} {:>10} {:>16} {:>14} {:>12} {:>8} {:>7} {:>6}",
        "task", "runs", "seconds", "calls", "input tokens", "cached", "cost $", "accept", "checks", "tests"
    )];
    for s in summaries {
        lines.push(format!(
            "{:<28} {:>4} {:>14} {:>10} {:>16} {:>14} {:>12} {:>8} {:>7} {:>6.0}",
            s.task,
            format!("{}/{}", s.runs - s.failed_runs, s.runs),
            format!("{:.0} / {:.0}", s.seconds.median, s.seconds.p90),
            format!("{:.0} / {:.0}", s.llm_calls.median, s.llm_calls.p90),
            format!("{:.0} / {:.0}", s.input_tokens.median, s.input_tokens.p90),
            format!("{:.0} / {:.0}", s.cache_read_tokens.median, s.cache_read_tokens.p90),
            format!("{:.3} / {:.3}", s.cost_usd.median, s.cost_usd.p90),
            percent(s.acceptance_rate),
            percent(s.checks_rate),
            s.mean_test_lines_added,
        ));
    }
    lines.push("Cells show median / p90 over completed runs; accept and checks are pass rates; tests is the mean of test lines added.".into());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seconds: f64, cost: i64, acceptance: Option<bool>, error: Option<&str>) -> RunResult {
        RunResult {
            task: "t".into(),
            turns: vec![TurnMetrics {
                seconds,
                llm_calls: 3,
                cost_microusd: cost,
                usage: Usage { input_tokens: 100, ..Default::default() },
                ..Default::default()
            }],
            acceptance,
            error: error.map(String::from),
            test_lines_added: 10,
            ..Default::default()
        }
    }

    #[test]
    fn the_spread_reports_median_and_p90() {
        let s = spread(&[5.0, 1.0, 3.0, 2.0, 4.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
        assert_eq!(s.median, 6.0);
        assert_eq!(s.p90, 9.0);
        assert_eq!(spread(&[]), Spread { median: 0.0, p90: 0.0 });
    }

    #[test]
    fn failed_runs_are_counted_but_not_measured() {
        let runs = vec![
            run(10.0, 100_000, Some(true), None),
            run(30.0, 300_000, Some(false), None),
            run(500.0, 9_000_000, None, Some("session failed")),
        ];
        let summary = summarize("t", &runs);
        assert_eq!(summary.failed_runs, 1);
        assert_eq!(summary.seconds.p90, 30.0);
        assert_eq!(summary.acceptance_rate, Some(0.5));
        assert_eq!(summary.checks_rate, None);
        assert_eq!(summary.mean_test_lines_added, 10.0);
        assert!((summary.cost_usd.median - 0.3).abs() < 1e-9);
    }

    #[test]
    fn the_table_has_a_row_per_task() {
        let summary = summarize("tweak", &[run(10.0, 50_000, Some(true), None)]);
        let text = table(&[summary]);
        assert!(text.lines().nth(1).unwrap().starts_with("tweak"));
        assert!(text.contains("100%"));
    }
}
