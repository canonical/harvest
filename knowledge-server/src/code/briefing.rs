use harvest_search::Scope;

use super::index::{self, IndexedVersion};
use super::CodeEnv;
use crate::agent::prompt::CODE_SESSION_GUIDE;

const SHORT_COMMIT: usize = 12;

pub struct Briefing {
    pub appendix: String,
    pub focus: Option<Scope>,
}

pub async fn for_session(env: &CodeEnv, session_id: &str) -> Briefing {
    let indexed = index::of_session(env, session_id).await.unwrap_or_else(|e| {
        tracing::warn!(session = session_id, error = %e, "could not look up the session's index");
        None
    });
    Briefing {
        appendix: format!("{CODE_SESSION_GUIDE}\n\n{}", describe(indexed.as_ref())),
        focus: indexed
            .filter(|i| i.ingested)
            .map(|i| Scope::new(&i.repo, &i.version)),
    }
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
