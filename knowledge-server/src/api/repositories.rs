use axum::{
    extract::{Path, Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{sse::Event, IntoResponse, Sse},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt;

use crate::api::GraphState;
use crate::ingestion::IngestionStatus;

#[derive(Serialize)]
pub struct RepositoryInfo {
    pub name: String,
    pub url: Option<String>,
    pub versions: Vec<String>,
    pub ingestion_status: Option<String>,
    pub ingestion_error: Option<String>,
}

#[derive(Deserialize)]
pub struct AddRepositoryBody {
    pub name: String,
    pub url: String,
    pub browse_url: Option<String>,
    pub refs: Vec<String>,
}

#[derive(Deserialize)]
pub struct VerifyRepositoryBody {
    pub url: String,
}

#[derive(Serialize)]
pub struct RefInfo {
    pub name: String,
    pub commit_sha: String,
}

#[derive(Serialize)]
pub struct VerifyResult {
    pub url: String,
    pub refs: Vec<RefInfo>,
}

#[derive(Deserialize, Default)]
pub struct StatsQuery {
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct LanguageStat {
    pub language: String,
    pub files: i64,
    pub percentage: f64,
}

#[derive(Serialize)]
pub struct SymbolStat {
    #[serde(rename = "type")]
    pub node_type: String,
    pub kind: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct RelationStat {
    pub relation: String,
    pub count: i64,
}

#[derive(Serialize)]
pub struct RepositoryTotals {
    pub files: i64,
    pub symbols: i64,
}

#[derive(Serialize)]
pub struct RepositoryStats {
    pub repository: String,
    pub url: Option<String>,
    pub version: String,
    pub versions: Vec<String>,
    pub languages: Vec<LanguageStat>,
    pub symbols: Vec<SymbolStat>,
    pub relations: Vec<RelationStat>,
    pub totals: RepositoryTotals,
}

pub async fn handle_list_repositories(
    State(state): State<Arc<GraphState>>,
) -> impl IntoResponse {
    let result = state.db
        .query(
            "SELECT r.name, r.url, array_agg(v.tag ORDER BY v.timestamp) AS versions
             FROM repositories r JOIN versions v ON v.repository_id = r.id AND v.ingested
             GROUP BY r.id
             ORDER BY r.name",
            json!({}),
        )
        .await;

    let ingestion_jobs = state.ingestion.read().await;

    let mut ingestion_statuses: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
    for (name, job) in ingestion_jobs.iter() {
        let status = job.status.read().await;
        let (s, e) = match &*status {
            IngestionStatus::Running => (Some("running".to_string()), None),
            IngestionStatus::Failed { error } => (Some("failed".to_string()), Some(error.clone())),
            IngestionStatus::Completed { .. } => (Some("completed".to_string()), None),
            IngestionStatus::Pending => (Some("pending".to_string()), None),
        };
        ingestion_statuses.insert(name.clone(), (s, e));
    }

    match result {
        Ok(rows) => {
            let mut repos: Vec<RepositoryInfo> = rows
                .into_iter()
                .map(|row| {
                    let name = row["name"].as_str().unwrap_or("").to_string();
                    let (ingestion_status, ingestion_error) =
                        ingestion_statuses.get(&name).cloned().unwrap_or((None, None));
                    RepositoryInfo {
                        name: name.clone(),
                        url: row["url"].as_str().map(String::from),
                        versions: row["versions"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect(),
                        ingestion_status,
                        ingestion_error,
                    }
                })
                .collect();

            for (name, (ingestion_status, ingestion_error)) in ingestion_statuses.iter() {
                let exists = repos.iter().any(|r| r.name == *name);
                if !exists {
                    repos.push(RepositoryInfo {
                        name: name.clone(),
                        url: None,
                        versions: vec![],
                        ingestion_status: ingestion_status.clone(),
                        ingestion_error: ingestion_error.clone(),
                    });
                }
            }

            repos.sort_by(|a, b| a.name.cmp(&b.name));
            Json(repos).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list repositories");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn handle_add_repository(
    State(state): State<Arc<GraphState>>,
    Json(body): Json<AddRepositoryBody>,
) -> impl IntoResponse {
    let name = body.name.trim().to_string();
    let url = body.url.trim().to_string();

    if name.is_empty() || url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "name and url are required".to_string(),
        )
            .into_response();
    }

    if body.refs.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "at least one ref must be selected".to_string(),
        )
            .into_response();
    }

    {
        let jobs = state.ingestion.read().await;
        if let Some(job) = jobs.get(&name) {
            let status = job.status.read().await;
            if matches!(*status, IngestionStatus::Running | IngestionStatus::Pending) {
                return (
                    StatusCode::CONFLICT,
                    format!("repository '{}' is already being ingested", name),
                )
                    .into_response();
            }
        }
    }

    let registry = Arc::clone(&state.ingestion);
    let db = (*state.db).clone();
    let cache = Arc::clone(&state.cache);
    let name_for_response = name.clone();

    tokio::spawn(async move {
        crate::ingestion::run_ingestion(
            registry.clone(),
            db,
            name.clone(),
            url,
            body.browse_url.clone(),
            body.refs.clone(),
        )
        .await;

        let mut cache_write = cache.write().await;
        let keys_to_remove: Vec<String> = cache_write
            .keys()
            .filter(|k| k.starts_with(&format!("{}:", name)))
            .cloned()
            .collect();
        for key in keys_to_remove {
            cache_write.remove(&key);
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "name": name_for_response, "status": "ingesting" })),
    )
        .into_response()
}

pub async fn handle_verify_repository(
    State(state): State<Arc<GraphState>>,
    Json(body): Json<VerifyRepositoryBody>,
) -> impl IntoResponse {
    let url = body.url.trim().to_string();
    if url.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "url is required".to_string(),
        )
            .into_response();
    }

    let pipeline = match knowledge_harvester::pipeline::Pipeline::with_db((*state.db).clone()) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to initialize: {e}"),
            )
                .into_response();
        }
    };

    match pipeline.list_remote_refs(&url).await {
        Ok(refs) => {
            let ref_infos: Vec<RefInfo> = refs
                .into_iter()
                .map(|t| RefInfo {
                    name: t.name,
                    commit_sha: t.commit_sha,
                })
                .collect();
            Json(VerifyResult {
                url,
                refs: ref_infos,
            })
            .into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("not found") || msg.contains("could not find") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            };
            (status, msg).into_response()
        }
    }
}

