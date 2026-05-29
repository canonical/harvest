/// Unit/integration tests for GitClient.
/// All tests run in-process with temporary directories — no Docker or network required.
use std::path::Path;

use git2::{Repository, Signature};
use knowledge_harvester::git::GitClient;
use knowledge_harvester::config::RepoConfig;
use tempfile::TempDir;

// ── fixture helpers ───────────────────────────────────────────────────────────

struct FixtureRepo {
    dir: TempDir,
}

impl FixtureRepo {
    /// Create a git repo with two commits, each tagged, plus a .gitignore.
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        let repo = Repository::init(path).unwrap();
        let sig = Signature::now("test", "test@example.com").unwrap();

        // ── commit 1: add a Rust source file and a gitignored file ──
        {
            std::fs::write(path.join("main.rs"), "fn main() {}").unwrap();
            std::fs::write(path.join(".gitignore"), "target/\n*.log\n").unwrap();
            std::fs::create_dir_all(path.join("target")).unwrap();
            std::fs::write(path.join("target/build.rs"), "ignored").unwrap();
            std::fs::write(path.join("debug.log"), "also ignored").unwrap();

            let mut idx = repo.index().unwrap();
            idx.add_path(Path::new("main.rs")).unwrap();
            idx.add_path(Path::new(".gitignore")).unwrap();
            idx.write().unwrap();
            let tree_id = idx.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "first commit", &tree, &[])
                .unwrap();
        }

        // tag v0.1.0 pointing at first commit
        let head1 = repo.head().unwrap().peel_to_commit().unwrap();
        repo.tag_lightweight("v0.1.0", head1.as_object(), false).unwrap();

        // ── commit 2: add a second file ──
        {
            std::fs::write(path.join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();
            let mut idx = repo.index().unwrap();
            idx.add_path(Path::new("lib.rs")).unwrap();
            idx.write().unwrap();
            let tree_id = idx.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let parent = repo.head().unwrap().peel_to_commit().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "second commit", &tree, &[&parent])
                .unwrap();
        }

        // tag v0.2.0 pointing at second commit
        let head2 = repo.head().unwrap().peel_to_commit().unwrap();
        repo.tag_lightweight("v0.2.0", head2.as_object(), false).unwrap();

        FixtureRepo { dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn url(&self) -> String {
        // git2 accepts local filesystem paths as clone URLs
        self.dir.path().to_string_lossy().into_owned()
    }
}

fn make_client() -> (GitClient, TempDir) {
    let clone_root = TempDir::new().unwrap();
    let client = GitClient::new(clone_root.path().to_path_buf());
    (client, clone_root)
}

// ── list_tags ─────────────────────────────────────────────────────────────────

#[test]
fn list_tags_returns_both_tags() {
    let fixture = FixtureRepo::new();
    let tags = GitClient::new(fixture.path().to_path_buf())
        .list_tags(fixture.path())
        .unwrap();
    let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"v0.1.0"), "tags: {names:?}");
    assert!(names.contains(&"v0.2.0"), "tags: {names:?}");
}

#[test]
fn list_tags_sorted_by_timestamp() {
    let fixture = FixtureRepo::new();
    let tags = GitClient::new(fixture.path().to_path_buf())
        .list_tags(fixture.path())
        .unwrap();
    // timestamps must be non-decreasing
    for w in tags.windows(2) {
        assert!(w[0].timestamp <= w[1].timestamp, "tags out of order: {:?}", tags);
    }
}

#[test]
fn list_tags_has_commit_sha() {
    let fixture = FixtureRepo::new();
    let tags = GitClient::new(fixture.path().to_path_buf())
        .list_tags(fixture.path())
        .unwrap();
    for tag in &tags {
        assert_eq!(tag.commit_sha.len(), 40, "SHA should be 40 hex chars: {}", tag.commit_sha);
        assert!(tag.commit_sha.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

#[test]
fn list_tags_empty_on_untagged_repo() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let sig = Signature::now("test", "test@example.com").unwrap();
    let mut idx = repo.index().unwrap();
    idx.write().unwrap();
    let tree_id = idx.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[]).unwrap();

    let tags = GitClient::new(dir.path().to_path_buf())
        .list_tags(dir.path())
        .unwrap();
    assert!(tags.is_empty());
}

// ── checkout ──────────────────────────────────────────────────────────────────

#[test]
fn checkout_v1_does_not_have_v2_file() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());

    client.checkout(fixture.path(), "v0.1.0").unwrap();
    assert!(!fixture.path().join("lib.rs").exists(),
        "lib.rs should not exist at v0.1.0");
    assert!(fixture.path().join("main.rs").exists());
}

