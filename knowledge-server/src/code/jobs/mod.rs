pub mod reporting;
pub mod shell;
pub mod store;

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Prepare,
    Command,
    Checks,
    Review,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Command => "command",
            Self::Checks => "checks",
            Self::Review => "review",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::Prepare, Self::Command, Self::Checks, Self::Review]
            .into_iter()
            .find(|kind| kind.as_str() == value)
    }

    pub fn runs_in_container(self) -> bool {
        !matches!(self, Self::Review)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::Running, Self::Succeeded, Self::Failed, Self::Cancelled]
            .into_iter()
            .find(|status| status.as_str() == value)
    }

    pub fn from_exit_code(code: i64) -> Self {
        if code == 0 {
            Self::Succeeded
        } else {
            Self::Failed
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Job {
    pub id: String,
    pub session_id: String,
    pub chat_id: Option<String>,
    pub kind: Kind,
    pub label: String,
    pub status: Status,
    pub exit_code: Option<i64>,
    pub output: String,
    pub created_at: String,
    pub finished_at: Option<String>,
}

impl Job {
    pub fn from_row(row: &Value) -> Option<Self> {
        let text = |key: &str| row[key].as_str().map(String::from);
        Some(Self {
            id: text("id")?,
            session_id: text("session_id")?,
            chat_id: text("chat_id"),
            kind: Kind::parse(row["kind"].as_str()?)?,
            label: text("label")?,
            status: Status::parse(row["status"].as_str()?)?,
            exit_code: row["exit_code"].as_i64(),
            output: text("output").unwrap_or_default(),
            created_at: text("created_at").unwrap_or_default(),
            finished_at: text("finished_at"),
        })
    }

    pub fn is_running(&self) -> bool {
        self.status == Status::Running
    }

    pub fn report(&self, output_chars: usize) -> String {
        let outcome = match (self.status, self.exit_code) {
            (Status::Running, _) => "is still running".to_string(),
            (Status::Cancelled, _) => "was stopped".to_string(),
            (status, Some(code)) => format!("{} with exit code {code}", status.as_str()),
            (status, None) => status.as_str().to_string(),
        };
        let output = tail_chars(self.output.trim_end(), output_chars);
        if output.is_empty() {
            format!("Background job {} ({}) {outcome}.", self.label, self.id)
        } else {
            format!("Background job {} ({}) {outcome}. Last output:\n{output}", self.label, self.id)
        }
    }
}

fn tail_chars(text: &str, chars: usize) -> &str {
    let count = text.chars().count();
    if count <= chars {
        return text;
    }
    let start = text.char_indices().nth(count - chars).map(|(i, _)| i).unwrap_or(0);
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job(status: Status, exit_code: Option<i64>, output: &str) -> Job {
        Job::from_row(&json!({
            "id": "j1",
            "session_id": "s1",
            "kind": "checks",
            "label": "checks",
            "status": status.as_str(),
            "exit_code": exit_code,
            "output": output,
        }))
        .unwrap()
    }

    #[test]
    fn kinds_and_statuses_round_trip_through_their_names() {
        for kind in [Kind::Prepare, Kind::Command, Kind::Checks, Kind::Review] {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        for status in [Status::Running, Status::Succeeded, Status::Failed, Status::Cancelled] {
            assert_eq!(Status::parse(status.as_str()), Some(status));
        }
        assert_eq!(Status::from_exit_code(0), Status::Succeeded);
        assert_eq!(Status::from_exit_code(2), Status::Failed);
    }

    #[test]
    fn a_report_names_the_outcome_and_the_end_of_the_output() {
        let failed = job(Status::Failed, Some(1), "line 1\nline 2\nFAILED test_x");
        assert_eq!(
            failed.report(13),
            "Background job checks (j1) failed with exit code 1. Last output:\nFAILED test_x"
        );
        assert_eq!(job(Status::Running, None, "").report(10), "Background job checks (j1) is still running.");
    }

    #[test]
    fn only_reviews_run_outside_the_container() {
        assert!(Kind::Command.runs_in_container());
        assert!(!Kind::Review.runs_in_container());
    }
}
