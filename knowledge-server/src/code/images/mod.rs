mod builder;
mod claim;
mod failure;
mod setup;
mod state;

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};

use super::projects::CodeProject;
use super::CodeEnv;

pub use failure::BuildFailure;
pub use state::{need, Attempt, Image, ImageNeed, ReadyImage};

pub const REPO_SETUP_FILE: &str = ".harvest/setup.sh";
const RECIPE_VERSION: u32 = 4;
const BUILDER_PREFIX: &str = "hcb-";
const ORPHAN_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildOutcome {
    Built,
    AlreadyBuilding,
}

pub fn setup_hash(script: Option<&str>) -> String {
    let mut hasher = DefaultHasher::new();
    RECIPE_VERSION.hash(&mut hasher);
    script.unwrap_or_default().trim().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

pub fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max)).collect()
}

pub async fn current(env: &CodeEnv, project_id: &str) -> Result<Option<Image>> {
    let rows = env
        .db
        .query(
            "SELECT status, alias, base_commit, setup_hash, log, setup_warning,
                    attempt_status, attempt_failure, attempt_hash, attempt_log,
                    extract(epoch FROM now() - attempted_at)::float8 AS attempt_age
             FROM code_images WHERE project_id = $pid",
            json!({ "pid": project_id }),
        )
        .await?;
    Ok(rows.first().map(image_from_row))
}

fn image_from_row(row: &Value) -> Image {
    let text = |key: &str| row[key].as_str().map(String::from);
    let ready = (row["status"].as_str() == Some("ready")).then(|| ReadyImage {
        alias: text("alias").unwrap_or_default(),
        base_commit: text("base_commit"),
        setup_hash: text("setup_hash").unwrap_or_default(),
    });
    let age = Duration::from_secs_f64(row["attempt_age"].as_f64().unwrap_or_default().max(0.0));
    let attempt = match row["attempt_status"].as_str() {
        Some("building") => Some(Attempt::Building { age }),
        Some("failed") => Some(Attempt::Failed {
            failure: row["attempt_failure"]
                .as_str()
                .and_then(BuildFailure::parse)
                .unwrap_or(BuildFailure::Infrastructure),
            setup_hash: text("attempt_hash").unwrap_or_default(),
            age,
        }),
        _ => None,
    };
    Image {
        ready,
        attempt,
        log: text("log").unwrap_or_default(),
        attempt_log: text("attempt_log").unwrap_or_default(),
        setup_warning: text("setup_warning"),
    }
}

pub async fn build(env: &CodeEnv, project: &CodeProject) -> Result<BuildOutcome> {
    let hash = setup_hash(project.setup_script.as_deref());
    let Some(claim) = claim::try_claim(&env.db, &project.id, &hash).await? else {
        return Ok(BuildOutcome::AlreadyBuilding);
    };
    let built = builder::build_in(env, project, &claim).await;
    if let Err(error) = env.lxd.delete_instance(&claim.builder_name()).await {
        tracing::warn!(project = %project.name, error = format!("{error:#}"), "could not remove the image builder");
    }
    match built {
        Ok(built) => {
            let completion = claim::Completion {
                base_commit: &built.base_commit,
                log: &built.log,
                setup_warning: built.setup_warning.as_deref(),
            };
            if let Some(previous) = claim::complete(&env.db, &claim, completion).await? {
                remove_image(env, &previous).await;
            }
            Ok(BuildOutcome::Built)
        }
        Err(error) => {
            let failure = BuildFailure::of(&error);
            tracing::warn!(project = %project.name, failure = failure.as_str(), error = format!("{error:#}"), "code image build failed");
            claim::fail(&env.db, &claim, failure, &tail(&format!("{error:#}"), 8_000)).await?;
            Err(error)
        }
    }
}

async fn remove_image(env: &CodeEnv, alias: &str) {
    if let Err(error) = env.lxd.delete_image_alias(alias).await {
        tracing::warn!(alias, error = format!("{error:#}"), "could not remove a replaced code image");
    }
}