pub async fn handle_delete_repository(
    State(state): State<Arc<GraphState>>,
    Path(repo): Path<String>,
) -> impl IntoResponse {
    {
        let jobs = state.ingestion.read().await;
        if let Some(job) = jobs.get(&repo) {
            let status = job.status.read().await;
            if matches!(*status, IngestionStatus::Running | IngestionStatus::Pending) {
                return (
                    StatusCode::CONFLICT,
                    format!("cannot delete repository '{}' while it is being ingested", repo),
                )
                    .into_response();
            }
        }
    }

    let result = state.db
        .execute(
            "DELETE FROM repositories WHERE name = $repo",
            json!({ "repo": repo }),
        )
        .await;

    match result {
        Ok(_) => {
            let mut cache_write = state.cache.write().await;
            let keys_to_remove: Vec<String> = cache_write
                .keys()
                .filter(|k| k.starts_with(&format!("{}:", repo)))
                .cloned()
                .collect();
            for key in keys_to_remove {
                cache_write.remove(&key);
            }

            let mut jobs = state.ingestion.write().await;
            jobs.remove(&repo);

            Json(json!({ "deleted": repo })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, repo = %repo, "failed to delete repository");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct IngestVersionsBody {
    pub refs: Vec<String>,
}

pub async fn handle_ingest_versions(
    State(state): State<Arc<GraphState>>,
    Path(repo): Path<String>,
    Json(body): Json<IngestVersionsBody>,
) -> impl IntoResponse {
    if body.refs.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "at least one ref must be selected".to_string(),
        )
            .into_response();
    }

    let url_result = state
        .db
        .query(
            "SELECT url FROM repositories WHERE name = $repo",
            json!({ "repo": repo }),
        )
        .await;

    let url = match url_result {
        Ok(rows) => rows
            .into_iter()
            .next()
            .and_then(|r| r["url"].as_str().map(String::from)),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
                .into_response();
        }
    };

    let url = match url {
        Some(u) => u,
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("repository '{}' not found", repo),
            )
                .into_response();
        }
    };

    {
        let jobs = state.ingestion.read().await;
        if let Some(job) = jobs.get(&repo) {
            let status = job.status.read().await;
            if matches!(*status, IngestionStatus::Running | IngestionStatus::Pending) {
                return (
                    StatusCode::CONFLICT,
                    format!("repository '{}' is already being ingested", repo),
                )
                    .into_response();
            }
        }
    }

    let registry = Arc::clone(&state.ingestion);
    let db = (*state.db).clone();
    let cache = Arc::clone(&state.cache);
    let repo_name = repo.clone();

    tokio::spawn(async move {
        crate::ingestion::run_ingestion(
            registry.clone(),
            db,
            repo_name.clone(),
            url,
            None,
            body.refs.clone(),
        )
        .await;

        let mut cache_write = cache.write().await;
        let keys_to_remove: Vec<String> = cache_write
            .keys()
            .filter(|k| k.starts_with(&format!("{}:", repo_name)))
            .cloned()
            .collect();
        for key in keys_to_remove {
            cache_write.remove(&key);
        }
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "name": repo, "status": "ingesting" })),
    )
        .into_response()
}

