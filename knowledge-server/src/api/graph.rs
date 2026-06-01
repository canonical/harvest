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

use crate::api::GraphState;
use crate::neo4j::Neo4jClient;

pub use crate::api::GraphCache;

/// Hard caps to keep the browser responsive on large repositories.
const MAX_NODES: usize = 1500;
const MAX_EDGES: usize = 6000;

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
    pub relation: String,
}

#[derive(Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// True when the graph exceeded MAX_NODES and was truncated.
    pub truncated: bool,
    /// Total symbol count before truncation.
    pub total_nodes: usize,
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

// ── Core computation (runs all Neo4j queries in parallel) ─────────────────────

async fn fetch_graph_data(neo4j: &Neo4jClient, repo: &str, version: &str) -> Result<GraphData, String> {
    let p = || json!({ "repo": repo, "version": version });

    let (r_nodes, r_calls, r_contains, r_impl_contains, r_inherits, r_implements, r_uses, r_embeds) =
        tokio::join!(
            neo4j.query_read(
                "MATCH (n {repo: $repo, version: $version})
                 WHERE n:Function OR n:Class
                 RETURN n.name AS name, n.file AS file,
                        coalesce(n.kind, toLower(labels(n)[0])) AS kind,
                        coalesce(n.start_line, 0) AS start_line, n.signature AS signature
                 ORDER BY n.file, n.start_line",
                p(),
            ),
            neo4j.query_read(
                "MATCH (a {repo: $repo, version: $version})-[:CALLS]->(b {repo: $repo, version: $version})
                 WHERE (a:Function OR a:Class) AND (b:Function OR b:Class)
                 RETURN a.file AS src_file, a.name AS src_name,
                        b.file AS tgt_file, b.name AS tgt_name",
                p(),
            ),
            neo4j.query_read(
                "MATCH (fn:Function {repo: $repo, version: $version}),
                       (cls:Class   {repo: $repo, version: $version})
                 WHERE fn.file = cls.file
                   AND cls.start_line <= fn.start_line
                   AND fn.end_line    <= cls.end_line
                 WITH fn, cls
                 ORDER BY (cls.end_line - cls.start_line) ASC
                 WITH fn, collect(cls)[0] AS innermost
                 RETURN fn.file   AS fn_file,  fn.name   AS fn_name,
                        innermost.file AS cls_file, innermost.name AS cls_name",
                p(),
            ),
            neo4j.query_read(
                "MATCH (fn:Function {repo: $repo, version: $version})
                 WHERE fn.impl_type IS NOT NULL
                 MATCH (cls:Class {repo: $repo, version: $version})
                 WHERE cls.name = fn.impl_type
                 RETURN fn.file  AS fn_file,  fn.name  AS fn_name,
                        cls.file AS cls_file, cls.name AS cls_name",
                p(),
            ),
            neo4j.query_read(
                "MATCH (child:Class {repo: $repo, version: $version})-[:INHERITS]->(parent:Class {repo: $repo, version: $version})
                 RETURN child.file AS child_file, child.name AS child_name,
                        parent.file AS parent_file, parent.name AS parent_name",
                p(),
            ),
            neo4j.query_read(
                "MATCH (impl:Class {repo: $repo, version: $version})-[:IMPLEMENTS]->(t:Class {repo: $repo, version: $version})
                 RETURN impl.file AS impl_file, impl.name AS impl_name,
                        t.file    AS trait_file, t.name    AS trait_name",
                p(),
            ),
            neo4j.query_read(
                "MATCH (user:Class {repo: $repo, version: $version})-[:USES]->(used:Class {repo: $repo, version: $version})
                 RETURN user.file AS user_file, user.name AS user_name,
                        used.file AS used_file, used.name AS used_name",
                p(),
            ),
            neo4j.query_read(
                "MATCH (outer:Class {repo: $repo, version: $version})-[:EMBEDS]->(inner:Class {repo: $repo, version: $version})
                 RETURN outer.file AS outer_file, outer.name AS outer_name,
                        inner.file AS inner_file, inner.name AS inner_name",
                p(),
            ),
        );

    macro_rules! require {
        ($result:expr, $label:literal) => {
            $result.map_err(|e| {
                tracing::error!(error = %e, "graph: {} failed", $label);
                e.to_string()
            })?
        };
    }

    let node_rows          = require!(r_nodes,          "node query");
    let edge_rows          = require!(r_calls,           "calls query");
    let contains_rows      = require!(r_contains,        "contains query");
    let impl_contains_rows = require!(r_impl_contains,   "impl_contains query");
    let inherits_rows      = require!(r_inherits,        "inherits query");
    let implements_rows    = require!(r_implements,      "implements query");
    let uses_rows          = require!(r_uses,            "uses query");
    let embeds_rows        = require!(r_embeds,          "embeds query");

    // ── Assemble nodes ───────────────────────────────────────────────────────
    let mut seen_ids = HashSet::new();
    let mut nodes: Vec<GraphNode> = node_rows
        .into_iter()
        .filter_map(|r| {
            let name = r["name"].as_str()?.to_string();
            let file = r["file"].as_str()?.to_string();
            let id = format!("{}:{}", file, name);
            if !seen_ids.insert(id.clone()) { return None; }
            Some(GraphNode {
                id,
                kind: r["kind"].as_str().unwrap_or("function").to_string(),
                start_line: r["start_line"].as_i64().unwrap_or(0),
                signature: r["signature"].as_str().map(String::from),
                name,
                file,
            })
        })
        .collect();

    // Truncate oversized graphs: keep type-like symbols first (class/struct/trait/…),
    // then functions. Within each priority group the original file/line order is preserved.
    let total_nodes = nodes.len();
    if total_nodes > MAX_NODES {
        fn kind_priority(k: &str) -> u8 {
            match k {
                "class" | "struct" | "trait" | "interface" | "enum"
                | "module" | "impl" | "type" => 0,
                _ => 1,
            }
        }
        nodes.sort_by_key(|n| kind_priority(&n.kind));
        nodes.truncate(MAX_NODES);
    }

    let node_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let mut seen_edges = HashSet::new();

    // ── Assemble edges ───────────────────────────────────────────────────────
    let mut edges: Vec<GraphEdge> = edge_rows
        .iter()
        .filter_map(|r| {
            let src = format!("{}:{}", r["src_file"].as_str()?, r["src_name"].as_str()?);
            let tgt = format!("{}:{}", r["tgt_file"].as_str()?, r["tgt_name"].as_str()?);
            if !node_ids.contains(&src) || !node_ids.contains(&tgt) || src == tgt { return None; }
            let key = format!("calls:{}>{}", src, tgt);
            if !seen_edges.insert(key.clone()) { return None; }
            Some(GraphEdge { id: key, source: src, target: tgt, relation: "calls".into() })
        })
        .collect();

    for rows in [&contains_rows, &impl_contains_rows] {
        for r in rows {
            let Some(cf) = r["cls_file"].as_str() else { continue };
            let Some(cn) = r["cls_name"].as_str() else { continue };
            let Some(ff) = r["fn_file"].as_str()  else { continue };
            let Some(fn_) = r["fn_name"].as_str() else { continue };
            let src = format!("{}:{}", cf, cn);
            let tgt = format!("{}:{}", ff, fn_);
            if !node_ids.contains(&src) || !node_ids.contains(&tgt) { continue; }
            let key = format!("contains:{}>{}", src, tgt);
            if !seen_edges.insert(key.clone()) { continue; }
            edges.push(GraphEdge { id: key, source: src, target: tgt, relation: "contains".into() });
        }
    }

    macro_rules! add_edges {
        ($rows:expr, $rel:literal, $sf:literal, $sn:literal, $tf:literal, $tn:literal) => {
            for r in $rows {
                let Some(sf) = r[$sf].as_str() else { continue };
                let Some(sn) = r[$sn].as_str() else { continue };
                let Some(tf) = r[$tf].as_str() else { continue };
                let Some(tn) = r[$tn].as_str() else { continue };
                let src = format!("{}:{}", sf, sn);
                let tgt = format!("{}:{}", tf, tn);
                if !node_ids.contains(&src) || !node_ids.contains(&tgt) { continue; }
                let key = format!("{}:{}>{}", $rel, src, tgt);
                if !seen_edges.insert(key.clone()) { continue; }
                edges.push(GraphEdge { id: key, source: src, target: tgt, relation: $rel.into() });
            }
        };
    }

    add_edges!(&inherits_rows,   "inherits",   "child_file", "child_name", "parent_file", "parent_name");
    add_edges!(&implements_rows, "implements", "impl_file",  "impl_name",  "trait_file",  "trait_name");
    add_edges!(&embeds_rows,     "embeds",     "outer_file", "outer_name", "inner_file",  "inner_name");
    add_edges!(&uses_rows,       "uses",       "user_file",  "user_name",  "used_file",   "used_name");

    // Cap edges: prefer structural edges over call graph edges.
    if edges.len() > MAX_EDGES {
        edges.retain(|e| e.relation != "calls");
    }
    if edges.len() > MAX_EDGES {
        edges.retain(|e| e.relation != "uses");
    }
    if edges.len() > MAX_EDGES {
        edges.truncate(MAX_EDGES);
    }

    Ok(GraphData { nodes, edges, truncated: total_nodes > MAX_NODES, total_nodes })
}