pub async fn sweep_builders(env: &CodeEnv) -> Result<()> {
    let attempts = claim::active_attempts(&env.db).await?;
    let instances = env.lxd.instance_names().await?;
    let existing: HashSet<String> = instances.iter().cloned().collect();
    for attempt in orphaned_attempts(&attempts, &existing) {
        tracing::warn!(project = attempt.project_id, "an image build stopped before it finished; it will be retried");
        claim::abandon(&env.db, attempt).await?;
    }
    let active: HashSet<String> = attempts.iter().map(claim::ActiveAttempt::builder_name).collect();
    for builder in abandoned_builders(&instances, &active) {
        if let Err(error) = env.lxd.delete_instance(&builder).await {
            tracing::warn!(builder, error = format!("{error:#}"), "could not remove an abandoned image builder");
        }
    }
    Ok(())
}

fn orphaned_attempts<'a>(attempts: &'a [claim::ActiveAttempt], instances: &HashSet<String>) -> Vec<&'a claim::ActiveAttempt> {
    attempts
        .iter()
        .filter(|attempt| attempt.age >= ORPHAN_AFTER && !instances.contains(&attempt.builder_name()))
        .collect()
}

fn abandoned_builders(instances: &[String], active: &HashSet<String>) -> Vec<String> {
    instances
        .iter()
        .filter(|name| name.starts_with(BUILDER_PREFIX) && !active.contains(*name))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_keeps_single_quotes_inside_a_shell_word() {
        assert_eq!(quote("feat/x"), "'feat/x'");
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn the_setup_hash_ignores_surrounding_whitespace_only() {
        assert_eq!(setup_hash(Some("make\n")), setup_hash(Some("make")));
        assert_ne!(setup_hash(Some("make")), setup_hash(Some("make test")));
        assert_eq!(setup_hash(None), setup_hash(Some("")));
    }

    #[test]
    fn only_builders_without_a_live_attempt_are_swept() {
        let instances: Vec<String> = ["hcb-aaaa-1111", "hcb-bbbb", "hcp-aaaa-xyz", "hcb-cccc-2222"].map(String::from).into();
        let active: HashSet<String> = ["hcb-cccc-2222".to_string()].into();
        assert_eq!(abandoned_builders(&instances, &active), ["hcb-aaaa-1111", "hcb-bbbb"]);
    }

    #[test]
    fn an_old_attempt_without_its_builder_is_orphaned() {
        let attempt = |attempt_id: &str, minutes: u64| claim::ActiveAttempt {
            project_id: "154f6dcf-b6e3".into(),
            attempt_id: attempt_id.into(),
            age: Duration::from_secs(minutes * 60),
        };
        let attempts = [attempt("aaaaaaaa", 30), attempt("bbbbbbbb", 30), attempt("cccccccc", 2)];
        let instances: HashSet<String> = ["hcb-154f6dcf-bbbbbbbb".to_string()].into();
        let orphaned: Vec<&str> = orphaned_attempts(&attempts, &instances).iter().map(|a| a.attempt_id.as_str()).collect();
        assert_eq!(orphaned, ["aaaaaaaa"]);
    }

    #[test]
    fn a_stored_row_becomes_a_typed_image() {
        let row = json!({
            "status": "ready", "alias": "harvest-code-a-b", "base_commit": "abc", "setup_hash": "h",
            "log": "ok", "setup_warning": null, "attempt_status": "failed",
            "attempt_failure": "lxd_unavailable", "attempt_hash": "h2", "attempt_log": "refused", "attempt_age": 30.0
        });
        let image = image_from_row(&row);
        assert_eq!(image.ready.as_ref().unwrap().alias, "harvest-code-a-b");
        assert_eq!(
            image.attempt,
            Some(Attempt::Failed { failure: BuildFailure::LxdUnavailable, setup_hash: "h2".into(), age: Duration::from_secs(30) })
        );
        let unbuilt = image_from_row(&json!({ "status": "none", "attempt_status": null }));
        assert_eq!(unbuilt.ready, None);
        assert_eq!(unbuilt.attempt, None);
    }
}
