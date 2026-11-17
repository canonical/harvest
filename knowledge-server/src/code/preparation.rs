use anyhow::Result;

use super::environment;
use super::jobs::shell::{self, Spec};
use super::jobs::{Job, Kind};
use super::project_facts::{self, Probe, ProjectFacts};
use super::{sessions, CodeEnv};

pub const LABEL: &str = "Prepare the environment";

pub async fn probe(env: &CodeEnv, container: &str) -> Result<(Probe, ProjectFacts)> {
    let output = sessions::run_checked(env, container, &project_facts::probe_script()).await?;
    let probe = project_facts::parse_probe(&output);
    let facts = project_facts::detect(&probe);
    Ok((probe, facts))
}

pub async fn script_for(env: &CodeEnv, container: &str) -> Result<Option<String>> {
    let (probe, facts) = probe(env, container).await?;
    Ok(environment::prepare_script(&probe, &facts))
}

pub async fn start_for_session(env: &CodeEnv, session_id: &str) -> Result<Option<Job>> {
    let container = sessions::active_container(env, session_id).await?;
    let Some(script) = script_for(env, &container).await? else {
        return Ok(None);
    };
    let job = shell::start(
        env,
        Spec {
            session_id,
            chat_id: None,
            kind: Kind::Prepare,
            label: LABEL,
            script: &script,
        },
    )
    .await?;
    Ok(Some(job))
}

pub fn spawn_for_session(env: &CodeEnv, session_id: &str) {
    let env = env.clone();
    let session_id = session_id.to_string();
    tokio::spawn(async move {
        if let Err(e) = start_for_session(&env, &session_id).await {
            tracing::warn!(session = session_id, error = %e, "could not prepare the session's environment");
        }
    });
}
