use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

use crate::neo4j::Neo4jClient;

#[derive(Serialize)]
pub struct RepositoryInfo {
    pub name: String,
    pub url: Option<String>,
    pub versions: Vec<String>,
}

pub async fn handle_list_repositories(
    State(neo4j): State<Arc<Neo4jClient>>,
) -> impl IntoResponse {
    let result = neo4j
        .query_read(
            "MATCH (r:Repository)-[:HAS_VERSION]->(v:Version {ingested: true})
             RETURN r.name AS name, r.url AS url, collect(v.tag) AS versions
             ORDER BY r.name",
            json!({}),
        )
        .await;

    match result {
        Ok(rows) => {
            let repos: Vec<RepositoryInfo> = rows
                .into_iter()
                .map(|row| RepositoryInfo {
                    name: row["name"].as_str().unwrap_or("").to_string(),
                    url: row["url"].as_str().map(String::from),
                    versions: row["versions"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                })
                .collect();
            Json(repos).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list repositories");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}
