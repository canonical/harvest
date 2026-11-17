use std::time::Duration;

use anyhow::{bail, Context, Result};

use super::store::{self, NewJob};
use super::{Job, Kind, Status};
use crate::code::images::tail;
use crate::code::process::{self, Process, State};
use crate::code::{sessions, CodeEnv, WORKSPACE};
use crate::lxd::containers::Command;

const OUTPUT_CHARS: usize = 8_000;
const POLL: Duration = Duration::from_secs(3);
const LONGEST_RUN: Duration = Duration::from_secs(6 * 60 * 60);
const QUICK: Duration = Duration::from_secs(15);
const STALE_REVIEW_SECS: u64 = 30 * 60;

pub struct Spec<'a> {
    pub session_id: &'a str,
    pub chat_id: Option<&'a str>,
    pub kind: Kind,
    pub label: &'a str,
    pub script: &'a str,
}

fn process_of(job_id: &str) -> Process {
    Process::named(&format!("job-{job_id}"))
}

pub async fn start(env: &CodeEnv, spec: Spec<'_>) -> Result<Job> {
    let container = sessions::active_container(env, spec.session_id).await?;
    let job = store::create(
        env,
        NewJob {
            session_id: spec.session_id,
            chat_id: spec.chat_id,
            kind: spec.kind,
            label: spec.label,
        },
    )
    .await?;
    let launched = env
        .lxd
        .run(
            &container,
            Command::shell(&process_of(&job.id).start_script(spec.script, &[]))
                .in_dir(WORKSPACE)
                .timeout(QUICK),
        )
        .await;
    if let Err(e) = launched {
        store::finish(env, &job.id, Status::Failed, None, &format!("could not start: {e}")).await?;
        bail!("could not start the job: {e}");
    }
    spawn_monitor(env.clone(), job.id.clone());
    Ok(job)
}

fn spawn_monitor(env: CodeEnv, job_id: String) {
    tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + LONGEST_RUN;
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
            match refresh(&env, &job_id).await {
                Ok(job) if !job.is_running() => return,
                Ok(_) => {}
                Err(e) => tracing::warn!(job = job_id, error = %e, "could not check a background job"),
            }
        }
        if let Err(e) = stop(&env, &job_id).await {
            tracing::warn!(job = job_id, error = %e, "could not stop a job that ran too long");
        }
    });
}

pub async fn refresh(env: &CodeEnv, job_id: &str) -> Result<Job> {
    let job = store::get(env, job_id).await?.context("this job does not exist")?;
    if !job.is_running() || !job.kind.runs_in_container() {
        return Ok(job);
    }
    let Ok(container) = sessions::active_container(env, &job.session_id).await else {
        return Ok(finished(env, &job, Status::Cancelled, None, "the session ended").await?);
    };
    let output = env
        .lxd
        .run(&container, Command::shell(&process_of(job_id).status_script(OUTPUT_CHARS)).timeout(QUICK))
        .await?;
    let status = process::parse_status(&output.stdout);
    let log = tail(&status.log, OUTPUT_CHARS);
    match status.state {
        State::Running => {
            store::record_output(env, job_id, &log).await?;
            Ok(Job { output: log, ..job })
        }
        State::Exited(code) => finished(env, &job, Status::from_exit_code(code), Some(code), &log).await,
        State::Lost => {
            let note = format!("{log}\nThe process stopped without reporting an exit code.");
            finished(env, &job, Status::Failed, None, note.trim_start()).await
        }
    }
}

async fn finished(env: &CodeEnv, job: &Job, status: Status, exit_code: Option<i64>, output: &str) -> Result<Job> {
    Ok(store::finish(env, &job.id, status, exit_code, output)
        .await?
        .unwrap_or_else(|| job.clone()))
}

pub async fn wait(env: &CodeEnv, job_id: &str, timeout: Duration) -> Result<Job> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let job = refresh(env, job_id).await?;
        if !job.is_running() || tokio::time::Instant::now() >= deadline {
            return Ok(job);
        }
        tokio::time::sleep(POLL).await;
    }
}

pub async fn stop(env: &CodeEnv, job_id: &str) -> Result<Job> {
    let job = store::get(env, job_id).await?.context("this job does not exist")?;
    if !job.is_running() {
        return Ok(job);
    }
    if job.kind.runs_in_container() {
        if let Ok(container) = sessions::active_container(env, &job.session_id).await {
            env.lxd
                .run(&container, Command::shell(&process_of(job_id).stop_script()).timeout(QUICK))
                .await?;
        }
    }
    finished(env, &job, Status::Cancelled, None, &job.output).await
}

pub async fn sweep(env: &CodeEnv) -> Result<()> {
    store::expire_stale(env, Kind::Review, STALE_REVIEW_SECS).await?;
    for job in store::running(env).await?.into_iter().filter(|job| job.kind.runs_in_container()) {
        if let Err(e) = refresh(env, &job.id).await {
            tracing::warn!(job = job.id, error = %e, "could not refresh a background job");
        }
    }
    Ok(())
}