#[test]
fn checkout_v2_has_both_files() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());

    client.checkout(fixture.path(), "v0.2.0").unwrap();
    assert!(fixture.path().join("main.rs").exists());
    assert!(fixture.path().join("lib.rs").exists());
}

#[test]
fn checkout_sets_head_to_correct_commit() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());

    // Record the SHA that v0.1.0 resolves to
    let repo = Repository::open(fixture.path()).unwrap();
    let v1_sha = repo.revparse_single("refs/tags/v0.1.0")
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .id()
        .to_string();

    client.checkout(fixture.path(), "v0.1.0").unwrap();
    let head_sha = repo.head().unwrap().peel_to_commit().unwrap().id().to_string();
    assert_eq!(head_sha, v1_sha);
}

#[test]
fn checkout_unknown_tag_returns_error() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());
    let result = client.checkout(fixture.path(), "v99.0.0");
    assert!(result.is_err());
}

// ── ensure_cloned ─────────────────────────────────────────────────────────────

#[test]
fn ensure_cloned_creates_repo_directory() {
    let fixture = FixtureRepo::new();
    let (client, _clone_root) = make_client();
    let cfg = RepoConfig { name: "myrepo".into(), url: fixture.url(), refs: None };

    let path = client.ensure_cloned(&cfg).unwrap();
    assert!(path.exists(), "clone directory should exist");
    assert!(path.join(".git").exists(), "should be a git repo");
}

#[test]
fn ensure_cloned_twice_does_not_error() {
    let fixture = FixtureRepo::new();
    let (client, _clone_root) = make_client();
    let cfg = RepoConfig { name: "myrepo".into(), url: fixture.url(), refs: None };

    client.ensure_cloned(&cfg).unwrap();
    // Second call should fetch, not re-clone, and succeed
    client.ensure_cloned(&cfg).unwrap();
}

#[test]
fn ensure_cloned_returns_correct_path() {
    let fixture = FixtureRepo::new();
    let (client, clone_root) = make_client();
    let cfg = RepoConfig { name: "testrepo".into(), url: fixture.url(), refs: None };

    let path = client.ensure_cloned(&cfg).unwrap();
    assert_eq!(path, clone_root.path().join("testrepo"));
}

#[test]
fn ensure_cloned_repo_has_expected_tags() {
    let fixture = FixtureRepo::new();
    let (client, _clone_root) = make_client();
    let cfg = RepoConfig { name: "myrepo".into(), url: fixture.url(), refs: None };

    let path = client.ensure_cloned(&cfg).unwrap();
    let tags = client.list_tags(&path).unwrap();
    let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"v0.1.0"));
    assert!(names.contains(&"v0.2.0"));
}

// ── walk_source_files ─────────────────────────────────────────────────────────

#[test]
fn walk_returns_rs_files() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());

    let files = client.walk_source_files(fixture.path()).unwrap();
    let paths: Vec<_> = files.iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert!(paths.contains(&"main.rs".to_string()), "files: {paths:?}");
}

#[test]
fn walk_respects_gitignore() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());

    let files = client.walk_source_files(fixture.path()).unwrap();
    let names: Vec<_> = files.iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert!(!names.contains(&"build.rs".to_string()),
        "target/build.rs should be excluded by .gitignore; got: {names:?}");
    assert!(!names.contains(&"debug.log".to_string()),
        "debug.log should be excluded by .gitignore; got: {names:?}");
}

#[test]
fn walk_does_not_return_git_internals() {
    let fixture = FixtureRepo::new();
    let client = GitClient::new(fixture.path().to_path_buf());

    let files = client.walk_source_files(fixture.path()).unwrap();
    for path in &files {
        let s = path.to_string_lossy();
        assert!(!s.contains("/.git/"),
            "walked into .git directory: {s}");
    }
}
