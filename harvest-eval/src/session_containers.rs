use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use harvest_db::Db;
use knowledge_server::config::Config;
use knowledge_server::lxd::containers::Command;
use knowledge_server::lxd::LxdClient;
use serde_json::json;

const WORKSPACE: &str = "/workspace";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct SessionContainers {
    pub lxd: LxdClient,
    pub db: Arc<Db>,
}

pub struct Ran {
    pub succeeded: bool,
    pub output: String,
}

impl SessionContainers {
    pub async fn from_config(path: &Path) -> Result<Self> {
        let config = Config::from_file(path)?;
        let db = Arc::new(Db::connect(&config.database.url).await?);
        let lxd_config = config.lxd.as_ref().context("the server configuration has no [lxd] section")?;
        let lxd = knowledge_server::lxd::resolve_client(lxd_config, &db)
            .await?
            .context("Harvest has no trusted LXD identity yet")?;
        Ok(Self { lxd, db })
    }

    pub async fn of_session(&self, session: &str) -> Result<String> {
        let rows = self
            .db
            .query("SELECT container FROM code_sessions WHERE id = $id", json!({ "id": session }))
            .await?;
        rows.first()
            .and_then(|row| row["container"].as_str())
            .map(String::from)
            .context("the session has no container")
    }

    pub async fn run(&self, container: &str, script: &str) -> Result<Ran> {
        let output = self
            .lxd
            .run(container, Command::shell(script).in_dir(WORKSPACE).timeout(COMMAND_TIMEOUT))
            .await?;
        Ok(Ran {
            succeeded: output.succeeded(),
            output: format!("{}{}", output.stdout, output.stderr),
        })
    }
}
