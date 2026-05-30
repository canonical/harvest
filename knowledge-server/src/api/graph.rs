use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

use crate::neo4j::Neo4jClient;

#[derive(Serialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    pub start_line: i64,
    pub signature: Option<String>,
}

#[derive(Serialize)]
pub struct GraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
}

#[derive(Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub truncated: bool,
}

#[derive(Deserialize)]
pub struct SourceParams {
    pub file: String,
    pub name: String,
}

#[derive(Serialize)]
pub struct SymbolSource {
    pub name: String,
    pub file: String,
    pub kind: String,
    pub start_line: i64,
    pub end_line: Option<i64>,
    pub signature: Option<String>,
    pub source: Option<String>,
}

pub async fn handle_get_graph(
    State(neo4j): State<Arc<Neo4jClient>>,
    Path((repo, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let node_rows = match neo4j
        .query_read(
            "MATCH (n {repo: $repo, version: $version})
             WHERE n:Function OR n:Class
             RETURN n.name AS name, n.file AS file, labels(n)[0] AS kind,
                    coalesce(n.start_line, 0) AS start_line, n.signature AS signature
             ORDER BY n.file, n.start_line
             LIMIT 301",
            json!({ "repo": repo, "version": version }),
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "graph: node query failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let edge_rows = match neo4j
        .query_read(
            "MATCH (a {repo: $repo, version: $version})-[:CALLS]->(b {repo: $repo, version: $version})
             WHERE (a:Function OR a:Class) AND (b:Function OR b:Class)
             RETURN a.file AS src_file, a.name AS src_name,
                    b.file AS tgt_file, b.name AS tgt_name
             LIMIT 1000",
            json!({ "repo": repo, "version": version }),
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!(error = %e, "graph: edge query failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let truncated = node_rows.len() > 300;
    let mut seen_ids = HashSet::new();

    let nodes: Vec<GraphNode> = node_rows
        .into_iter()
        .take(300)
        .filter_map(|r| {
            let name = r["name"].as_str()?.to_string();
            let file = r["file"].as_str()?.to_string();
            let id = format!("{}:{}", file, name);
            if !seen_ids.insert(id.clone()) {
                return None;
            }
            Some(GraphNode {
                id,
                kind: r["kind"].as_str().unwrap_or("Function").to_string(),
                start_line: r["start_line"].as_i64().unwrap_or(0),
                signature: r["signature"].as_str().map(String::from),
                name,
                file,
            })
        })
        .collect();

    let node_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut seen_edges = HashSet::new();

    let edges: Vec<GraphEdge> = edge_rows
        .iter()
        .filter_map(|r| {
            let src = format!("{}:{}", r["src_file"].as_str()?, r["src_name"].as_str()?);
            let tgt = format!("{}:{}", r["tgt_file"].as_str()?, r["tgt_name"].as_str()?);
            if !node_ids.contains(&src) || !node_ids.contains(&tgt) || src == tgt {
                return None;
            }
            let key = format!("{}->{}", src, tgt);
            if !seen_edges.insert(key.clone()) {
                return None;
            }
            Some(GraphEdge { id: key, source: src, target: tgt })
        })
        .collect();

    Json(GraphData { nodes, edges, truncated }).into_response()
}

pub async fn handle_get_symbol_source(
    State(neo4j): State<Arc<Neo4jClient>>,
    Path((repo, version)): Path<(String, String)>,
    Query(params): Query<SourceParams>,
) -> impl IntoResponse {
    let rows = match neo4j
        .query_read(
            "MATCH (n {repo: $repo, version: $version, file: $file, name: $name})
             WHERE n:Function OR n:Class
             RETURN n.name AS name, n.file AS file, labels(n)[0] AS kind,
                    coalesce(n.start_line, 0) AS start_line, n.end_line AS end_line,
                    n.signature AS signature, n.source AS source
             LIMIT 1",
            json!({
                "repo": repo, "version": version,
                "file": params.file, "name": params.name
            }),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "symbol source fetch failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    match rows.into_iter().next() {
        Some(row) => Json(SymbolSource {
            name: row["name"].as_str().unwrap_or("").to_string(),
            file: row["file"].as_str().unwrap_or("").to_string(),
            kind: row["kind"].as_str().unwrap_or("Function").to_string(),
            start_line: row["start_line"].as_i64().unwrap_or(0),
            end_line: row["end_line"].as_i64(),
            signature: row["signature"].as_str().map(String::from),
            source: row["source"].as_str().map(String::from),
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "symbol not found").into_response(),
    }
}
