use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::projects::{self, CodeProject};
use super::sessions::{self, CloseReason, Session};
use super::{change_requests, chats, images, index, learned_setup, previews, CodeEnv};
use crate::access::repos::{check_remote, RepoAccess, RepoScope};
use crate::api::query::{stream_turn, QueryRequest};
use crate::api::repositories::{start_ingestion, IngestionSource};
use crate::api::{GraphState, QueryState};
use crate::auth::jwt::Claims;
use crate::credentials::remote::RemoteUrl;
use crate::modes::Space;

pub struct CodeState {
    pub env: Option<CodeEnv>,
    pub graph: Arc<GraphState>,
    pub query: Arc<QueryState>,
}

pub(crate) type Reply = Result<Response, Response>;

pub(crate) fn failure(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

pub(crate) fn internal(e: anyhow::Error) -> Response {
    tracing::error!(error = %e, "code request failed");
    failure(StatusCode::INTERNAL_SERVER_ERROR, "server error")
}

pub(crate) fn env(state: &CodeState) -> Result<&CodeEnv, Response> {
    state.env.as_ref().ok_or_else(|| {
        failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "Code mode needs LXD; ask an administrator to configure it",
        )
    })
}

pub(crate) async fn scope(env: &CodeEnv, user: &Claims) -> RepoScope {
    RepoAccess::new(&env.db)
        .scope_for(&user.sub)
        .await
        .unwrap_or_else(|_| RepoScope::only(Vec::<String>::new()))
}

pub(crate) async fn project_for(env: &CodeEnv, user: &Claims, id: &str) -> Result<CodeProject, Response> {
    let project = projects::get(&env.db, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "not found"))?;
    if !scope(env, user).await.allows(&project.name) {
        return Err(failure(StatusCode::NOT_FOUND, "not found"));
    }
    Ok(project)
}

pub(crate) async fn session_for(env: &CodeEnv, user: &Claims, id: &str) -> Result<Session, Response> {
    sessions::get(env, id)
        .await
        .map_err(internal)?
        .filter(|s| s.user_id == user.sub)
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "not found"))
}

pub async fn list_projects(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
) -> Reply {
    let env = env(&state)?;
    let scope = scope(env, &user).await;
    let rows = projects::list(&env.db, &scope, &user.sub).await.map_err(internal)?;
    Ok(Json(rows).into_response())
}

pub async fn list_sessions(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
) -> Reply {
    let env = env(&state)?;
    let scope = scope(env, &user).await;
    let rows = sessions::list_for_user(&env.db, &scope, &user.sub).await.map_err(internal)?;
    Ok(Json(rows).into_response())
}

#[derive(Deserialize)]
pub struct OpenBody {
    pub url: String,
}

pub async fn open_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Json(body): Json<OpenBody>,
) -> Reply {
    let env = env(&state)?;
    let url = body.url.trim().to_string();
    let remote = RemoteUrl::parse(&url)
        .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "enter the URL of a git repository"))?;
    let credentials = env.git_credentials(&user.sub, &url).await;
    let check = check_remote(&url, credentials.clone())
        .await
        .map_err(|reason| failure(StatusCode::FORBIDDEN, reason))?;
    let listing = list_remote(&url, credentials.clone()).await;
    let default_branch = listing
        .default_branch
        .or_else(|| listing.branches.first().cloned())
        .ok_or_else(|| failure(StatusCode::UNPROCESSABLE_ENTITY, "the repository has no branches"))?;
    let name = existing_name(env, &url).await.map_err(internal)?.unwrap_or_else(|| remote.name());
    RepoAccess::new(&env.db)
        .register(&name, &url, check.visibility, &user.sub)
        .await
        .map_err(internal)?;
    if !is_ingested(env, &name).await.map_err(internal)? {
        let source = IngestionSource {
            name: name.clone(),
            url: url.clone(),
            browse_url: None,
            refs: vec![default_branch.clone()],
            credentials,
            force: false,
        };
        let _ = start_ingestion(&state.graph, source).await;
    }
    let project = projects::ensure(&env.db, &name, Some(&default_branch), &user.sub)
        .await
        .map_err(internal)?;
    Ok((StatusCode::CREATED, Json(project)).into_response())
}

async fn list_remote(url: &str, credentials: knowledge_harvester::git::GitCredentials) -> knowledge_harvester::git::RemoteListing {
    let url = url.to_string();
    let scratch = std::env::temp_dir().join("harvest-remote-checks");
    tokio::task::spawn_blocking(move || {
        knowledge_harvester::git::GitClient::new(scratch)
            .with_credentials(credentials)
            .list_remote(&url)
    })
    .await
    .ok()
    .and_then(Result::ok)
    .unwrap_or_default()
}

async fn existing_name(env: &CodeEnv, url: &str) -> anyhow::Result<Option<String>> {
    let rows = env
        .db
        .query("SELECT name FROM repositories WHERE url = $url", json!({ "url": url }))
        .await?;
    Ok(rows.first().and_then(|r| r["name"].as_str()).map(String::from))
}

async fn is_ingested(env: &CodeEnv, name: &str) -> anyhow::Result<bool> {
    let rows = env
        .db
        .query(
            "SELECT 1 AS ok FROM versions v JOIN repositories r ON r.id = v.repository_id
             WHERE r.name = $name AND v.ingested LIMIT 1",
            json!({ "name": name }),
        )
        .await?;
    Ok(!rows.is_empty())
}

