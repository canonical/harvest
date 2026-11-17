use anyhow::{Context, Result};
use serde_json::json;
use uuid::Uuid;

use super::{Job, Kind, Status};
use crate::code::CodeEnv;

const COLUMNS: &str = "id, session_id, chat_id, kind, label, status, exit_code, output, created_at, finished_at";
const LISTED: i64 = 50;

pub struct NewJob<'a> {
    pub session_id: &'a str,
    pub chat_id: Option<&'a str>,
    pub kind: Kind,
    pub label: &'a str,
}

pub async fn create(env: &CodeEnv, job: NewJob<'_>) -> Result<Job> {
    let rows = env
        .db
        .query(
            &format!(
                "INSERT INTO code_session_jobs (id, session_id, chat_id, kind, label, status)
                 VALUES ($id, $sid, $chat, $kind, $label, 'running') RETURNING {COLUMNS}"
            ),
            json!({
                "id": Uuid::new_v4().simple().to_string(),
                "sid": job.session_id,
                "chat": job.chat_id,
                "kind": job.kind.as_str(),
                "label": job.label,
            }),
        )
        .await?;
    rows.first().and_then(Job::from_row).context("the job was not recorded")
}

pub async fn get(env: &CodeEnv, id: &str) -> Result<Option<Job>> {
    let rows = env
        .db
        .query(&format!("SELECT {COLUMNS} FROM code_session_jobs WHERE id = $id"), json!({ "id": id }))
        .await?;
    Ok(rows.first().and_then(Job::from_row))
}

pub async fn list(env: &CodeEnv, session_id: &str) -> Result<Vec<Job>> {
    let rows = env
        .db
        .query(
            &format!(
                "SELECT {COLUMNS} FROM code_session_jobs WHERE session_id = $sid
                 ORDER BY created_at DESC LIMIT $limit"
            ),
            json!({ "sid": session_id, "limit": LISTED }),
        )
        .await?;
    Ok(rows.iter().filter_map(Job::from_row).collect())
}

pub async fn running(env: &CodeEnv) -> Result<Vec<Job>> {
    let rows = env
        .db
        .query(&format!("SELECT {COLUMNS} FROM code_session_jobs WHERE status = 'running'"), json!({}))
        .await?;
    Ok(rows.iter().filter_map(Job::from_row).collect())
}

pub async fn latest(env: &CodeEnv, session_id: &str, kind: Kind) -> Result<Option<Job>> {
    let rows = env
        .db
        .query(
            &format!(
                "SELECT {COLUMNS} FROM code_session_jobs WHERE session_id = $sid AND kind = $kind
                 ORDER BY created_at DESC LIMIT 1"
            ),
            json!({ "sid": session_id, "kind": kind.as_str() }),
        )
        .await?;
    Ok(rows.first().and_then(Job::from_row))
}

pub async fn record_output(env: &CodeEnv, id: &str, output: &str) -> Result<()> {
    env.db
        .execute(
            "UPDATE code_session_jobs SET output = $output WHERE id = $id AND status = 'running'",
            json!({ "id": id, "output": output }),
        )
        .await?;
    Ok(())
}

pub async fn finish(env: &CodeEnv, id: &str, status: Status, exit_code: Option<i64>, output: &str) -> Result<Option<Job>> {
    let rows = env
        .db
        .query(
            &format!(
                "UPDATE code_session_jobs SET status = $status, exit_code = $code, output = $output, finished_at = now()
                 WHERE id = $id AND status = 'running' RETURNING {COLUMNS}"
            ),
            json!({ "id": id, "status": status.as_str(), "code": exit_code, "output": output }),
        )
        .await?;
    Ok(rows.first().and_then(Job::from_row))
}

pub async fn take_unreported(env: &CodeEnv, session_id: &str, chat_id: &str) -> Result<Vec<Job>> {
    let rows = env
        .db
        .query(
            &format!(
                "UPDATE code_session_jobs SET reported = true
                 WHERE session_id = $sid AND status <> 'running' AND NOT reported
                   AND kind <> 'review' AND (chat_id IS NULL OR chat_id = $chat)
                 RETURNING {COLUMNS}"
            ),
            json!({ "sid": session_id, "chat": chat_id }),
        )
        .await?;
    let mut jobs: Vec<Job> = rows.iter().filter_map(Job::from_row).collect();
    jobs.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(jobs)
}

pub async fn running_in_session(env: &CodeEnv, session_id: &str) -> Result<Vec<Job>> {
    let rows = env
        .db
        .query(
            &format!(
                "SELECT {COLUMNS} FROM code_session_jobs WHERE session_id = $sid AND status = 'running'
                 ORDER BY created_at"
            ),
            json!({ "sid": session_id }),
        )
        .await?;
    Ok(rows.iter().filter_map(Job::from_row).collect())
}

pub async fn expire_stale(env: &CodeEnv, kind: Kind, older_than_secs: u64) -> Result<u64> {
    env.db
        .execute(
            "UPDATE code_session_jobs SET status = 'failed', output = output || $note, finished_at = now()
             WHERE kind = $kind AND status = 'running'
               AND created_at < now() - make_interval(secs => $secs::float8)",
            json!({
                "kind": kind.as_str(),
                "secs": older_than_secs as f64,
                "note": "\nThe server restarted before this job finished.",
            }),
        )
        .await
}

pub async fn mark_reported(env: &CodeEnv, id: &str) -> Result<()> {
    env.db
        .execute("UPDATE code_session_jobs SET reported = true WHERE id = $id", json!({ "id": id }))
        .await?;
    Ok(())
}
