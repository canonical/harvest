use super::{store, Job, Kind};
use crate::code::job_tools::REPORT_CHARS;
use crate::code::CodeEnv;

pub async fn for_turn(env: &CodeEnv, session_id: &str, chat_id: &str, takes_reports: bool) -> Option<String> {
    let finished = async {
        if takes_reports {
            store::take_unreported(env, session_id, chat_id).await
        } else {
            Ok(Vec::new())
        }
    };
    let (running, finished) = tokio::join!(store::running_in_session(env, session_id), finished);
    let warn = |e: anyhow::Error| tracing::warn!(session = session_id, error = %e, "could not read background jobs");
    render(&running.map_err(warn).unwrap_or_default(), &finished.map_err(warn).unwrap_or_default())
}

pub fn render(running: &[Job], finished: &[Job]) -> Option<String> {
    if running.is_empty() && finished.is_empty() {
        return None;
    }
    let mut lines = vec!["## Background jobs".to_string()];
    for job in running {
        let advice = if job.kind == Kind::Prepare {
            " It installs the tools and dependencies this repository needs: call wait_job before running its tests or checks."
        } else {
            ""
        };
        lines.push(format!("- {} ({}) is running.{advice}", job.label, job.id));
    }
    for job in finished {
        let advice = if job.kind == Kind::Checks && job.exit_code != Some(0) {
            "\nThese are the repository's checks run by Harvest after your last changes. Fix what failed unless the user asked for something else."
        } else {
            ""
        };
        lines.push(format!("{}{advice}", job.report(REPORT_CHARS)));
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(kind: &str, status: &str, exit_code: Option<i64>) -> Job {
        Job::from_row(&json!({
            "id": format!("{kind}-1"),
            "session_id": "s",
            "kind": kind,
            "label": kind,
            "status": status,
            "exit_code": exit_code,
            "output": "tail",
        }))
        .unwrap()
    }

    #[test]
    fn nothing_running_or_finished_needs_no_note() {
        assert_eq!(render(&[], &[]), None);
    }

    #[test]
    fn a_running_preparation_tells_the_agent_to_wait() {
        let text = render(&[job("prepare", "running", None)], &[]).unwrap();
        assert!(text.contains("- prepare (prepare-1) is running. It installs"));
        assert!(text.contains("call wait_job"));
    }

    #[test]
    fn failed_checks_ask_for_a_fix() {
        let text = render(&[], &[job("checks", "failed", Some(1))]).unwrap();
        assert!(text.contains("Background job checks (checks-1) failed with exit code 1."));
        assert!(text.contains("Fix what failed"));
        let passed = render(&[], &[job("checks", "succeeded", Some(0))]).unwrap();
        assert!(!passed.contains("Fix what failed"));
    }
}
