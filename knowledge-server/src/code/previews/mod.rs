pub mod clones;
pub mod diagnosis;
pub mod framing;
pub mod handlers;
pub mod origin;
pub mod owner;
pub mod pass;
pub mod proxy;
pub mod runtime;
pub mod site;
pub mod visit;

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use harvest_db::Db;

use super::CodeEnv;

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
    pub conversation_id: Option<String>,
    #[serde(skip)]
    pub session_container: Option<String>,
}

const SELECT: &str = "SELECT p.id, p.session_id, p.kind, p.port, p.start_command, p.audience, p.conversation_id,
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
        conversation_id: text("conversation_id"),
        session_container: text("session_container"),
    })
}

pub async fn get(db: &Db, id: &str) -> Result<Option<Preview>> {
    let rows = db
        .query(&format!("{SELECT} WHERE p.id = $id"), json!({ "id": id }))
        .await?;
    Ok(rows.first().and_then(from_row))
}

pub async fn list(db: &Db, session_id: &str, lifetime: Duration) -> Result<Vec<Value>> {
    Ok(db
        .query(
            "SELECT p.id, p.kind, p.port, p.start_command, p.audience, p.conversation_id,
                    c.title AS chat_title, p.created_at, p.last_used_at,
                    p.last_used_at + make_interval(secs => $lifetime::float8) AS expires_at
             FROM code_previews p LEFT JOIN conversations c ON c.id = p.conversation_id
             WHERE p.session_id = $sid ORDER BY p.created_at DESC",
            json!({ "sid": session_id, "lifetime": lifetime.as_secs_f64() }),
        )
        .await?)
}

#[derive(Clone, Copy)]
pub enum Slot<'a> {
    New,
    Existing(&'a str),
    LatestOfChat(&'a str),
}

pub struct Settings<'a> {
    pub kind: Kind,
    pub port: Option<u16>,
    pub start_command: Option<&'a str>,
}

pub async fn configure(
    db: &Db,
    session_id: &str,
    chat_id: Option<&str>,
    slot: Slot<'_>,
    settings: Settings<'_>,
) -> Result<Preview> {
    if settings.kind.needs_port() && settings.port.is_none() {
        bail!("a {} preview needs the port the app listens on", settings.kind.as_str());
    }
    let existing = resolve(db, session_id, slot).await?;
    if let Some(port) = settings.port.filter(|_| settings.kind.needs_port()) {
        ensure_port_free(db, session_id, existing.as_deref(), port).await?;
    }
    let id = existing.unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    db.execute(
        "INSERT INTO code_previews (id, session_id, kind, port, start_command, conversation_id)
         VALUES ($id, $sid, $kind, $port, $command, $chat)
         ON CONFLICT (id) DO UPDATE SET
             kind = EXCLUDED.kind,
             port = EXCLUDED.port,
             start_command = EXCLUDED.start_command,
             updated_at = now(),
             last_used_at = now()",
        json!({
            "id": id,
            "sid": session_id,
            "kind": settings.kind.as_str(),
            "port": settings.port.filter(|_| settings.kind.needs_port()),
            "command": settings.start_command.map(str::trim).filter(|c| !c.is_empty()),
            "chat": chat_id,
        }),
    )
    .await?;
    get(db, &id).await?.context("the preview was not saved")
}

async fn resolve(db: &Db, session_id: &str, slot: Slot<'_>) -> Result<Option<String>> {
    let (sql, params) = match slot {
        Slot::New => return Ok(None),
        Slot::Existing(id) => (
            "SELECT id FROM code_previews WHERE id = $id AND session_id = $sid",
            json!({ "id": id, "sid": session_id }),
        ),
        Slot::LatestOfChat(chat_id) => (
            "SELECT id FROM code_previews WHERE conversation_id = $chat AND session_id = $sid
             ORDER BY created_at DESC LIMIT 1",
            json!({ "chat": chat_id, "sid": session_id }),
        ),
    };
    let found = db.query(sql, params).await?.first().and_then(|r| r["id"].as_str().map(String::from));
    if found.is_none() && matches!(slot, Slot::Existing(_)) {
        bail!("this session has no such preview");
    }
    Ok(found)
}

async fn ensure_port_free(db: &Db, session_id: &str, preview_id: Option<&str>, port: u16) -> Result<()> {
    let taken = db
        .query(
            "SELECT id FROM code_previews
             WHERE session_id = $sid AND port = $port AND id IS DISTINCT FROM $id",
            json!({ "sid": session_id, "port": port, "id": preview_id }),
        )
        .await?;
    match taken.first().and_then(|r| r["id"].as_str()) {
        Some(other) => bail!(
            "port {port} already belongs to preview {other} of this session; pick another port, or update that preview by passing its preview_id"
        ),
        None => Ok(()),
    }
}

pub async fn apply(env: &CodeEnv, container: &str, preview: &Preview) -> Result<()> {
    runtime::stop(env, container, &preview.id).await?;
    if preview.kind.needs_port() {
        runtime::address(env, container, preview).await?;
    }
    Ok(())
}

pub async fn keep(db: &Db, id: &str) -> Result<()> {
    db.execute(
        "UPDATE code_previews SET last_used_at = now() WHERE id = $id",
        json!({ "id": id }),
    )
    .await?;
    Ok(())
}

pub async fn touch(db: &Db, id: &str) -> Result<()> {
    db.execute(
        "UPDATE code_previews SET last_used_at = now()
         WHERE id = $id AND last_used_at < now() - interval '1 minute'",
        json!({ "id": id }),
    )
    .await?;
    Ok(())
}

pub async fn set_audience(db: &Db, id: &str, audience: Audience) -> Result<()> {
    db.execute(
        "UPDATE code_previews SET audience = $audience, updated_at = now() WHERE id = $id",
        json!({ "id": id, "audience": audience.as_str() }),
    )
    .await?;
    Ok(())
}

pub async fn delete(env: &CodeEnv, preview: &Preview) -> Result<()> {
    if let (Some(container), "active") = (&preview.session_container, preview.session_status.as_str()) {
        runtime::stop(env, container, &preview.id).await?;
    }
    clones::release_preview(env, &preview.id).await?;
    env.db
        .execute("DELETE FROM code_previews WHERE id = $id", json!({ "id": preview.id }))
        .await?;
    Ok(())
}

pub async fn reap_expired(env: &CodeEnv) -> Result<()> {
    let expired = env
        .db
        .query(
            "SELECT id FROM code_previews WHERE last_used_at < now() - make_interval(secs => $lifetime::float8)",
            json!({ "lifetime": env.settings.preview_lifetime.as_secs_f64() }),
        )
        .await?;
    for row in &expired {
        let Some(preview) = get(&env.db, row["id"].as_str().unwrap_or_default()).await? else {
            continue;
        };
        if let Err(e) = delete(env, &preview).await {
            tracing::warn!(preview = %preview.id, error = %e, "could not remove an expired preview");
        }
    }
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
            conversation_id: None,
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
