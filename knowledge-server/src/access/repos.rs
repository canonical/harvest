use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use harvest_db::Db;
use knowledge_harvester::git::{GitClient, GitCredentials, TagInfo};

use crate::credentials::remote::RemoteUrl;

pub const SCOPE_ALL_PARAM: &str = "_scope_all";
pub const SCOPE_REPOS_PARAM: &str = "_scope_repos";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoScope {
    All,
    Only(Arc<BTreeSet<String>>),
}

impl RepoScope {
    pub fn only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::Only(Arc::new(names.into_iter().map(Into::into).collect()))
    }

    pub fn allows(&self, repo: &str) -> bool {
        match self {
            Self::All => true,
            Self::Only(names) => names.contains(repo),
        }
    }

    pub fn is_all(&self) -> bool {
        matches!(self, Self::All)
    }

    pub fn sql_condition(column: &str) -> String {
        format!("(${SCOPE_ALL_PARAM}::bool OR {column} = ANY(${SCOPE_REPOS_PARAM}::text[]))")
    }

    pub fn write_into(&self, params: &mut Value) {
        let Some(object) = params.as_object_mut() else {
            return;
        };
        let names: Vec<&String> = match self {
            Self::All => Vec::new(),
            Self::Only(names) => names.iter().collect(),
        };
        object.insert(SCOPE_ALL_PARAM.into(), json!(self.is_all()));
        object.insert(SCOPE_REPOS_PARAM.into(), json!(names));
    }

    pub fn read_from(params: &Value) -> (bool, Vec<String>) {
        let all = params[SCOPE_ALL_PARAM].as_bool().unwrap_or(true);
        let names = params[SCOPE_REPOS_PARAM]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        (all, names)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
        }
    }
}

pub struct RemoteCheck {
    pub visibility: Visibility,
    pub refs: Vec<TagInfo>,
}

pub async fn check_remote(url: &str, credentials: GitCredentials) -> Result<RemoteCheck, String> {
    let anonymous = list_refs(url, GitCredentials::Anonymous).await;
    if let Ok(refs) = anonymous {
        return Ok(RemoteCheck {
            visibility: Visibility::Public,
            refs,
        });
    }
    if credentials == GitCredentials::Anonymous {
        return Err(
            "the repository is not public and you have no credentials for it; add an SSH key or a platform token in Settings"
                .into(),
        );
    }
    list_refs(url, credentials)
        .await
        .map(|refs| RemoteCheck {
            visibility: Visibility::Private,
            refs,
        })
        .map_err(|e| format!("your credentials cannot read this repository: {e}"))
}

async fn list_refs(url: &str, credentials: GitCredentials) -> Result<Vec<TagInfo>, String> {
    let url = url.to_string();
    let scratch = std::env::temp_dir().join("harvest-remote-checks");
    tokio::task::spawn_blocking(move || {
        GitClient::new(scratch)
            .with_credentials(credentials)
            .list_remote_refs(&url)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

pub struct RepoAccess<'a> {
    db: &'a Db,
}

impl<'a> RepoAccess<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub async fn scope_for(&self, user_id: &str) -> Result<RepoScope> {
        let rows = self
            .db
            .query(
                "SELECT role FROM users WHERE id = $uid",
                json!({ "uid": user_id }),
            )
            .await?;
        if rows.first().and_then(|r| r["role"].as_str()) == Some("admin") {
            return Ok(RepoScope::All);
        }
        let rows = self
            .db
            .query(
                "SELECT r.name FROM repositories r
                 WHERE r.visibility = 'public'
                    OR EXISTS (SELECT 1 FROM repository_access a
                               WHERE a.repository_id = r.id AND a.user_id = $uid)",
                json!({ "uid": user_id }),
            )
            .await?;
        Ok(RepoScope::only(
            rows.iter()
                .filter_map(|r| r["name"].as_str().map(String::from)),
        ))
    }

    pub async fn register(
        &self,
        repo: &str,
        url: &str,
        visibility: Visibility,
        user_id: &str,
    ) -> Result<()> {
        let host = RemoteUrl::parse(url).map(|r| r.host);
        self.db
            .execute(
                "INSERT INTO repositories (name, url, host, visibility)
                 VALUES ($repo, $url, $host, $visibility)
                 ON CONFLICT (name) DO UPDATE SET
                     url = EXCLUDED.url, host = EXCLUDED.host, visibility = EXCLUDED.visibility",
                json!({ "repo": repo, "url": url, "host": host, "visibility": visibility.as_str() }),
            )
            .await?;
        if visibility == Visibility::Private {
            self.grant(repo, user_id).await?;
        }
        Ok(())
    }

    pub async fn grant(&self, repo: &str, user_id: &str) -> Result<()> {
        self.db
            .execute(
                "INSERT INTO repository_access (repository_id, user_id, verified_at)
                 SELECT id, $uid, now() FROM repositories WHERE name = $repo
                 ON CONFLICT (repository_id, user_id) DO UPDATE SET verified_at = now()",
                json!({ "repo": repo, "uid": user_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn revoke(&self, repo: &str, user_id: &str) -> Result<()> {
        self.db
            .execute(
                "DELETE FROM repository_access a USING repositories r
                 WHERE a.repository_id = r.id AND r.name = $repo AND a.user_id = $uid",
                json!({ "repo": repo, "uid": user_id }),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restricted_scope_allows_only_its_repositories() {
        let scope = RepoScope::only(["cinder"]);
        assert!(scope.allows("cinder"));
        assert!(!scope.allows("secret"));
        assert!(RepoScope::All.allows("secret"));
    }

    #[test]
    fn the_sql_condition_reads_the_params_the_scope_writes() {
        assert_eq!(
            RepoScope::sql_condition("r.name"),
            "($_scope_all::bool OR r.name = ANY($_scope_repos::text[]))"
        );
    }

    #[test]
    fn the_scope_written_into_params_overrides_what_the_caller_sent() {
        let mut params =
            json!({ "query": "x", SCOPE_ALL_PARAM: true, SCOPE_REPOS_PARAM: ["secret"] });
        RepoScope::only(["cinder"]).write_into(&mut params);
        assert_eq!(
            RepoScope::read_from(&params),
            (false, vec!["cinder".to_string()])
        );
    }

    #[test]
    fn params_without_a_scope_read_as_unrestricted() {
        assert_eq!(RepoScope::read_from(&json!({})), (true, Vec::new()));
    }

    #[test]
    fn an_empty_scope_allows_nothing_rather_than_everything() {
        let mut params = json!({});
        RepoScope::only(Vec::<String>::new()).write_into(&mut params);
        assert_eq!(RepoScope::read_from(&params), (false, Vec::new()));
    }
}