// ── Cache pre-warming (called at server startup) ──────────────────────────────

pub async fn warm_graph_cache(neo4j: Arc<Neo4jClient>, cache: Arc<GraphCache>) {
    let pairs = match neo4j
        .query_read(
            "MATCH (v:Version {ingested: true}) RETURN v.repo AS repo, v.tag AS version",
            json!({}),
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => { tracing::error!(error = %e, "cache warm: failed to list versions"); return; }
    };

    if pairs.is_empty() {
        tracing::info!("graph cache: no ingested versions found");
        return;
    }

    tracing::info!(count = pairs.len(), "pre-warming graph cache");

    for row in pairs {
        let Some(repo)    = row["repo"].as_str()    else { continue };
        let Some(version) = row["version"].as_str() else { continue };
        let key = format!("{}:{}", repo, version);

        if cache.read().await.contains_key(&key) { continue; }

        match fetch_graph_data(&neo4j, repo, version).await {
            Ok(data) => {
                let json = serde_json::to_string(&data).unwrap_or_default();
                cache.write().await.insert(key, Arc::new(json));
                tracing::info!(repo, version, "graph cached");
            }
            Err(e) => tracing::warn!(repo, version, error = %e, "graph cache warm failed"),
        }
    }

    tracing::info!("graph cache ready");
}

