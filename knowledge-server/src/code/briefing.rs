use harvest_search::Scope;

use super::freshness::{Baseline, Drift};
use super::index::{self, IndexedVersion};
use super::{freshness, jobs, project_facts, sessions, working_set, CodeEnv};
use crate::agent::prompt::CODE_SESSION_GUIDE;

const SHORT_COMMIT: usize = 12;
const LISTED_FILES: usize = 20;
const LARGE_REPOSITORY_FILES: i64 = 1_500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    Author,
    Reviewer,
}

pub struct Briefing {
    pub context: String,
    pub focus: Option<Scope>,
    pub parallel_research: bool,
}

impl Briefing {
    pub fn without_session() -> Self {
        Self {
            context: String::new(),
            focus: None,
            parallel_research: false,
        }
    }

    pub fn appendix(&self) -> String {
        self.appendix_for(CODE_SESSION_GUIDE)
    }

    pub fn appendix_for(&self, instructions: &str) -> String {
        [instructions, self.context.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

pub fn explores_in_parallel(files: i64) -> bool {
    files >= LARGE_REPOSITORY_FILES
}

pub async fn for_session(env: &CodeEnv, session_id: &str, chat_id: &str, reader: Reader) -> Briefing {
    let indexed = index::of_session(env, session_id).await.unwrap_or_else(|e| {
        tracing::warn!(session = session_id, error = %e, "could not look up the session's index");
        None
    });
    let (drift, facts, working, background, files) = tokio::join!(
        drift_note(env, session_id, indexed.as_ref()),
        facts_note(env, session_id),
        working_set::for_chat(env, session_id, chat_id),
        jobs::notes::for_turn(env, session_id, chat_id, reader == Reader::Author),
        indexed_files(env, indexed.as_ref()),
    );
    let sections = [
        Some(describe(indexed.as_ref())),
        drift,
        facts,
        background,
        working,
    ];
    Briefing {
        context: sections.into_iter().flatten().collect::<Vec<_>>().join("\n\n"),
        focus: indexed
            .filter(|i| i.ingested)
            .map(|i| Scope::new(&i.repo, &i.version)),
        parallel_research: explores_in_parallel(files),
    }
}

async fn indexed_files(env: &CodeEnv, indexed: Option<&IndexedVersion>) -> i64 {
    let Some(indexed) = indexed.filter(|i| i.ingested) else {
        return 0;
    };
    index::file_count(&env.db, indexed).await.unwrap_or_else(|e| {
        tracing::warn!(repo = indexed.repo, error = %e, "could not count the indexed files");
        0
    })
}

async fn drift_note(env: &CodeEnv, session_id: &str, indexed: Option<&IndexedVersion>) -> Option<String> {
    let indexed = indexed.filter(|i| i.ingested)?;
    match freshness::drift(env, session_id, indexed).await {
        Ok(drift) => describe_drift(&drift),
        Err(e) => {
            tracing::warn!(session = session_id, error = %e, "could not compare the workspace with the index");
            None
        }
    }
}

async fn facts_note(env: &CodeEnv, session_id: &str) -> Option<String> {
    let container = sessions::active_container(env, session_id).await.ok()?;
    match sessions::run_checked(env, &container, &project_facts::probe_script()).await {
        Ok(output) => project_facts::render(&project_facts::detect(&project_facts::parse_probe(&output))),
        Err(e) => {
            tracing::warn!(session = session_id, error = %e, "could not read the project's commands");
            None
        }
    }
}

pub fn describe_drift(drift: &Drift) -> Option<String> {
    if drift.files.is_empty() {
        return None;
    }
    let against = match drift.baseline {
        Baseline::Indexed => "the indexed commit",
        Baseline::BaseBranch => "the base branch",
    };
    let mut lines = vec![format!(
        "{} files in your workspace differ from {against}, so the code graph and search_code do not see their current content. Use search_workspace and read_file for them:",
        drift.files.len()
    )];
    lines.extend(drift.files.keys().take(LISTED_FILES).map(|path| format!("- {path}")));
    if drift.files.len() > LISTED_FILES {
        lines.push(format!("- and {} more", drift.files.len() - LISTED_FILES));
    }
    Some(lines.join("\n"))
}

pub fn describe(indexed: Option<&IndexedVersion>) -> String {
    let Some(indexed) = indexed else {
        return "This project has no code graph; explore it with search_workspace and read_file.".into();
    };
    if !indexed.ingested {
        return format!(
            "The code graph of {} is still being built; until it is ready, explore with search_workspace and read_file.",
            indexed.repo
        );
    }
    let at = match &indexed.commit {
        Some(commit) => format!(" at commit {}", &commit[..commit.len().min(SHORT_COMMIT)]),
        None => String::new(),
    };
    let mut text = format!(
        "The code graph and search tools index this project as repository \"{}\", version \"{}\"{at}; pass these as repo and version.",
        indexed.repo, indexed.version
    );
    if let Some(error) = &indexed.error {
        text.push_str(&format!(
            " The index could not be refreshed recently ({error}), so it may be older than the remote branch."
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::freshness::Change;

    #[test]
    fn the_coding_guide_or_a_role_comes_before_the_session_context() {
        let briefing = Briefing { context: "CONTEXT".into(), ..Briefing::without_session() };
        assert!(briefing.appendix().starts_with("## Coding session"));
        assert!(briefing.appendix().ends_with("\n\nCONTEXT"));
        assert_eq!(briefing.appendix_for("ROLE"), "ROLE\n\nCONTEXT");
        assert_eq!(Briefing::without_session().appendix_for("ROLE"), "ROLE");
    }

    #[test]
    fn only_large_repositories_are_explored_in_parallel() {
        assert!(!explores_in_parallel(200));
        assert!(explores_in_parallel(LARGE_REPOSITORY_FILES));
    }

    #[test]
    fn a_workspace_matching_the_index_needs_no_note() {
        let drift = Drift { baseline: Baseline::Indexed, files: Default::default() };
        assert_eq!(describe_drift(&drift), None);
    }

    #[test]
    fn files_the_graph_does_not_see_are_listed() {
        let files = (0..25).map(|i| (format!("src/f{i:02}.js"), Change::Added)).collect();
        let text = describe_drift(&Drift { baseline: Baseline::Indexed, files }).unwrap();
        assert!(text.starts_with("25 files in your workspace differ from the indexed commit"));
        assert!(text.contains("- src/f00.js"));
        assert!(!text.contains("- src/f20.js"));
        assert!(text.ends_with("- and 5 more"));
    }

    fn indexed(ingested: bool, commit: Option<&str>, error: Option<&str>) -> IndexedVersion {
        IndexedVersion {
            repo: "app".into(),
            version: "main".into(),
            commit: commit.map(String::from),
            ingested,
            error: error.map(String::from),
        }
    }

    #[test]
    fn an_indexed_project_names_its_repository_version_and_commit() {
        let text = describe(Some(&indexed(true, Some("0123456789abcdef"), None)));
        assert_eq!(
            text,
            "The code graph and search tools index this project as repository \"app\", version \"main\" at commit 0123456789ab; pass these as repo and version."
        );
    }

    #[test]
    fn an_unknown_commit_is_left_out() {
        assert!(describe(Some(&indexed(true, None, None))).contains("version \"main\"; pass"));
    }

    #[test]
    fn a_failed_refresh_warns_that_the_index_may_be_old() {
        let text = describe(Some(&indexed(true, Some("abc"), Some("authentication failed"))));
        assert!(text.ends_with("(authentication failed), so it may be older than the remote branch."));
    }

    #[test]
    fn a_project_still_being_indexed_points_to_the_workspace() {
        assert!(describe(Some(&indexed(false, None, None))).starts_with("The code graph of app is still being built"));
        assert!(describe(None).contains("search_workspace"));
    }
}
