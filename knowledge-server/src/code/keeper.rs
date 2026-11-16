use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use super::images;
use super::projects;
use super::sessions::{self, container_name, CloseReason};
use super::CodeEnv;
use crate::cluster::lease::{self, Leases};
use crate::lxd::containers::Source;

const EVERY: Duration = Duration::from_secs(30);
const LEASE_KEY: &str = "code-keeper";

#[derive(Debug, PartialEq, Eq)]
pub enum ImageNeed {
    Keep,
    Build,
}

pub fn image_need(image: Option<&images::Image>, setting_hash: &str) -> ImageNeed {
    match image {
        None => ImageNeed::Build,
        Some(image) if image.is_building() => ImageNeed::Keep,
        Some(image) if image.stale => ImageNeed::Build,
        Some(image) if image.setup_hash != setting_hash => ImageNeed::Build,
        Some(_) => ImageNeed::Keep,
    }
}

pub async fn tick(env: &CodeEnv) -> Result<()> {
    let warm_for = env.settings.warm_for.as_secs_f64();
    let warm = env
        .db
        .query(
            "SELECT id FROM code_projects WHERE last_used_at > now() - make_interval(secs => $secs::float8)",
            json!({ "secs": warm_for }),
        )
        .await?;
    for row in &warm {
        let id = row["id"].as_str().unwrap_or_default();
        if let Err(e) = keep_warm(env, id).await {
            tracing::warn!(project = id, error = %e, "could not keep a code project warm");
        }
    }
    cool_down(env, warm_for).await?;
    for id in sessions::idle_sessions(env).await? {
        sessions::close(env, &id, CloseReason::Idle).await?;
    }
    Ok(())
}

async fn keep_warm(env: &CodeEnv, project_id: &str) -> Result<()> {
    let Some(project) = projects::get(&env.db, project_id).await? else {
        return Ok(());
    };
    let image = images::current(env, project_id).await?;
    let hash = images::setup_hash(project.setup_script.as_deref());
    if image_need(image.as_ref(), &hash) == ImageNeed::Build {
        images::build(env, &project).await?;
    }
    let Some(image) = images::current(env, project_id).await?.filter(|i| i.status == "ready") else {
        return Ok(());
    };
    let pooled = env
        .db
        .query(
            "SELECT count(*) AS n FROM code_containers
             WHERE project_id = $pid AND role = 'pool' AND state IN ('building', 'ready')",
            json!({ "pid": project_id }),
        )
        .await?;
    let have = pooled.first().and_then(|r| r["n"].as_i64()).unwrap_or(0) as usize;
    for _ in have..env.settings.pool_size {
        add_to_pool(env, project_id, &image).await?;
    }
    Ok(())
}

async fn add_to_pool(env: &CodeEnv, project_id: &str, image: &images::Image) -> Result<()> {
    let name = container_name("hcp", project_id);
    env.db
        .execute(
            "INSERT INTO code_containers (name, project_id, role, state, base_commit)
             VALUES ($name, $pid, 'pool', 'building', $base)",
            json!({ "name": name, "pid": project_id, "base": image.base_commit }),
        )
        .await?;
    let launched = env
        .lxd
        .launch(&name, Source::Image(&image.alias), &[("project", project_id)])
        .await;
    match launched {
        Ok(()) => {
            env.db
                .execute(
                    "UPDATE code_containers SET state = 'ready' WHERE name = $name AND state = 'building'",
                    json!({ "name": name }),
                )
                .await?;
            Ok(())
        }
        Err(e) => {
            let _ = env.lxd.delete_instance(&name).await;
            env.db
                .execute("DELETE FROM code_containers WHERE name = $name", json!({ "name": name }))
                .await?;
            Err(e)
        }
    }
}

async fn cool_down(env: &CodeEnv, warm_for: f64) -> Result<()> {
    let cold = env
        .db
        .query(
            "SELECT c.name FROM code_containers c JOIN code_projects p ON p.id = c.project_id
             WHERE c.role = 'pool'
               AND p.last_used_at <= now() - make_interval(secs => $secs::float8)",
            json!({ "secs": warm_for }),
        )
        .await?;
    for row in &cold {
        let name = row["name"].as_str().unwrap_or_default();
        let _ = env.lxd.delete_instance(name).await;
        env.db
            .execute("DELETE FROM code_containers WHERE name = $name", json!({ "name": name }))
            .await?;
    }
    Ok(())
}

pub fn spawn(leases: Arc<dyn Leases>, env: CodeEnv) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let result = lease::run_exclusive(leases.as_ref(), LEASE_KEY, || async {
                if let Err(e) = tick(&env).await {
                    tracing::warn!(error = %e, "code pool keeper failed");
                }
            })
            .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "code pool keeper could not run");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(status: &str, hash: &str) -> images::Image {
        images::Image {
            alias: "a".into(),
            status: status.into(),
            base_commit: None,
            setup_hash: hash.into(),
            stale: false,
        }
    }

    #[test]
    fn an_image_is_built_when_missing_or_when_its_setup_changed() {
        assert_eq!(image_need(None, "h"), ImageNeed::Build);
        assert_eq!(image_need(Some(&image("ready", "h")), "h"), ImageNeed::Keep);
        assert_eq!(image_need(Some(&image("ready", "old")), "h"), ImageNeed::Build);
        assert_eq!(image_need(Some(&image("building", "old")), "h"), ImageNeed::Keep);
    }

    #[test]
    fn a_build_left_behind_by_a_restart_is_built_again() {
        let stale = images::Image { stale: true, ..image("building", "h") };
        assert_eq!(image_need(Some(&stale), "h"), ImageNeed::Build);
    }

    #[test]
    fn a_failed_image_waits_for_a_new_setup_script_before_retrying() {
        assert_eq!(image_need(Some(&image("failed", "h")), "h"), ImageNeed::Keep);
        assert_eq!(image_need(Some(&image("failed", "h")), "fixed"), ImageNeed::Build);
    }
}