pub async fn handle_resync_version(
    State(state): State<Arc<GraphState>>,
    Path((repo, version)): Path<(String, String)>,
) -> impl IntoResponse {
    {
        let jobs = state.ingestion.read().await;
        if let Some(job) = jobs.get(&repo) {
            let status = job.status.read().await;
            if matches!(*status, IngestionStatus::Running | IngestionStatus::Pending) {
                return (
                    StatusCode::CONFLICT,
                    format!("repository '{}' is already being ingested", repo),
                )
                    .into_response();
            }
        }
    }

    let url_result = state
        .db
        .query(
            "SELECT url FROM repositories WHERE name = $repo",
            json!({ "repo": repo }),
        )
        .await;

    let url = match url_result {
        Ok(rows) => rows
            .into_iter()
            .next()
            .and_then(|r| r["url"].as_str().map(String::from)),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
                .into_response();
        }
    };

    let url = match url {
        Some(u) => u,
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("repository '{}' not found", repo),
            )
                .into_response();
        }
    };

    let registry = Arc::clone(&state.ingestion);
    let db = (*state.db).clone();
    let cache = Arc::clone(&state.cache);
    let repo_name = repo.clone();
    let version_name = version.clone();

    tokio::spawn(async move {
        crate::ingestion::run_resync(
            registry.clone(),
            db,
            repo_name.clone(),
            url,
            version_name.clone(),
        )
        .await;

        let mut cache_write = cache.write().await;
        let key = format!("{}:{}", repo_name, version_name);
        cache_write.remove(&key);
    });

    (
        StatusCode::ACCEPTED,
        Json(json!({ "name": repo, "version": version, "status": "resyncing" })),
    )
        .into_response()
}

