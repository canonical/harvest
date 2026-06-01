use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, path::PathBuf, sync::Arc};

pub const SECTIONS: &[&str] = &["tutorials", "how-to-guides", "explanations", "reference"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DocEntry {
    pub filename: String,
    pub title: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DocIndex {
    pub repo: String,
    pub version: String,
    pub sections: HashMap<String, Vec<DocEntry>>,
}

pub async fn handle_get_index(
    Path((repo, version)): Path<(String, String)>,
    State(docs_dir): State<Arc<PathBuf>>,
) -> impl IntoResponse {
    let index_path = docs_dir.join(&repo).join(&version).join("index.json");
    match tokio::fs::read_to_string(&index_path).await {
        Ok(text) => match serde_json::from_str::<DocIndex>(&text) {
            Ok(index) => axum::Json(index).into_response(),
            Err(e) => {
                tracing::error!(error = %e, path = %index_path.display(), "malformed index.json");
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, axum::Json(json!({"error": "documentation not found"}))).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to read index.json");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn handle_get_page(
    Path((repo, version, section, filename)): Path<(String, String, String, String)>,
    State(docs_dir): State<Arc<PathBuf>>,
) -> impl IntoResponse {
    if !SECTIONS.contains(&section.as_str()) {
        return (StatusCode::NOT_FOUND, axum::Json(json!({"error": "unknown section"}))).into_response();
    }
    if filename.contains("..") || filename.contains('/') {
        return (StatusCode::BAD_REQUEST, axum::Json(json!({"error": "invalid filename"}))).into_response();
    }
    let file_path = docs_dir.join(&repo).join(&version).join(&section).join(&filename);
    match tokio::fs::read(&file_path).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, HeaderValue::from_static("text/markdown; charset=utf-8"))
            .body(Body::from(bytes))
            .unwrap(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, axum::Json(json!({"error": "page not found"}))).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to read doc page");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
