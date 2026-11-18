use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use super::images::{self, ImageNeed, ReadyImage};
use super::index;
use super::projects;
use super::sessions::{self, container_name, CloseReason};
use super::CodeEnv;
use crate::api::GraphState;
use crate::cluster::lease::{self, Leases};
use crate::lxd::containers::Source;

const EVERY: Duration = Duration::from_secs(30);
const LEASE_KEY: &str = "code-keeper";

pub async fn tick(env: &CodeEnv, graph: &GraphState) -> Result<()> {
    if !env.lxd.is_reachable().await {
        tracing::debug!("LXD is not reachable yet; the code keeper waits");
        return Ok(());
    }
    images::sweep_builders(env).await?;
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
            tracing::warn!(project = id, error = %format!("{e:#}"), "could not keep a code project warm");
        }
    }
    cool_down(env, warm_for).await?;
    index::refresh_due(env, graph).await?;
    super::previews::clones::reap(env).await?;
    super::previews::reap_expired(env).await?;
    super::jobs::shell::sweep(env).await?;
    super::change_requests::refresh(env).await?;
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
    if images::need(image.as_ref(), &hash) == ImageNeed::Build {
        images::build(env, &project).await?;
    }
    let Some(image) = images::current(env, project_id).await?.and_then(|image| image.ready) else {
        return Ok(());
    };
    retire_outdated_pool(env, project_id, &image).await?;
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

async fn add_to_pool(env: &CodeEnv, project_id: &str, image: &ReadyImage) -> Result<()> {
    let name = container_name("hcp", project_id);
    env.db
        .execute(
            "INSERT INTO code_containers (name, project_id, role, state, base_commit, image_alias)
             VALUES ($name, $pid, 'pool', 'building', $base, $alias)",
            json!({ "name": name, "pid": project_id, "base": image.base_commit, "alias": image.alias }),
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

async fn retire_outdated_pool(env: &CodeEnv, project_id: &str, image: &ReadyImage) -> Result<()> {
    let outdated = env
        .db
        .query(
            "DELETE FROM code_containers
             WHERE project_id = $pid AND role = 'pool' AND state = 'ready' AND image_alias IS DISTINCT FROM $alias
             RETURNING name",
            json!({ "pid": project_id, "alias": image.alias }),
        )
        .await?;
    for row in &outdated {
        let name = row["name"].as_str().unwrap_or_default();
        if let Err(error) = env.lxd.delete_instance(name).await {
            tracing::warn!(container = name, error = %format!("{error:#}"), "could not remove an outdated pool container");
        }
    }
    Ok(())
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

pub fn spawn(leases: Arc<dyn Leases>, env: CodeEnv, graph: Arc<GraphState>) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(EVERY);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let result = lease::run_exclusive(leases.as_ref(), LEASE_KEY, || async {
                if let Err(e) = tick(&env, &graph).await {
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
