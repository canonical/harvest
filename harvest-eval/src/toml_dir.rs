use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn read(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            Ok((path, text))
        })
        .collect()
}
