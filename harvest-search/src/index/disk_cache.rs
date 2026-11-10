use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::domain::Scope;
use crate::index::corpus::Corpus;

#[derive(Clone, Debug)]
pub struct DiskCache {
    root: PathBuf,
}

fn component(value: &str) -> String {
    let safe: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut hash: u32 = 0x811c_9dc5;
    for byte in value.as_bytes() {
        hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
    }
    format!("{}-{hash:08x}", safe.trim_start_matches('.'))
}

impl DiskCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self, scope: &Scope) -> PathBuf {
        self.root
            .join(component(&scope.repo))
            .join(component(&scope.version))
    }

    fn file(&self, scope: &Scope, generation: &str) -> PathBuf {
        self.dir(scope)
            .join(format!("{}.json", component(generation)))
    }

    pub async fn read(&self, scope: &Scope, generation: &str) -> Option<Corpus> {
        read_file(&self.file(scope, generation)).await
    }

    pub async fn latest(&self, scope: &Scope) -> Option<Corpus> {
        let mut entries = tokio::fs::read_dir(self.dir(scope)).await.ok()?;
        let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let modified = entry.metadata().await.ok()?.modified().ok()?;
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, path));
            }
        }
        read_file(&newest?.1).await
    }

    pub async fn write(&self, corpus: &Corpus) -> Result<()> {
        let dir = self.dir(&corpus.scope);
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("creating {}", dir.display()))?;
        let target = self.file(&corpus.scope, &corpus.generation);
        let temporary = target.with_extension("json.tmp");
        let bytes = serde_json::to_vec(corpus)?;
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, &target).await?;
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.path() != target {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
        Ok(())
    }
}

async fn read_file(path: &Path) -> Option<Corpus> {
    let bytes = tokio::fs::read(path).await.ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(corpus) => Some(corpus),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "discarding a corrupt index cache file");
            let _ = tokio::fs::remove_file(path).await;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::corpus::Unit;

    fn corpus(generation: &str) -> Corpus {
        let mut corpus = Corpus::new(Scope::new("org/repo", "v1.0"), generation);
        corpus.push_unit(Unit {
            name: "f".into(),
            ..Unit::default()
        });
        corpus
    }

    #[tokio::test]
    async fn writes_replace_older_generations_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path());
        let scope = Scope::new("org/repo", "v1.0");
        cache.write(&corpus("g1")).await.unwrap();
        cache.write(&corpus("g2")).await.unwrap();
        assert!(cache.read(&scope, "g1").await.is_none());
        assert_eq!(cache.read(&scope, "g2").await.unwrap().units.len(), 1);
        assert_eq!(cache.latest(&scope).await.unwrap().generation, "g2");
    }

    #[tokio::test]
    async fn corrupt_files_are_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCache::new(dir.path());
        let scope = Scope::new("r", "v");
        cache.write(&corpus("g")).await.unwrap();
        let path = cache.file(&Scope::new("org/repo", "v1.0"), "g");
        tokio::fs::write(&path, b"{not json").await.unwrap();
        assert!(cache
            .read(&Scope::new("org/repo", "v1.0"), "g")
            .await
            .is_none());
        assert!(!path.exists());
        assert!(cache.latest(&scope).await.is_none());
    }
}
