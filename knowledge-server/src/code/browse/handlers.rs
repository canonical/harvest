use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::json;

use super::catalog::{self, Repository};
use super::model::{self, FileContent, FileView, RepoPath, MAX_RAW_BYTES};
use super::source::RepositorySource;
use crate::auth::jwt::Claims;
use crate::code::handlers::{env, failure, internal, scope, CodeState, Reply};
use crate::code::CodeEnv;

const RAW_SECURITY_POLICY: &str = "sandbox; default-src 'none'; img-src 'self' data:; style-src 'unsafe-inline'";

#[derive(Deserialize)]
pub struct Location {
    version: Option<String>,
    #[serde(default)]
    path: String,
}

pub async fn versions(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(name): Path<String>,
) -> Reply {
    let env = env(&state)?;
    let repository = repository_for(env, &user, &name).await?;
    let versions = catalog::versions(&env.db, &repository).await.map_err(internal)?;
    let default = catalog::default_version(&repository, &versions).map(|version| version.name.clone());
    Ok(Json(json!({ "versions": versions, "default": default })).into_response())
}

pub async fn tree(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(name): Path<String>,
    Query(location): Query<Location>,
) -> Reply {
    let browser = Browser::open(&state, &user, &name, &location).await?;
    let entries = browser
        .source
        .entries(&browser.commit, &browser.path)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    Ok(Json(entries).into_response())
}

pub async fn file(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(name): Path<String>,
    Query(location): Query<Location>,
) -> Reply {
    let browser = Browser::open(&state, &user, &name, &location).await?;
    let size = browser.file_size().await?;
    let content = if model::is_too_large_to_show(size) {
        FileContent::TooLarge
    } else {
        model::classify(&browser.file_bytes().await?)
    };
    let view = FileView {
        path: browser.path.as_str().to_string(),
        size,
        content,
    };
    Ok(Json(view).into_response())
}

pub async fn raw(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<CodeState>>,
    Path(name): Path<String>,
    Query(location): Query<Location>,
) -> Reply {
    let browser = Browser::open(&state, &user, &name, &location).await?;
    if browser.file_size().await? > MAX_RAW_BYTES {
        return Err(failure(StatusCode::PAYLOAD_TOO_LARGE, "the file is too large to download here"));
    }
    let bytes = browser.file_bytes().await?;
    let path = browser.path.as_str();
    let (content_type, disposition) = match model::image_type(path) {
        Some(image) => (image, "inline"),
        None => ("application/octet-stream", "attachment"),
    };
    let headers = [
        (header::CONTENT_TYPE, content_type.to_string()),
        (header::CONTENT_DISPOSITION, format!("{disposition}; filename=\"{}\"", download_name(path))),
        (header::CONTENT_SECURITY_POLICY, RAW_SECURITY_POLICY.to_string()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        (header::CACHE_CONTROL, "private, max-age=300".to_string()),
    ];
    Ok((headers, bytes).into_response())
}

struct Browser {
    source: Box<dyn RepositorySource>,
    commit: String,
    path: RepoPath,
}

impl Browser {
    async fn open(state: &CodeState, user: &Claims, name: &str, location: &Location) -> Result<Self, Response> {
        let env = env(state)?;
        let path = RepoPath::parse(&location.path).ok_or_else(|| failure(StatusCode::BAD_REQUEST, "invalid path"))?;
        let repository = repository_for(env, user, name).await?;
        let versions = catalog::versions(&env.db, &repository).await.map_err(internal)?;
        let version = match &location.version {
            Some(wanted) => versions.iter().find(|version| &version.name == wanted),
            None => catalog::default_version(&repository, &versions),
        }
        .ok_or_else(|| failure(StatusCode::NOT_FOUND, "this version has not been ingested"))?;
        let credentials = env.git_credentials(&user.sub, &repository.url).await;
        let mirror = env
            .mirrors
            .open(&repository, &version.commit, credentials)
            .await
            .map_err(unreachable_repository)?;
        Ok(Self {
            source: Box::new(mirror),
            commit: version.commit.clone(),
            path,
        })
    }

    async fn file_size(&self) -> Result<u64, Response> {
        self.source
            .file_size(&self.commit, &self.path)
            .await
            .map_err(internal)?
            .ok_or_else(not_found)
    }

    async fn file_bytes(&self) -> Result<Vec<u8>, Response> {
        self.source.file_bytes(&self.commit, &self.path).await.map_err(internal)
    }
}

async fn repository_for(env: &CodeEnv, user: &Claims, name: &str) -> Result<Repository, Response> {
    if !scope(env, user).await.allows(name) {
        return Err(not_found());
    }
    catalog::repository(&env.db, name)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)
}

fn not_found() -> Response {
    failure(StatusCode::NOT_FOUND, "not found")
}

fn unreachable_repository(error: anyhow::Error) -> Response {
    tracing::warn!(error = %error, "could not prepare the repository mirror");
    failure(
        StatusCode::BAD_GATEWAY,
        "could not fetch the repository; check your Git access settings",
    )
}

fn download_name(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_names_keep_only_safe_characters_of_the_file_name() {
        assert_eq!(download_name("docs/logo.png"), "logo.png");
        assert_eq!(download_name("a/evil\"; x.html"), "evil___x.html");
        assert_eq!(download_name("résumé.pdf"), "r_sum_.pdf");
    }
}
