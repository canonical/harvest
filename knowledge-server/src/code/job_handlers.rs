use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::handlers::{env, failure, internal, project_for, session_for, CodeState, Reply};
use super::jobs::{shell, store, Job};
use super::{projects, verification, CodeEnv};
use crate::auth::jwt::Claims;

async fn job_for(env: &CodeEnv, user: &Claims, id: &str) -> Result<Job, axum::response::Response> {
    let job = store::get(env, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "this job does not exist"))?;
    session_for(env, user, &job.session_id).await?;
    Ok(job)
}

pub async fn list(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let jobs = store::list(env, &session_id).await.map_err(internal)?;
    Ok(Json(json!({ "jobs": jobs })).into_response())
}

pub async fn get(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    job_for(env, &user, &id).await?;
    let job = shell::refresh(env, &id).await.map_err(internal)?;
    Ok(Json(json!({ "job": job })).into_response())
}

pub async fn stop(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    job_for(env, &user, &id).await?;
    let job = shell::stop(env, &id).await.map_err(internal)?;
    Ok(Json(json!({ "job": job })).into_response())
}

pub async fn run_checks(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(session_id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &session_id).await?;
    let job = verification::start(env, &session_id, None)
        .await
        .map_err(|e| failure(StatusCode::CONFLICT, e.to_string()))?;
    match job {
        Some(job) => Ok((StatusCode::CREATED, Json(json!({ "job": job }))).into_response()),
        None => Err(failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            "this repository has no checks yet; add them in the project settings",
        )),
    }
}

#[derive(Deserialize)]
pub struct ChecksBody {
    pub checks: Option<String>,
}

pub async fn set_checks(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(project_id): Path<String>,
    Json(body): Json<ChecksBody>,
) -> Reply {
    let env = env(&state)?;
    project_for(env, &user, &project_id).await?;
    projects::set_checks(&env.db, &project_id, body.checks.as_deref())
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "ok": true })).into_response())
}
