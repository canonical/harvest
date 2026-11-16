use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use anyhow::Result;

use super::images::quote;
use super::index::IndexedVersion;
use super::{sessions, CodeEnv};

const LISTED: usize = 20;
const FAR_BEHIND: usize = 500;
const INDEXED_MARK: &str = "@indexed";
const BASE_MARK: &str = "@base";
const UNTRACKED_MARK: &str = "@untracked";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
}

impl Change {
    fn from_status(status: &str) -> Self {
        match status.chars().next() {
            Some('A') => Self::Added,
            Some('D') => Self::Deleted,
            _ => Self::Modified,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Modified => "modified",
            Self::Added => "added",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Baseline {
    Indexed,
    BaseBranch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drift {
    pub baseline: Baseline,
    pub files: BTreeMap<String, Change>,
}

pub fn script(indexed_commit: Option<&str>, base_branch: &str) -> String {
    let base = quote(&format!("refs/remotes/origin/{base_branch}"));
    let choose = match indexed_commit {
        Some(commit) => format!(
            "if git cat-file -e {}^{{commit}} 2>/dev/null; then echo {INDEXED_MARK}; ref={}; else echo {BASE_MARK}; ref={base}; fi",
            quote(commit),
            quote(commit)
        ),
        None => format!("echo {BASE_MARK}; ref={base}"),
    };
    format!(
        "{choose}\ngit -c core.quotePath=false diff --name-status --no-renames \"$ref\" --\necho {UNTRACKED_MARK}\ngit -c core.quotePath=false ls-files --others --exclude-standard"
    )
}

pub fn parse(output: &str) -> Drift {
    let mut lines = output.lines();
    let baseline = match lines.next().map(str::trim) {
        Some(INDEXED_MARK) => Baseline::Indexed,
        _ => Baseline::BaseBranch,
    };
    let mut files = BTreeMap::new();
    let mut untracked = false;
    for line in lines {
        if line == UNTRACKED_MARK {
            untracked = true;
        } else if untracked {
            if !line.is_empty() {
                files.insert(line.to_string(), Change::Added);
            }
        } else if let Some((status, path)) = line.split_once('\t') {
            files.insert(path.to_string(), Change::from_status(status));
        }
    }
    Drift { baseline, files }
}

impl Drift {
    pub fn footer(&self, output: &str, repo: &str) -> Option<String> {
        let mentioned: Vec<(&String, &Change)> =
            self.files.iter().filter(|(path, _)| mentions(output, path)).collect();
        let far_behind = self.files.len() > FAR_BEHIND;
        if mentioned.is_empty() && !far_behind {
            return None;
        }
        let mut lines = Vec::new();
        if !mentioned.is_empty() {
            let against = match self.baseline {
                Baseline::Indexed => format!("the indexed version of {repo}"),
                Baseline::BaseBranch => {
                    format!("the base branch of {repo} (the indexed commit is not in this workspace)")
                }
            };
            lines.push(format!(
                "Workspace check: these files differ from {against}, so the results above may be out of date for them:"
            ));
            lines.extend(
                mentioned
                    .iter()
                    .take(LISTED)
                    .map(|(path, change)| format!("- {path} ({} in your workspace)", change.label())),
            );
            if mentioned.len() > LISTED {
                lines.push(format!("- and {} more", mentioned.len() - LISTED));
            }
            lines.push("Read them from /workspace with read_file before relying on them.".into());
        }
        if far_behind {
            lines.push(format!(
                "The index differs from your workspace in {} files; prefer search_workspace and read_file for this repository.",
                self.files.len()
            ));
        }
        Some(lines.join("\n"))
    }
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/')
}

fn mentions(text: &str, path: &str) -> bool {
    text.match_indices(path).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        !before.is_some_and(is_path_char) && ends_path(&text[start + path.len()..])
    })
}

fn ends_path(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        None => true,
        Some('.') => !chars.next().is_some_and(char::is_alphanumeric),
        Some(c) => !is_path_char(c),
    }
}

#[derive(Default)]
pub struct DriftCache {
    entries: Mutex<HashMap<String, (Option<String>, Arc<Drift>)>>,
}

impl DriftCache {
    pub fn invalidate(&self, session_id: &str) {
        self.lock().remove(session_id);
    }

    fn get(&self, session_id: &str, commit: Option<&str>) -> Option<Arc<Drift>> {
        self.lock()
            .get(session_id)
            .filter(|(cached, _)| cached.as_deref() == commit)
            .map(|(_, drift)| Arc::clone(drift))
    }

