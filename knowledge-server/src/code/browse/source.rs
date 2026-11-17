use anyhow::Result;
use async_trait::async_trait;

use super::model::{RepoPath, TreeEntry};

#[async_trait]
pub trait RepositorySource: Send + Sync {
    async fn entries(&self, commit: &str, directory: &RepoPath) -> Result<Option<Vec<TreeEntry>>>;

    async fn file_size(&self, commit: &str, file: &RepoPath) -> Result<Option<u64>>;

    async fn file_bytes(&self, commit: &str, file: &RepoPath) -> Result<Vec<u8>>;
}
