use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

use harvest_db::Db;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repository {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub default_branch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Version {
    pub name: String,
    pub commit: String,
}

pub async fn repository(db: &Db, name: &str) -> Result<Option<Repository>> {
    let rows = db
        .query(
            "SELECT id, name, url, default_branch FROM repositories WHERE name = $name AND url IS NOT NULL",
            json!({ "name": name }),
        )
        .await?;
    Ok(rows.first().map(|row| Repository {
        id: row["id"].as_i64().unwrap_or_default(),
        name: text(row, "name").unwrap_or_default(),
        url: text(row, "url").unwrap_or_default(),
        default_branch: text(row, "default_branch"),
    }))
}

pub async fn versions(db: &Db, repository: &Repository) -> Result<Vec<Version>> {
    let rows = db
        .query(
            "SELECT tag, commit_sha FROM versions
             WHERE repository_id = $id AND ingested AND commit_sha IS NOT NULL
             ORDER BY timestamp DESC NULLS LAST, tag",
            json!({ "id": repository.id }),
        )
        .await?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some(Version {
                name: text(row, "tag")?,
                commit: text(row, "commit_sha")?,
            })
        })
        .collect())
}

pub fn default_version<'a>(repository: &Repository, versions: &'a [Version]) -> Option<&'a Version> {
    let default_branch = repository.default_branch.as_deref();
    versions
        .iter()
        .find(|version| Some(version.name.as_str()) == default_branch)
        .or_else(|| versions.first())
}

fn text(row: &Value, key: &str) -> Option<String> {
    row[key].as_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(name: &str) -> Version {
        Version { name: name.into(), commit: format!("{name}-sha") }
    }

    fn repository(default_branch: Option<&str>) -> Repository {
        Repository { id: 1, name: "flask".into(), url: "https://x/flask".into(), default_branch: default_branch.map(String::from) }
    }

    #[test]
    fn the_default_version_is_the_default_branch_when_it_was_ingested() {
        let versions = [version("v2.0"), version("main")];
        assert_eq!(default_version(&repository(Some("main")), &versions), Some(&versions[1]));
    }

    #[test]
    fn otherwise_the_default_version_is_the_latest_ingested() {
        let versions = [version("v2.0"), version("v1.0")];
        assert_eq!(default_version(&repository(Some("main")), &versions), Some(&versions[0]));
        assert_eq!(default_version(&repository(None), &versions), Some(&versions[0]));
        assert_eq!(default_version(&repository(None), &[]), None);
    }
}
