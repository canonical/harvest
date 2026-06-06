use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server_url:  String,
    pub agent_token: String,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config: {}", path.display()))?;
        toml::from_str(&text).context("parsing config TOML")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("serialising config")?;
        std::fs::write(path, text)
            .with_context(|| format!("writing config: {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample() -> Config {
        Config {
            server_url:  "https://harvest.example.com".into(),
            agent_token: "tok-abc-123".into(),
        }
    }

    #[test]
    fn config_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = sample();
        cfg.save(&path).unwrap();

        let loaded = Config::from_file(&path).unwrap();
        assert_eq!(loaded.server_url, cfg.server_url);
        assert_eq!(loaded.agent_token, cfg.agent_token);
    }

    #[test]
    fn config_no_project_id_in_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        sample().save(&path).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("project_id"),
            "config must not write project_id (got: {text})"
        );
    }

    #[test]
    fn config_update_agent_token() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let mut cfg = sample();
        cfg.save(&path).unwrap();

        cfg.agent_token = "permanent-token-xyz".into();
        cfg.save(&path).unwrap();

        let loaded = Config::from_file(&path).unwrap();
        assert_eq!(loaded.agent_token, "permanent-token-xyz");
        assert_eq!(loaded.server_url, "https://harvest.example.com");
    }

    #[test]
    fn config_missing_file_returns_error() {
        let result = Config::from_file(Path::new("/nonexistent/path/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn config_malformed_toml_returns_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "not valid toml :::").unwrap();
        assert!(Config::from_file(&path).is_err());
    }
}
