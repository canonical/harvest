use anyhow::Result;

use super::{sessions, CodeEnv};

pub const SNAPSHOT_SCRIPT: &str =
    "git rev-parse HEAD 2>/dev/null || echo none; git status --porcelain=v1 --untracked-files=all 2>/dev/null | sha256sum";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub head: String,
    pub status: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Changes {
    pub files: bool,
    pub commits: Option<(String, String)>,
}

pub fn parse(output: &str) -> Snapshot {
    let mut lines = output.lines().map(str::trim);
    Snapshot {
        head: lines.next().unwrap_or_default().to_string(),
        status: lines.next().unwrap_or_default().to_string(),
    }
}

pub fn between(before: &Snapshot, after: &Snapshot) -> Changes {
    Changes {
        files: before != after,
        commits: (before.head != after.head).then(|| (before.head.clone(), after.head.clone())),
    }
}

pub async fn take(env: &CodeEnv, session_id: &str) -> Result<Snapshot> {
    let container = sessions::active_container(env, session_id).await?;
    Ok(parse(&sessions::run_checked(env, &container, SNAPSHOT_SCRIPT).await?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(head: &str, status: &str) -> Snapshot {
        Snapshot { head: head.into(), status: status.into() }
    }

    #[test]
    fn snapshots_hold_the_head_and_the_status_hash() {
        assert_eq!(parse("abc123\ne3b0  -\n"), snapshot("abc123", "e3b0  -"));
    }

    #[test]
    fn an_untouched_workspace_has_no_changes() {
        let same = snapshot("a", "s");
        assert_eq!(between(&same, &same), Changes { files: false, commits: None });
    }

    #[test]
    fn edits_without_a_commit_change_files_only() {
        assert_eq!(between(&snapshot("a", "s1"), &snapshot("a", "s2")), Changes { files: true, commits: None });
    }

    #[test]
    fn a_new_commit_names_the_range() {
        let changes = between(&snapshot("a", "s"), &snapshot("b", "s"));
        assert!(changes.files);
        assert_eq!(changes.commits, Some(("a".into(), "b".into())));
    }
}