pub async fn handle_delete_version(
    State(state): State<Arc<GraphState>>,
    Path((repo, version)): Path<(String, String)>,
) -> impl IntoResponse {
    {
        let jobs = state.ingestion.read().await;
        if let Some(job) = jobs.get(&repo) {
            let status = job.status.read().await;
            if matches!(*status, IngestionStatus::Running | IngestionStatus::Pending) {
                return (
                    StatusCode::CONFLICT,
                    format!("cannot delete version while repository '{}' is being ingested", repo),
                )
                    .into_response();
            }
        }
    }

    let result = state.db
        .execute(
            "DELETE FROM versions v USING repositories r
             WHERE v.repository_id = r.id AND r.name = $repo AND v.tag = $version",
            json!({ "repo": repo, "version": version }),
        )
        .await;

    match result {
        Ok(_) => {
            let mut cache_write = state.cache.write().await;
            let key = format!("{}:{}", repo, version);
            cache_write.remove(&key);

            Json(json!({ "deleted": version, "repo": repo })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, repo = %repo, version = %version, "failed to delete version");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

pub async fn handle_repository_progress(
    State(state): State<Arc<GraphState>>,
    Path(repo): Path<String>,
) -> impl IntoResponse {
    let jobs = state.ingestion.read().await;
    let job = match jobs.get(&repo) {
        Some(j) => Arc::clone(j),
        None => {
            drop(jobs);
            return (
                StatusCode::NOT_FOUND,
                "no ingestion job found for this repository".to_string(),
            )
                .into_response();
        }
    };
    drop(jobs);

    let mut rx = job.progress.subscribe();
    let progress = job.progress.clone();
    let job_clone = Arc::clone(&job);

    let (tx, rx_sse) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        let mut sent_count = 0;

        loop {
            let lines = progress.lines().await;
            while sent_count < lines.len() {
                let line = &lines[sent_count];
                if tx
                    .send(Ok(Event::default().data(
                        json!({ "type": "progress", "message": line }).to_string(),
                    )))
                    .await
                    .is_err()
                {
                    return;
                }
                sent_count += 1;
            }

            if {
                let status = job_clone.status.read().await;
                matches!(*status, IngestionStatus::Completed { .. } | IngestionStatus::Failed { .. })
            } {
                let status = job_clone.status.read().await;
                let final_event = match &*status {
                    IngestionStatus::Completed { versions } => Event::default().data(
                        json!({ "type": "completed", "versions": versions }).to_string(),
                    ),
                    IngestionStatus::Failed { error } => Event::default().data(
                        json!({ "type": "failed", "error": error }).to_string(),
                    ),
                    _ => Event::default().data(json!({ "type": "done" }).to_string()),
                };
                let _ = tx.send(Ok(final_event)).await;
                return;
            }

            if rx.changed().await.is_err() {
                return;
            }
        }
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx_sse);

    let mut response =
        Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    response
}

pub async fn handle_get_repository_stats(
    State(state): State<Arc<GraphState>>,
    Path(repo): Path<String>,
    Query(params): Query<StatsQuery>,
) -> impl IntoResponse {
    let versions_result = state
        .db
        .query(
            "SELECT tag, timestamp FROM code_versions
             WHERE repo = $repo AND ingested
             ORDER BY timestamp DESC",
            json!({ "repo": repo }),
        )
        .await;

    let versions_rows = match versions_result {
        Ok(rows) => rows,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
                .into_response();
        }
    };

    if versions_rows.is_empty() {
        return (StatusCode::NOT_FOUND, "repository not found or has no ingested versions").into_response();
    }

    let versions: Vec<String> = versions_rows
        .iter()
        .filter_map(|r| r["tag"].as_str().map(String::from))
        .collect();

    let version = params
        .version
        .filter(|v| versions.contains(v))
        .unwrap_or_else(|| versions[0].clone());

    let p = || json!({ "repo": repo, "version": version });

    let url_result = state
        .db
        .query(
            "SELECT url FROM repositories WHERE name = $repo",
            json!({ "repo": repo }),
        )
        .await;

    let url = url_result
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|r| r["url"].as_str().map(String::from));

    let (lang_result, symbol_result, rel_result, file_count_result, symbol_count_result) = tokio::join!(
        state.db.query(
            "SELECT language, count(*) AS count FROM code_files
             WHERE repo = $repo AND version = $version
             GROUP BY language ORDER BY count DESC",
            p(),
        ),
        state.db.query(
            "SELECT label AS type, COALESCE(kind, lower(label)) AS kind, count(*) AS count
             FROM code_symbols WHERE repo = $repo AND version = $version
             GROUP BY 1, 2 ORDER BY count DESC",
            p(),
        ),
        state.db.query(
            "SELECT relation, count(*) AS count FROM code_edges
             WHERE repo = $repo AND version = $version
             GROUP BY relation ORDER BY count DESC",
            p(),
        ),
        state.db.query(
            "SELECT count(*) AS total FROM code_files WHERE repo = $repo AND version = $version",
            p(),
        ),
        state.db.query(
            "SELECT count(*) AS total FROM code_symbols WHERE repo = $repo AND version = $version",
            p(),
        ),
    );

    let lang_rows = match lang_result {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let symbol_rows = match symbol_result {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let rel_rows = match rel_result {
        Ok(r) => r,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let file_count = file_count_result
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|r| r["total"].as_i64())
        .unwrap_or(0);
    let symbol_count = symbol_count_result
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|r| r["total"].as_i64())
        .unwrap_or(0);

    let total_files: i64 = lang_rows
        .iter()
        .filter_map(|r| r["count"].as_i64())
        .sum();

    let languages: Vec<LanguageStat> = lang_rows
        .into_iter()
        .filter_map(|r| {
            let language = r["language"].as_str()?.to_string();
            let files = r["count"].as_i64()?;
            let percentage = if total_files > 0 {
                (files as f64 / total_files as f64) * 100.0
            } else {
                0.0
            };
            Some(LanguageStat {
                language,
                files,
                percentage,
            })
        })
        .collect();

    let symbols: Vec<SymbolStat> = symbol_rows
        .into_iter()
        .filter_map(|r| {
            Some(SymbolStat {
                node_type: r["type"].as_str()?.to_string(),
                kind: r["kind"].as_str()?.to_string(),
                count: r["count"].as_i64()?,
            })
        })
        .collect();

    let relations: Vec<RelationStat> = rel_rows
        .into_iter()
        .filter_map(|r| {
            Some(RelationStat {
                relation: r["relation"].as_str()?.to_string(),
                count: r["count"].as_i64()?,
            })
        })
        .collect();

    Json(RepositoryStats {
        repository: repo,
        url,
        version,
        versions,
        languages,
        symbols,
        relations,
        totals: RepositoryTotals {
            files: file_count,
            symbols: symbol_count,
        },
    })
    .into_response()
}
