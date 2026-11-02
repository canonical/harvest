use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::store::Storage;

pub async fn set_aside(data_dir: &Path, stamp: &str) -> Result<PathBuf> {
    if !data_dir.exists() {
        bail!("{} does not exist: nothing to recover", data_dir.display());
    }
    drop(Storage::open(data_dir).await?);
    let name = data_dir
        .file_name()
        .context("the data directory has no name")?
        .to_string_lossy();
    let backup = data_dir.with_file_name(format!("{name}.before-recovery-{stamp}"));
    std::fs::rename(data_dir, &backup)
        .with_context(|| format!("moving {} to {}", data_dir.display(), backup.display()))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_data_directory_is_moved_aside() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("quorium");
        drop(Storage::open(&dir).await.unwrap());
        let backup = set_aside(&dir, "1").await.unwrap();
        assert!(!dir.exists());
        assert!(backup.join(crate::store::DATABASE_FILE).exists());
    }

    #[tokio::test]
    async fn a_running_server_blocks_recovery() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("quorium");
        let _running = Storage::open(&dir).await.unwrap();
        assert!(set_aside(&dir, "1").await.is_err());
        assert!(dir.exists());
    }

    #[tokio::test]
    async fn a_missing_directory_is_reported() {
        let parent = tempfile::tempdir().unwrap();
        assert!(set_aside(&parent.path().join("absent"), "1").await.is_err());
    }
}
