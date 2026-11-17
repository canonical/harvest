use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use knowledge_harvester::git::GitCredentials;
use uuid::Uuid;

use super::catalog::Repository;
use super::git::Git;
use super::model::{parse_tree, EntryKind, RepoPath, TreeEntry};
use super::source::RepositorySource;

const BRANCHES_AND_TAGS: [&str; 2] = ["+refs/heads/*:refs/heads/*", "+refs/tags/*:refs/tags/*"];

pub struct Mirrors {
    root: PathBuf,
    fetch_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
}

impl Mirrors {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            fetch_locks: Mutex::default(),
        }
    }

    pub async fn open(&self, repository: &Repository, commit: &str, credentials: GitCredentials) -> Result<Mirror> {
        let mirror = Mirror {
            directory: self.root.join(format!("{}.git", repository.id)),
            credentials,
        };
        let fetch_lock = self.fetch_lock(repository.id);
        let _only_one_fetch_per_repository = fetch_lock.lock().await;
        if !mirror.directory.exists() {
            mirror.clone_from(&repository.url, &self.root).await?;
        }
        if !mirror.has_commit(commit).await? {
            mirror.fetch(commit).await?;
        }
        Ok(mirror)
    }

    fn fetch_lock(&self, repository_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.fetch_locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(locks.entry(repository_id).or_default())
    }
}

pub struct Mirror {
    directory: PathBuf,
    credentials: GitCredentials,
}

impl Mirror {
    fn git(&self) -> Git<'_> {
        Git::new(&self.credentials)
    }

    async fn clone_from(&self, url: &str, root: &Path) -> Result<()> {
        tokio::fs::create_dir_all(root).await.context("creating the mirror directory")?;
        let partial = root.join(format!(".partial-{}", Uuid::new_v4()));
        let partial_path = partial.to_str().context("the mirror path is not valid UTF-8")?;
        let cloned = self
            .git()
            .run(&["clone", "--bare", "--filter=blob:none", "--quiet", "--", url, partial_path])
            .await;
        if let Err(error) = cloned {
            let _ = tokio::fs::remove_dir_all(&partial).await;
            return Err(error.context("cloning the repository"));
        }
        tokio::fs::rename(&partial, &self.directory).await.context("storing the mirror")
    }

    async fn has_commit(&self, commit: &str) -> Result<bool> {
        let object = format!("{commit}^{{commit}}");
        self.git().succeeds_in(&self.directory, &["cat-file", "-e", &object]).await
    }

    async fn fetch(&self, commit: &str) -> Result<()> {
        let mut fetch_branches_and_tags = vec!["fetch", "--quiet", "--filter=blob:none", "origin"];
        fetch_branches_and_tags.extend(BRANCHES_AND_TAGS);
        self.git().run_in(&self.directory, &fetch_branches_and_tags).await?;
        if self.has_commit(commit).await? {
            return Ok(());
        }
        self.git()
            .run_in(&self.directory, &["fetch", "--quiet", "--filter=blob:none", "origin", commit])
            .await
            .context("fetching the version's commit")?;
        if !self.has_commit(commit).await? {
            bail!("the repository no longer has commit {commit}");
        }
        Ok(())
    }

    async fn kind_of(&self, commit: &str, path: &RepoPath) -> Result<Option<EntryKind>> {
        if path.is_root() {
            return Ok(Some(EntryKind::Directory));
        }
        let listing = self
            .git()
            .run_in(&self.directory, &["ls-tree", "-z", commit, "--", path.as_str()])
            .await?;
        let entries = parse_tree(&listing, &RepoPath::root());
        Ok(entries.into_iter().find(|entry| entry.path == path.as_str()).map(|entry| entry.kind))
    }
}

#[async_trait]
impl RepositorySource for Mirror {
    async fn entries(&self, commit: &str, directory: &RepoPath) -> Result<Option<Vec<TreeEntry>>> {
        if self.kind_of(commit, directory).await? != Some(EntryKind::Directory) {
            return Ok(None);
        }
        let listing = self
            .git()
            .run_in(&self.directory, &["ls-tree", "-z", &directory.at(commit)])
            .await?;
        Ok(Some(parse_tree(&listing, directory)))
    }

    async fn file_size(&self, commit: &str, file: &RepoPath) -> Result<Option<u64>> {
        if !matches!(self.kind_of(commit, file).await?, Some(EntryKind::File | EntryKind::Symlink)) {
            return Ok(None);
        }
        let size = self
            .git()
            .run_in(&self.directory, &["cat-file", "-s", &file.at(commit)])
            .await?;
        let size = String::from_utf8_lossy(&size).trim().parse().context("reading the file size")?;
        Ok(Some(size))
    }

