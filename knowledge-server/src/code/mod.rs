pub mod briefing;
pub mod change_requests;
pub mod freshness;
pub mod handlers;
pub mod images;
pub mod index;
pub mod keeper;
pub mod overlay;
pub mod previews;
pub mod projects;
pub mod repo_git;
pub mod sessions;
pub mod workspace;
pub mod workspace_search;

use std::sync::Arc;
use std::time::Duration;

use knowledge_harvester::git::GitCredentials;

use harvest_db::Db;

use crate::config::CodeConfig;
use crate::credentials::CredentialStore;
use crate::lxd::LxdClient;

pub const WORKSPACE: &str = "/workspace";

#[derive(Clone, Debug)]
pub struct CodeSettings {
    pub pool_size: usize,
    pub warm_for: Duration,
    pub session_idle: Duration,
    pub max_preview_copies: usize,
    pub reindex_every: Duration,
}

impl From<&CodeConfig> for CodeSettings {
    fn from(config: &CodeConfig) -> Self {
        const HOUR: u64 = 60 * 60;
        Self {
            pool_size: config.pool_size,
            warm_for: Duration::from_secs(config.warm_days * 24 * HOUR),
            session_idle: Duration::from_secs(config.idle_hours * HOUR),
            max_preview_copies: config.max_preview_copies,
            reindex_every: Duration::from_secs(config.reindex_hours * HOUR),
        }
    }
}

#[derive(Clone)]
pub struct CodeEnv {
    pub db: Arc<Db>,
    pub lxd: Arc<LxdClient>,
    pub credentials: Option<Arc<CredentialStore>>,
    pub settings: CodeSettings,
    pub drift: Arc<freshness::DriftCache>,
}

impl CodeEnv {
    pub async fn git_credentials(&self, user_id: &str, url: &str) -> GitCredentials {
        let Some(store) = &self.credentials else {
            return GitCredentials::Anonymous;
        };
        store.git_credentials(user_id, url).await.unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not load git credentials");
            GitCredentials::Anonymous
        })
    }
}

pub fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}