    fn put(&self, session_id: &str, commit: Option<&str>, drift: Arc<Drift>) {
        self.lock()
            .insert(session_id.to_string(), (commit.map(String::from), drift));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Option<String>, Arc<Drift>)>> {
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub async fn drift(env: &CodeEnv, session_id: &str, index: &IndexedVersion) -> Result<Arc<Drift>> {
    let commit = index.commit.as_deref();
    if let Some(cached) = env.drift.get(session_id, commit) {
        return Ok(cached);
    }
    let session = sessions::get(env, session_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the session no longer exists"))?;
    let container = sessions::active_container(env, session_id).await?;
    let output = sessions::run_checked(env, &container, &script(commit, &session.base_branch)).await?;
    let drift = Arc::new(parse(&output));
    env.drift.put(session_id, commit, Arc::clone(&drift));
    Ok(drift)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drift(files: &[(&str, Change)]) -> Drift {
        Drift {
            baseline: Baseline::Indexed,
            files: files.iter().map(|(p, c)| (p.to_string(), *c)).collect(),
        }
    }

    #[test]
    fn the_script_falls_back_to_the_base_branch_without_an_indexed_commit() {
        let with = script(Some("abc123"), "main");
        assert!(with.contains("git cat-file -e 'abc123'^{commit}"));
        assert!(with.contains("ref='refs/remotes/origin/main'"));
        let without = script(None, "main");
        assert!(without.starts_with("echo @base; ref='refs/remotes/origin/main'"));
        assert!(without.contains("ls-files --others --exclude-standard"));
    }

    #[test]
    fn parsing_reads_the_baseline_changes_and_untracked_files() {
        let parsed = parse("@indexed\nM\tsrc/a.rs\nD\tsrc/old.rs\nA\tsrc/new.rs\nT\tbin/run\n@untracked\nnotes.md\n");
        assert_eq!(parsed.baseline, Baseline::Indexed);
        assert_eq!(parsed.files["src/a.rs"], Change::Modified);
        assert_eq!(parsed.files["src/old.rs"], Change::Deleted);
        assert_eq!(parsed.files["src/new.rs"], Change::Added);
        assert_eq!(parsed.files["bin/run"], Change::Modified);
        assert_eq!(parsed.files["notes.md"], Change::Added);
        assert_eq!(parse("@base\n@untracked\n").baseline, Baseline::BaseBranch);
    }

    #[test]
    fn paths_only_match_as_whole_paths() {
        assert!(mentions("found in src/lib.rs:12", "src/lib.rs"));
        assert!(mentions("{\"file\": \"src/lib.rs\"}", "src/lib.rs"));
        assert!(mentions("see src/lib.rs.", "src/lib.rs"));
        assert!(!mentions("found in src/lib.rs.bak", "src/lib.rs"));
        assert!(!mentions("found in app/src/lib.rs", "src/lib.rs"));
        assert!(!mentions("found in src/lib.rsx", "src/lib.rs"));
    }

    #[test]
    fn the_footer_lists_only_files_the_results_mention() {
        let d = drift(&[("src/a.rs", Change::Modified), ("src/b.rs", Change::Deleted), ("src/c.rs", Change::Added)]);
        let footer = d.footer("a in src/a.rs:3, b in src/b.rs:9", "app").unwrap();
        assert!(footer.contains("differ from the indexed version of app"));
        assert!(footer.contains("- src/a.rs (modified in your workspace)"));
        assert!(footer.contains("- src/b.rs (deleted in your workspace)"));
        assert!(!footer.contains("src/c.rs"));
        assert_eq!(d.footer("nothing relevant", "app"), None);
    }

    #[test]
    fn the_footer_caps_the_list_and_warns_when_far_behind() {
        let many: Vec<(String, Change)> = (0..600).map(|i| (format!("f{i}.rs"), Change::Modified)).collect();
        let d = Drift {
            baseline: Baseline::BaseBranch,
            files: many.into_iter().collect(),
        };
        let output: String = (0..30).map(|i| format!("f{i}.rs:1\n")).collect();
        let footer = d.footer(&output, "app").unwrap();
        assert!(footer.contains("the indexed commit is not in this workspace"));
        assert!(footer.contains("- and 10 more"));
        assert!(footer.contains("differs from your workspace in 600 files"));
        assert!(d.footer("unrelated", "app").unwrap().starts_with("The index differs"));
    }

    #[test]
    fn the_cache_is_dropped_on_invalidation_and_when_the_index_moves() {
        let cache = DriftCache::default();
        cache.put("s", Some("c1"), Arc::new(drift(&[])));
        assert!(cache.get("s", Some("c1")).is_some());
        assert!(cache.get("s", Some("c2")).is_none());
        cache.invalidate("s");
        assert!(cache.get("s", Some("c1")).is_none());
    }
}
