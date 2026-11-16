use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::json;

use super::super::sessions::container_name;
use super::super::CodeEnv;
use super::{Kind, Preview, Viewer};
use crate::lxd::containers::Source;

const CLONE_IDLE: Duration = Duration::from_secs(2 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Ready(String),
    Starting,
    Failed(String),
}

pub async fn target_for(env: &CodeEnv, preview: &Preview, viewer: &Viewer) -> Result<Target> {
    if preview.session_status != "active" {
        bail!("this preview ended with its session");
    }
    if viewer.is_user(&preview.owner_id) {
        return match &preview.session_container {
            Some(container) => Ok(Target::Ready(container.clone())),
            None => Ok(Target::Starting),
        };
    }
    let key = viewer.key();
    let rows = env
        .db
        .query(
            "UPDATE code_preview_viewers v SET last_seen_at = now()
             FROM code_containers c
             WHERE v.preview_id = $pid AND v.viewer = $viewer AND c.name = v.container
             RETURNING v.container, v.error, c.state",
            json!({ "pid": preview.id, "viewer": key }),
        )
        .await?;
    if let Some(row) = rows.first() {
        let container = row["container"].as_str().unwrap_or_default().to_string();
        return Ok(match row["state"].as_str() {
            Some("ready") => Target::Ready(container),
            Some("failed") => Target::Failed(row["error"].as_str().unwrap_or("the copy failed").to_string()),
            _ => Target::Starting,
        });
    }
    create(env, preview, &key).await?;
    Ok(Target::Starting)
}

async fn create(env: &CodeEnv, preview: &Preview, key: &str) -> Result<()> {
    let Some(source) = preview.session_container.clone() else {
        bail!("the session has no container yet");
    };
    let count = env
        .db
        .query("SELECT count(*) AS n FROM code_containers WHERE role = 'preview'", json!({}))
        .await?;
    if count.first().and_then(|r| r["n"].as_i64()).unwrap_or(0) as usize >= env.settings.max_preview_copies {
        bail!("too many people are viewing previews right now; try again later");
    }
    let name = container_name("hcv", &preview.session_id);
    env.db
        .execute(
            "WITH c AS (
                 INSERT INTO code_containers (name, project_id, role, state)
                 SELECT $name, s.project_id, 'preview', 'building' FROM code_sessions s WHERE s.id = $sid
                 RETURNING name
             )
             INSERT INTO code_preview_viewers (preview_id, viewer, container)
             SELECT $pid, $viewer, c.name FROM c
             ON CONFLICT (preview_id, viewer) DO NOTHING",
            json!({ "name": name, "sid": preview.session_id, "pid": preview.id, "viewer": key }),
        )
        .await?;
    let env = env.clone();
    let preview = preview.clone();
    tokio::spawn(async move {
        let outcome = prepare(&env, &preview, &source, &name).await;
        let (state, error) = match outcome {
            Ok(()) => ("ready", None),
            Err(e) => ("failed", Some(e.to_string())),
        };
        let _ = env
            .db
            .execute(
                "WITH c AS (UPDATE code_containers SET state = $state WHERE name = $name RETURNING name)
                 UPDATE code_preview_viewers SET error = $error WHERE container IN (SELECT name FROM c)",
                json!({ "name": name, "state": state, "error": error }),
            )
            .await;
    });
    Ok(())
}

async fn prepare(env: &CodeEnv, preview: &Preview, source: &str, name: &str) -> Result<()> {
    env.lxd
        .launch(name, Source::Copy(source), &[("preview", &preview.id)])
        .await?;
    if preview.kind != Kind::Cli {
        super::runtime::address(env, name, preview).await?;
    }
    Ok(())
}

pub async fn reset(env: &CodeEnv, preview: &Preview, viewer: &Viewer) -> Result<()> {
    let rows = env
        .db
        .query(
            "SELECT container FROM code_preview_viewers WHERE preview_id = $pid AND viewer = $viewer",
            json!({ "pid": preview.id, "viewer": viewer.key() }),
        )
        .await?;
    for row in &rows {
        remove(env, row["container"].as_str().unwrap_or_default()).await?;
    }
    Ok(())
}

pub async fn release_preview(env: &CodeEnv, preview_id: &str) -> Result<()> {
    let rows = env
        .db
        .query(
            "SELECT container FROM code_preview_viewers WHERE preview_id = $pid",
            json!({ "pid": preview_id }),
        )
        .await?;
    for row in &rows {
        remove(env, row["container"].as_str().unwrap_or_default()).await?;
    }
    Ok(())
}

pub async fn release_session(env: &CodeEnv, session_id: &str) -> Result<()> {
    let rows = env
        .db
        .query(
            "SELECT v.container FROM code_preview_viewers v
             JOIN code_previews p ON p.id = v.preview_id WHERE p.session_id = $sid",
            json!({ "sid": session_id }),
        )
        .await?;
    for row in &rows {
        remove(env, row["container"].as_str().unwrap_or_default()).await?;
    }
    Ok(())
}

pub async fn reap(env: &CodeEnv) -> Result<()> {
    let rows = env
        .db
        .query(
            "SELECT container FROM code_preview_viewers
             WHERE last_seen_at < now() - make_interval(secs => $secs::float8)",
            json!({ "secs": CLONE_IDLE.as_secs_f64() }),
        )
        .await?;
    for row in &rows {
        remove(env, row["container"].as_str().unwrap_or_default()).await?;
    }
    Ok(())
}

async fn remove(env: &CodeEnv, container: &str) -> Result<()> {
    let _ = env.lxd.delete_instance(container).await;
    env.db
        .execute("DELETE FROM code_containers WHERE name = $name", json!({ "name": container }))
        .await?;
    Ok(())
}
