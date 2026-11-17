pub const LABEL: &str = "Review";
pub const REQUEST_TEXT: &str = "Review the latest changes";
pub const REVIEWER_NAME: &str = "Harvest reviewer";

const REQUEST_CHARS: usize = 4_000;
const SHORT_COMMIT: usize = 12;

pub fn prompt(request: &str, from: &str, to: &str) -> String {
    let request: String = request.chars().take(REQUEST_CHARS).collect();
    format!(
        "The coding agent just committed {} for this request from the user:\n\n{request}\n\n\
         Review what changed between commit {} and commit {}.",
        commit_range(from, to),
        short(from),
        short(to),
    )
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(SHORT_COMMIT)]
}

fn commit_range(from: &str, to: &str) -> String {
    format!("{}..{}", short(from), short(to))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_names_the_request_and_the_commits() {
        let text = prompt("Add undo", "0123456789abcdef", "fedcba9876543210");
        assert!(text.starts_with("The coding agent just committed 0123456789ab..fedcba987654 for this request"));
        assert!(text.contains("\n\nAdd undo\n\n"));
        assert!(text.ends_with("between commit 0123456789ab and commit fedcba987654."));
    }

    #[test]
    fn long_requests_are_cut() {
        let text = prompt(&"x".repeat(10_000), "a", "b");
        assert!(text.chars().filter(|c| *c == 'x').count() == REQUEST_CHARS);
    }
}
