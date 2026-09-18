use anyhow::{Context, Result};
use chrono::Utc;
use std::path::{Path, PathBuf};

use crate::explore::ExplorationResult;
use crate::llm::pricing::format_cost;

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for c in text.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_dash = false;
        } else if !last_was_dash && !slug.is_empty() {
            slug.push('-');
            last_was_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "exploration".to_string()
    } else {
        slug.chars().take(60).collect()
    }
}

pub fn run_dir(output_dir: &str, objective: &str) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S");
    Path::new(output_dir).join(format!("{timestamp}-{}", slugify(objective)))
}

fn render_markdown(result: &ExplorationResult) -> String {
    let mut md = String::new();
    md.push_str("# Squall Exploration Report\n\n");
    md.push_str("## Objective\n\n");
    md.push_str(&result.objective);
    md.push_str("\n\n## Quality Assessment\n\n");
    md.push_str(&result.report.quality_assessment);
    md.push_str("\n\n## Bugs Found\n\n");
    if result.report.bugs_found.is_empty() {
        md.push_str("None reported.\n");
    } else {
        let mut bugs = result.report.bugs_found.clone();
        bugs.sort_by_key(|b| severity_rank(&b.severity));
        for bug in &bugs {
            md.push_str(&format!("### [{}] {}\n\n", bug.severity, bug.area));
            md.push_str(&bug.description);
            md.push('\n');
            if !bug.repro_steps.is_empty() {
                md.push_str(&format!("\nRepro: {}\n", bug.repro_steps));
            }
            if !bug.evidence.is_empty() {
                md.push_str(&format!("\nEvidence: {}\n", bug.evidence));
            }
            md.push('\n');
        }
    }
    md.push_str("## Improvements\n\n");
    if result.report.improvements.is_empty() {
        md.push_str("None reported.\n");
    } else {
        for imp in &result.report.improvements {
            md.push_str(&format!("### {}\n\n{}\n", imp.area, imp.suggestion));
            if !imp.rationale.is_empty() {
                md.push_str(&format!("\nRationale: {}\n", imp.rationale));
            }
            md.push('\n');
        }
    }
    md.push_str("## Summary\n\n");
    md.push_str(&result.report.summary);
    md.push_str("\n\n## Run Stats\n\n");
    md.push_str(&format!("- Iterations in transcript: {}\n", result.transcript.len()));
    md.push_str(&format!("- Tool calls made: {}\n", result.tool_calls_made));
    md.push_str(&format!("- Hit max iterations: {}\n", result.hit_max_iterations));
    md.push_str(&format!("- Total tokens: {}\n", result.total_usage.total_tokens()));
    md.push_str(&format!("- Estimated cost: {}\n", format_cost(result.estimated_cost_microusd)));
    md.push_str(&format!("- Duration: {} ms\n", result.duration_ms));
    md
}

fn severity_rank(severity: &str) -> u8 {
    match severity.to_lowercase().as_str() {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

fn render_transcript_jsonl(result: &ExplorationResult) -> Result<String> {
    let mut lines = String::new();
    for entry in &result.transcript {
        lines.push_str(&serde_json::to_string(entry)?);
        lines.push('\n');
    }
    Ok(lines)
}

pub async fn write(dir: &Path, result: &ExplorationResult) -> Result<()> {
    tokio::fs::create_dir_all(dir).await.with_context(|| format!("creating report dir: {}", dir.display()))?;

    let md = render_markdown(result);
    tokio::fs::write(dir.join("report.md"), md).await.context("writing report.md")?;

    let json = serde_json::to_string_pretty(result)?;
    tokio::fs::write(dir.join("report.json"), json).await.context("writing report.json")?;

    let jsonl = render_transcript_jsonl(result)?;
    tokio::fs::write(dir.join("transcript.jsonl"), jsonl).await.context("writing transcript.jsonl")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::explore::{Bug, FinishExplorationPayload, Improvement, TranscriptEntry};
    use crate::llm::types::Usage;

    fn sample_result() -> ExplorationResult {
        ExplorationResult {
            objective: "Test the chat flow".to_string(),
            transcript: vec![TranscriptEntry {
                iteration: 0,
                provider_id: "anthropic-0".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                assistant_text: "creating a project".to_string(),
                tool_calls: vec![],
            }],
            tool_calls_made: 3,
            total_usage: Usage { input_tokens: 100, output_tokens: 50, ..Default::default() },
            estimated_cost_microusd: 42_000,
            duration_ms: 1500,
            hit_max_iterations: false,
            report: FinishExplorationPayload {
                quality_assessment: "solid".to_string(),
                bugs_found: vec![
                    Bug { severity: "low".to_string(), area: "chat".to_string(), description: "minor issue".to_string(), repro_steps: String::new(), evidence: String::new() },
                    Bug { severity: "critical".to_string(), area: "deploy".to_string(), description: "big issue".to_string(), repro_steps: "do X".to_string(), evidence: "log line".to_string() },
                ],
                improvements: vec![Improvement { area: "ux".to_string(), suggestion: "clearer errors".to_string(), rationale: "confused users".to_string() }],
                summary: "all good".to_string(),
            },
        }
    }

    #[test]
    fn slugify_lowercases_and_dashes_the_objective() {
        assert_eq!(slugify("Test the Chat Flow!"), "test-the-chat-flow");
    }

    #[test]
    fn slugify_collapses_repeated_separators() {
        assert_eq!(slugify("a   b---c"), "a-b-c");
    }

    #[test]
    fn slugify_falls_back_when_nothing_alphanumeric() {
        assert_eq!(slugify("!!!"), "exploration");
    }

    #[test]
    fn run_dir_embeds_a_timestamp_and_the_slug() {
        let dir = run_dir("./squall-reports", "Test the Chat Flow");
        let name = dir.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with("test-the-chat-flow"));
        assert_eq!(dir.parent().unwrap(), Path::new("./squall-reports"));
    }

    #[test]
    fn markdown_report_sorts_bugs_by_severity_and_includes_stats() {
        let md = render_markdown(&sample_result());
        let critical_pos = md.find("[critical]").unwrap();
        let low_pos = md.find("[low]").unwrap();
        assert!(critical_pos < low_pos, "critical bug should be listed before low severity");
        assert!(md.contains("Tool calls made: 3"));
        assert!(md.contains("Estimated cost: $0.042000"));
    }

    #[test]
    fn markdown_report_notes_when_no_bugs_found() {
        let mut result = sample_result();
        result.report.bugs_found = vec![];
        let md = render_markdown(&result);
        assert!(md.contains("None reported."));
    }

    #[test]
    fn transcript_jsonl_has_one_line_per_entry() {
        let jsonl = render_transcript_jsonl(&sample_result()).unwrap();
        assert_eq!(jsonl.lines().count(), 1);
        let parsed: serde_json::Value = serde_json::from_str(jsonl.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["assistant_text"], "creating a project");
    }

    #[tokio::test]
    async fn write_creates_all_three_report_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("run1");
        write(&target, &sample_result()).await.unwrap();
        assert!(target.join("report.md").exists());
        assert!(target.join("report.json").exists());
        assert!(target.join("transcript.jsonl").exists());
    }
}