// ── HTTP handlers ─────────────────────────────────────────────────────────────

pub async fn handle_get_graph(
    State(state): State<Arc<GraphState>>,
    Path((repo, version)): Path<(String, String)>,
) -> impl IntoResponse {
    let key = format!("{}:{}", repo, version);

    // Cache hit — return pre-computed JSON directly, no Neo4j queries needed.
    if let Some(cached) = state.cache.read().await.get(&key) {
        let json = Arc::clone(cached);
        return ([("content-type", "application/json")], json.as_ref().to_owned()).into_response();
    }

    // Cache miss — compute, store, and return.
    match fetch_graph_data(&state.neo4j, &repo, &version).await {
        Ok(data) => {
            let json = serde_json::to_string(&data).unwrap_or_default();
            state.cache.write().await.insert(key, Arc::new(json.clone()));
            ([("content-type", "application/json")], json).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn handle_get_symbol_source(
    State(state): State<Arc<GraphState>>,
    Path((repo, version)): Path<(String, String)>,
    Query(params): Query<SourceParams>,
) -> impl IntoResponse {
    let rows = match state.neo4j
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
            kind: row["kind"].as_str().unwrap_or("function").to_string(),
            start_line: row["start_line"].as_i64().unwrap_or(0),
            end_line: row["end_line"].as_i64(),
            signature: row["signature"].as_str().map(String::from),
            source: row["source"].as_str().map(String::from),
        })
        .into_response(),
        None => (StatusCode::NOT_FOUND, "symbol not found").into_response(),
    }
}
