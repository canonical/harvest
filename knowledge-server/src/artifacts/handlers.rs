use axum::{
    body::Body,
    extract::{Extension, Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::jwt::Claims;
use crate::neo4j::Neo4jClient;

type ApiError = (StatusCode, Json<Value>);

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(json!({ "error": msg })))
}

#[derive(Clone)]
pub struct ArtifactState {
    pub neo4j: Arc<Neo4jClient>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    Markdown,
    Pdf,
}

impl ArtifactKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "markdown" => Some(Self::Markdown),
            "pdf"      => Some(Self::Pdf),
            _          => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Pdf      => "pdf",
        }
    }
}

fn sanitize_filename(title: &str) -> String {
    let words: Vec<&str> = title
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .filter(|s| !s.is_empty())
        .collect();
    let mut slug = words.join("-");
    slug.truncate(80);
    if slug.is_empty() { "artifact".to_string() } else { slug }
}

pub async fn require_artifact_access(
    neo4j: &Neo4jClient,
    user_id: &str,
    user_role: &str,
    artifact_id: &str,
) -> Result<Value, ApiError> {
    let rows = neo4j.query_read(
        "MATCH (g:Group)-[:HAS_PROJECT]->(p:Project)-[:HAS_ARTIFACT]->(a:Artifact {id: $aid})
         WHERE $role = 'admin'
            OR EXISTS { MATCH (:User {id: $uid})-[:MEMBER_OF]->(g) }
         RETURN a.id AS id, a.title AS title, a.kind AS kind, a.content AS content,
                a.created_by AS created_by, a.created_at AS created_at, a.updated_at AS updated_at,
                p.id AS project_id",
        json!({ "aid": artifact_id, "uid": user_id, "role": user_role }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;

    rows.into_iter().next()
        .ok_or_else(|| err(StatusCode::NOT_FOUND, "not found"))
}

pub async fn create_artifact(
    neo4j: &Neo4jClient,
    project_id: &str,
    kind: ArtifactKind,
    title: &str,
    content: &str,
    created_by: &str,
) -> anyhow::Result<Value> {
    let title = title.trim();
    anyhow::ensure!(!title.is_empty(), "title is required");
    let id  = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    neo4j.query_read(
        "MATCH (p:Project {id: $pid})
         CREATE (a:Artifact {
             id: $id, title: $title, kind: $kind, content: $content,
             created_by: $created_by, created_at: $now, updated_at: $now
         })
         CREATE (p)-[:HAS_ARTIFACT]->(a)
         RETURN a.id AS id",
        json!({
            "pid": project_id, "id": id, "title": title, "kind": kind.as_str(),
            "content": content, "created_by": created_by, "now": now,
        }),
    ).await?;
    Ok(json!({ "id": id, "title": title, "kind": kind.as_str(), "created_at": now }))
}

pub async fn get_artifact(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ArtifactState>>,
    Path(artifact_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let row = require_artifact_access(&state.neo4j, &user.sub, &user.role, &artifact_id).await?;
    Ok(Json(row))
}

pub async fn delete_artifact(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ArtifactState>>,
    Path(artifact_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    require_artifact_access(&state.neo4j, &user.sub, &user.role, &artifact_id).await?;
    state.neo4j.query_read(
        "MATCH (:Project)-[:HAS_ARTIFACT]->(a:Artifact {id: $aid}) DETACH DELETE a",
        json!({ "aid": artifact_id }),
    ).await.map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "server error"))?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn download_artifact(
    Extension(user): Extension<Claims>,
    State(state): State<Arc<ArtifactState>>,
    Path(artifact_id): Path<String>,
) -> Response {
    let row = match require_artifact_access(&state.neo4j, &user.sub, &user.role, &artifact_id).await {
        Ok(row) => row,
        Err(e) => return e.into_response(),
    };
    let title   = row["title"].as_str().unwrap_or("artifact");
    let kind    = row["kind"].as_str().unwrap_or("markdown");
    let content = row["content"].as_str().unwrap_or("").to_string();
    let slug    = sanitize_filename(title);

    if kind == "pdf" {
        match markdown2pdf::parse_into_bytes(content, markdown2pdf::config::ConfigSource::Default, None) {
            Ok(bytes) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, HeaderValue::from_static("application/pdf"))
                .header(
                    header::CONTENT_DISPOSITION,
                    HeaderValue::from_str(&format!("attachment; filename=\"{slug}.pdf\"")).unwrap(),
                )
                .body(Body::from(bytes))
                .unwrap(),
            Err(e) => {
                tracing::error!(error = %e, "failed to render artifact pdf");
                err(StatusCode::INTERNAL_SERVER_ERROR, "failed to render pdf").into_response()
            }
        }
    } else {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, HeaderValue::from_static("text/markdown; charset=utf-8"))
            .header(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{slug}.md\"")).unwrap(),
            )
            .body(Body::from(content))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_unsafe_characters() {
        let slug = sanitize_filename("My \"Report\"\r\nInjected: value");
        assert!(!slug.contains('"'));
        assert!(!slug.contains('\r'));
        assert!(!slug.contains('\n'));
        assert!(!slug.contains(':'));
        assert!(!slug.is_empty());
    }

    #[test]
    fn sanitize_filename_falls_back_when_no_safe_characters_remain() {
        assert_eq!(sanitize_filename("!!!"), "artifact");
    }

    #[test]
    fn sanitize_filename_joins_words_with_dashes() {
        assert_eq!(sanitize_filename("My Deploy Report"), "My-Deploy-Report");
    }

    #[test]
    fn artifact_kind_parses_known_values_only() {
        assert_eq!(ArtifactKind::parse("markdown"), Some(ArtifactKind::Markdown));
        assert_eq!(ArtifactKind::parse("pdf"), Some(ArtifactKind::Pdf));
        assert_eq!(ArtifactKind::parse("docx"), None);
    }

    #[test]
    fn markdown2pdf_smoke_test_produces_pdf_bytes() {
        let bytes = markdown2pdf::parse_into_bytes(
            "# Hello\nWorld".to_string(),
            markdown2pdf::config::ConfigSource::Default,
            None,
        ).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
    }
}
