pub mod clones;
pub mod handlers;
pub mod runtime;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use harvest_db::Db;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Web,
    Api,
    Cli,
}

impl Kind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "web" => Some(Self::Web),
            "api" => Some(Self::Api),
            "cli" => Some(Self::Cli),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Api => "api",
            Self::Cli => "cli",
        }
    }

    pub fn needs_port(self) -> bool {
        !matches!(self, Self::Cli)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Audience {
    Owner,
    Repository,
    Link,
}

impl Audience {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "repository" => Some(Self::Repository),
            "link" => Some(Self::Link),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Repository => "repository",
            Self::Link => "link",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    User { id: String, repo_allowed: bool },
    Anonymous(String),
}

impl Viewer {
    pub fn key(&self) -> String {
        match self {
            Self::User { id, .. } => format!("user:{id}"),
            Self::Anonymous(token) => format!("anon:{token}"),
        }
    }

    pub fn is_user(&self, user_id: &str) -> bool {
        matches!(self, Self::User { id, .. } if id == user_id)
    }
}

pub fn may_view(preview: &Preview, viewer: &Viewer) -> bool {
    if viewer.is_user(&preview.owner_id) {
        return true;
    }
    match preview.audience {
        Audience::Owner => false,
        Audience::Repository => matches!(viewer, Viewer::User { repo_allowed: true, .. }),
        Audience::Link => true,
    }
}

pub fn base_path(preview_id: &str) -> String {
    format!("/previews/{preview_id}/web/")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub id: String,
    pub session_id: String,
    pub kind: Kind,
    pub port: Option<u16>,
    pub start_command: Option<String>,
    pub audience: Audience,
    pub owner_id: String,
    pub project_name: String,
    pub branch: String,
    pub session_status: String,
    #[serde(skip)]
    pub session_container: Option<String>,
}

const SELECT: &str = "SELECT p.id, p.session_id, p.kind, p.port, p.start_command, p.audience,
           s.user_id AS owner_id, s.branch, s.status AS session_status, s.container AS session_container,
           r.name AS project_name
    FROM code_previews p
    JOIN code_sessions s ON s.id = p.session_id
    JOIN code_projects cp ON cp.id = s.project_id
    JOIN repositories r ON r.id = cp.repository_id";

fn from_row(row: &Value) -> Option<Preview> {
    let text = |key: &str| row[key].as_str().map(String::from);
    Some(Preview {
        id: text("id")?,
        session_id: text("session_id")?,
        kind: Kind::parse(row["kind"].as_str()?)?,
        port: row["port"].as_u64().and_then(|p| u16::try_from(p).ok()),
        start_command: text("start_command"),
        audience: Audience::parse(row["audience"].as_str()?)?,
        owner_id: text("owner_id")?,
        project_name: text("project_name").unwrap_or_default(),
        branch: text("branch").unwrap_or_default(),
        session_status: text("session_status").unwrap_or_default(),
        session_container: text("session_container"),
    })
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Preview>> {
    let rows = db
        .query(&format!("{SELECT} WHERE p.id = $id"), json!({ "id": id }))
        .await?;
    Ok(rows.first().and_then(from_row))
}

pub async fn for_session(db: &Db, session_id: &str) -> Result<Option<Preview>> {
    let rows = db
        .query(&format!("{SELECT} WHERE p.session_id = $sid"), json!({ "sid": session_id }))
        .await?;
    Ok(rows.first().and_then(from_row))
}

pub struct Settings<'a> {
    pub kind: Kind,
    pub port: Option<u16>,
    pub start_command: Option<&'a str>,
}

pub async fn configure(db: &Db, session_id: &str, settings: Settings<'_>) -> Result<Preview> {
    if settings.kind.needs_port() && settings.port.is_none() {
        bail!("a {} preview needs the port the app listens on", settings.kind.as_str());
    }
    db.execute(
        "INSERT INTO code_previews (id, session_id, kind, port, start_command)
         VALUES ($id, $sid, $kind, $port, $command)
         ON CONFLICT (session_id) DO UPDATE SET
             kind = EXCLUDED.kind,
             port = EXCLUDED.port,
             start_command = EXCLUDED.start_command,
             updated_at = now()",
        json!({
            "id": Uuid::new_v4().simple().to_string(),
            "sid": session_id,
            "kind": settings.kind.as_str(),
            "port": settings.port,
            "command": settings.start_command.map(str::trim).filter(|c| !c.is_empty()),
        }),
    )
    .await?;
    for_session(db, session_id)
        .await?
        .context("the preview was not saved")
}

pub async fn set_audience(db: &Db, session_id: &str, audience: Audience) -> Result<()> {
    db.execute(
        "UPDATE code_previews SET audience = $audience, updated_at = now() WHERE session_id = $sid",
        json!({ "sid": session_id, "audience": audience.as_str() }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(audience: Audience) -> Preview {
        Preview {
            id: "p".into(),
            session_id: "s".into(),
            kind: Kind::Web,
            port: Some(3000),
            start_command: None,
            audience,
            owner_id: "owner".into(),
            project_name: "repo".into(),
            branch: "b".into(),
            session_status: "active".into(),
            session_container: None,
        }
    }

    fn user(id: &str, repo_allowed: bool) -> Viewer {
        Viewer::User { id: id.into(), repo_allowed }
    }

    #[test]
    fn the_owner_always_sees_their_preview() {
        for audience in [Audience::Owner, Audience::Repository, Audience::Link] {
            assert!(may_view(&preview(audience), &user("owner", false)));
        }
    }

    #[test]
    fn repository_previews_need_access_to_the_repository() {
        let shared = preview(Audience::Repository);
        assert!(may_view(&shared, &user("colleague", true)));
        assert!(!may_view(&shared, &user("stranger", false)));
        assert!(!may_view(&shared, &Viewer::Anonymous("x".into())));
    }

    #[test]
    fn link_previews_open_for_anyone_and_private_ones_for_nobody_else() {
        assert!(may_view(&preview(Audience::Link), &Viewer::Anonymous("x".into())));
        assert!(!may_view(&preview(Audience::Owner), &user("colleague", true)));
    }

    #[test]
    fn viewers_get_distinct_keys() {
        assert_eq!(user("u", true).key(), "user:u");
        assert_eq!(Viewer::Anonymous("t".into()).key(), "anon:t");
    }

    #[test]
    fn only_cli_previews_work_without_a_port() {
        assert!(Kind::Web.needs_port());
        assert!(Kind::Api.needs_port());
        assert!(!Kind::Cli.needs_port());
    }
}