    async fn file_bytes(&self, commit: &str, file: &RepoPath) -> Result<Vec<u8>> {
        self.git()
            .run_in(&self.directory, &["cat-file", "blob", &file.at(commit)])
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    struct Upstream {
        directory: tempfile::TempDir,
    }

    impl Upstream {
        fn new() -> Self {
            let upstream = Self { directory: tempfile::tempdir().unwrap() };
            upstream.git(&["init", "--quiet", "--initial-branch=main"]);
            upstream
        }

        fn write(&self, path: &str, contents: &[u8]) {
            let file = self.directory.path().join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }

        fn commit(&self) -> String {
            self.git(&["add", "-A"]);
            self.git(&["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--quiet", "-m", "change"]);
            self.git(&["rev-parse", "HEAD"]).trim().to_string()
        }

        fn git(&self, args: &[&str]) -> String {
            let output = Command::new("git").arg("-C").arg(self.directory.path()).args(args).output().unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            String::from_utf8(output.stdout).unwrap()
        }

        fn repository(&self) -> Repository {
            Repository {
                id: 7,
                name: "sample".into(),
                url: self.directory.path().to_str().unwrap().into(),
                default_branch: Some("main".into()),
            }
        }
    }

    fn path(raw: &str) -> RepoPath {
        RepoPath::parse(raw).unwrap()
    }

    fn names(entries: &[TreeEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[tokio::test]
    async fn a_mirror_lists_directories_and_reads_files_at_a_commit() {
        let upstream = Upstream::new();
        upstream.write("README.md", b"# Sample\n");
        upstream.write("src/main.rs", b"fn main() {}\n");
        let commit = upstream.commit();
        let root = tempfile::tempdir().unwrap();
        let mirrors = Mirrors::new(root.path().join("mirrors"));

        let mirror = mirrors.open(&upstream.repository(), &commit, GitCredentials::Anonymous).await.unwrap();

        let top = mirror.entries(&commit, &RepoPath::root()).await.unwrap().unwrap();
        assert_eq!(names(&top), ["src", "README.md"]);
        let source = mirror.entries(&commit, &path("src")).await.unwrap().unwrap();
        assert_eq!(source[0].path, "src/main.rs");
        assert_eq!(mirror.file_size(&commit, &path("src/main.rs")).await.unwrap(), Some(13));
        assert_eq!(mirror.file_bytes(&commit, &path("README.md")).await.unwrap(), b"# Sample\n");
        assert!(root.path().join("mirrors/7.git").is_dir());
    }

    #[tokio::test]
    async fn paths_that_are_not_there_or_of_another_kind_are_not_found() {
        let upstream = Upstream::new();
        upstream.write("src/main.rs", b"fn main() {}\n");
        let commit = upstream.commit();
        let root = tempfile::tempdir().unwrap();
        let mirror = Mirrors::new(root.path().to_path_buf())
            .open(&upstream.repository(), &commit, GitCredentials::Anonymous)
            .await
            .unwrap();

        assert_eq!(mirror.entries(&commit, &path("missing")).await.unwrap(), None);
        assert_eq!(mirror.entries(&commit, &path("src/main.rs")).await.unwrap(), None);
        assert_eq!(mirror.file_size(&commit, &path("src")).await.unwrap(), None);
        assert_eq!(mirror.file_size(&commit, &path("src/missing.rs")).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_commit_made_after_the_mirror_was_created_is_fetched() {
        let upstream = Upstream::new();
        upstream.write("a.txt", b"first\n");
        let first = upstream.commit();
        let root = tempfile::tempdir().unwrap();
        let mirrors = Mirrors::new(root.path().to_path_buf());
        mirrors.open(&upstream.repository(), &first, GitCredentials::Anonymous).await.unwrap();

        upstream.write("a.txt", b"second\n");
        let second = upstream.commit();
        let mirror = mirrors.open(&upstream.repository(), &second, GitCredentials::Anonymous).await.unwrap();

        assert_eq!(mirror.file_bytes(&second, &path("a.txt")).await.unwrap(), b"second\n");
        assert_eq!(mirror.file_bytes(&first, &path("a.txt")).await.unwrap(), b"first\n");
    }

    #[tokio::test]
    async fn an_unknown_commit_is_an_error() {
        let upstream = Upstream::new();
        upstream.write("a.txt", b"first\n");
        upstream.commit();
        let root = tempfile::tempdir().unwrap();
        let missing = "0123456789abcdef0123456789abcdef01234567";

        let opened = Mirrors::new(root.path().to_path_buf())
            .open(&upstream.repository(), missing, GitCredentials::Anonymous)
            .await;

        assert!(opened.is_err());
    }
}