pub async fn get_project(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    let project = project_for(env, &user, &id).await?;
    let credentials = env.git_credentials(&user.sub, &project.url).await;
    let listing = list_remote(&project.url, credentials).await;
    let image = images::current(env, &id).await.map_err(internal)?;
    let sessions = sessions::list_for(env, &id, &user.sub).await.map_err(internal)?;
    let index = index::of_project(&env.db, &project).await.map_err(internal)?;
    Ok(Json(json!({
        "project": project,
        "branches": listing.branches,
        "image": image.map(|image| image.summary()),
        "sessions": sessions,
        "index": index,
        "learned_setup": learned_setup::list(env, &id).await.map_err(internal)?,
    }))
    .into_response())
}

#[derive(Deserialize)]
pub struct SetupBody {
    pub script: Option<String>,
}

pub async fn set_setup(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SetupBody>,
) -> Reply {
    let env = env(&state)?;
    project_for(env, &user, &id).await?;
    projects::set_setup_script(&env.db, &id, body.script.as_deref())
        .await
        .map_err(internal)?;
    projects::touch(&env.db, &id).await.map_err(internal)?;
    learned_setup::forget(env, &id).await.map_err(internal)?;
    Ok(Json(json!({ "ok": true })).into_response())
}

#[derive(Deserialize)]
pub struct SessionBody {
    pub branch: String,
    #[serde(default)]
    pub base_branch: Option<String>,
}

pub async fn open_session(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
    Json(body): Json<SessionBody>,
) -> Reply {
    let env = env(&state)?;
    let project = project_for(env, &user, &id).await?;
    let base = body
        .base_branch
        .or_else(|| project.default_branch.clone())
        .unwrap_or_else(|| "main".into());
    let session = sessions::open(env, &project, &user.sub, body.branch.trim(), base.trim())
        .await
        .map_err(|e| failure(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(session)).into_response())
}

pub async fn get_session(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    let session = session_for(env, &user, &id).await?;
    let project = projects::get(&env.db, &session.project_id).await.map_err(internal)?;
    let change_request = change_requests::for_session(env, &id).await.map_err(internal)?;
    let chats = chats::list(&env.db, &id).await.map_err(internal)?;
    let previews = previews::list(&env.db, &id, env.settings.preview_lifetime)
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "session": session,
        "project": project,
        "change_request": change_request,
        "chats": chats,
        "previews": previews,
        "web_previews": env.settings.preview_origin.is_some(),
    }))
    .into_response())
}

pub async fn close_session(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &id).await?;
    sessions::close(env, &id, CloseReason::Manual).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub struct ChangesQuery {
    pub commit: Option<String>,
}

pub async fn session_changes(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
    Query(query): Query<ChangesQuery>,
) -> Reply {
    let env = env(&state)?;
    let session = session_for(env, &user, &id).await?;
    let container = sessions::active_container(env, &id)
        .await
        .map_err(|e| failure(StatusCode::CONFLICT, e.to_string()))?;
    let base = images::quote(&format!("refs/remotes/origin/{}", session.base_branch));
    let script = match query.commit.as_deref() {
        Some(commit) => commit_diff_script(commit)
            .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "commit must be a commit hash"))?,
        None => format!(
            "index=$(mktemp) && cp .git/index \"$index\" && GIT_INDEX_FILE=\"$index\" git add -N . && \
             GIT_INDEX_FILE=\"$index\" git diff {base}; status=$?; rm -f \"$index\"; exit $status"
        ),
    };
    let diff = sessions::run_checked(env, &container, &script)
        .await
        .map_err(|e| failure(StatusCode::BAD_GATEWAY, e.to_string()))?;
    let commits = sessions::run_checked(env, &container, &format!("git log --oneline {base}..HEAD"))
        .await
        .unwrap_or_default();
    Ok(Json(json!({
        "diff": diff,
        "commits": commits.lines().collect::<Vec<_>>(),
        "commit": query.commit,
    }))
    .into_response())
}

pub fn commit_diff_script(commit: &str) -> Option<String> {
    let valid = (4..=40).contains(&commit.len()) && commit.chars().all(|c| c.is_ascii_hexdigit());
    valid.then(|| format!("git show --format= {commit}"))
}

#[derive(Deserialize)]
pub struct ProposalBody {
    pub title: String,
    #[serde(default)]
    pub description: String,
}

pub async fn propose_change(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
    Json(body): Json<ProposalBody>,
) -> Reply {
    let env = env(&state)?;
    session_for(env, &user, &id).await?;
    let request = change_requests::submit(env, &id, &body.title, &body.description)
        .await
        .map_err(|e| failure(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    Ok(Json(request).into_response())
}

pub async fn session_query_stream(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<QueryRequest>,
) -> Reply {
    let env = env(&state)?;
    let session = session_for(env, &user, &id).await?;
    if session.status != "active" {
        return Err(failure(StatusCode::CONFLICT, format!("the session is {}", session.status)));
    }
    let chat_id = request
        .conversation_id
        .clone()
        .ok_or_else(|| failure(StatusCode::BAD_REQUEST, "choose the chat to send this message to"))?;
    if !chats::belongs(&env.db, &id, &chat_id).await.map_err(internal)? {
        return Err(failure(StatusCode::NOT_FOUND, "this chat does not belong to the session"));
    }
    sessions::touch(env, &id).await.map_err(internal)?;
    let space = Space::code_session(&session.id, &user.sub);
    Ok(stream_turn(user, Arc::clone(&state.query), headers, request, space).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_commit_hashes_reach_the_shell() {
        assert_eq!(commit_diff_script("0be3747").as_deref(), Some("git show --format= 0be3747"));
        assert_eq!(commit_diff_script("abc"), None);
        assert_eq!(commit_diff_script("0be3747; rm -rf /"), None);
        assert_eq!(commit_diff_script("HEAD~1"), None);
    }
}
