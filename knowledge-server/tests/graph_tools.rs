use std::sync::Arc;
use serde_json::{json, Value};

use knowledge_server::agent::graph_tools::*;
use knowledge_server::agent::tool::Tool as _;
use knowledge_server::neo4j::Neo4jClient;

use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j, Neo4jImageExt as _};
use neo4rs::{query, Graph};

macro_rules! setup {
    ($client:ident, $container:ident) => {
        let $container = Neo4j::default().start().await;
        let uri = $container.image().bolt_uri_ipv4();
        let user = $container.image().user().unwrap_or("neo4j");
        let pass = $container.image().password().unwrap_or("neo");
        let $client = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());
        seed_graph(&Graph::new(&uri, user, pass).await.unwrap()).await;
    };
}

async fn seed_graph(graph: &Graph) {
    let stmts = [
        "CREATE INDEX repo_name IF NOT EXISTS FOR (r:Repository) ON (r.name)",
        "CREATE INDEX version_key IF NOT EXISTS FOR (v:Version) ON (v.repo, v.tag)",
        "CREATE INDEX file_path IF NOT EXISTS FOR (f:File) ON (f.repo, f.version, f.path)",
        "CREATE INDEX fn_key IF NOT EXISTS FOR (f:Function) ON (f.repo, f.version, f.name)",
        "CREATE INDEX cls_key IF NOT EXISTS FOR (c:Class) ON (c.repo, c.version, c.name)",
        "CREATE FULLTEXT INDEX symbol_names IF NOT EXISTS FOR (n:Function|Class) ON EACH [n.name]",
        "CREATE FULLTEXT INDEX file_paths IF NOT EXISTS FOR (f:File) ON EACH [f.path]",
        "CREATE (:Repository {name: 'myrepo', url: 'https://example.com/myrepo.git'})",
        "CREATE (:Version {repo: 'myrepo', tag: 'v1.0', timestamp: 1000, ingested: true})",
        "CREATE (:Version {repo: 'myrepo', tag: 'v2.0', timestamp: 2000, ingested: true})",
        "MATCH (r:Repository {name:'myrepo'}), (v:Version {repo:'myrepo', tag:'v1.0'}) CREATE (r)-[:HAS_VERSION]->(v)",
        "MATCH (r:Repository {name:'myrepo'}), (v:Version {repo:'myrepo', tag:'v2.0'}) CREATE (r)-[:HAS_VERSION]->(v)",
        "CREATE (:File {repo:'myrepo', version:'v1.0', path:'src/lib.rs', language:'rust'})",
        "MATCH (v:Version {repo:'myrepo',tag:'v1.0'}),(f:File {repo:'myrepo',version:'v1.0',path:'src/lib.rs'}) CREATE (v)-[:HAS_FILE]->(f)",
        "CREATE (:Function {repo:'myrepo',version:'v1.0',file:'src/lib.rs',name:'alpha',signature:'fn alpha(x: i32)',start_line:1,end_line:5,source:'fn alpha(x: i32) { beta(); }'})",
        "CREATE (:Function {repo:'myrepo',version:'v1.0',file:'src/lib.rs',name:'beta', signature:'fn beta()',      start_line:7,end_line:9,source:'fn beta() {}'})",
        "MATCH (f:File {repo:'myrepo',version:'v1.0',path:'src/lib.rs'}),(fn:Function {repo:'myrepo',version:'v1.0',name:'alpha'}) CREATE (f)-[:DEFINES]->(fn)",
        "MATCH (f:File {repo:'myrepo',version:'v1.0',path:'src/lib.rs'}),(fn:Function {repo:'myrepo',version:'v1.0',name:'beta'})  CREATE (f)-[:DEFINES]->(fn)",
        "MATCH (a:Function {repo:'myrepo',version:'v1.0',name:'alpha'}),(b:Function {repo:'myrepo',version:'v1.0',name:'beta'}) CREATE (a)-[:CALLS {line:2}]->(b)",
        "CREATE (:Class {repo:'myrepo',version:'v1.0',file:'src/lib.rs',name:'MyStruct',start_line:11,end_line:13,source:'struct MyStruct { x: i32 }'})",
        "MATCH (f:File {repo:'myrepo',version:'v1.0',path:'src/lib.rs'}),(c:Class {repo:'myrepo',version:'v1.0',name:'MyStruct'}) CREATE (f)-[:DEFINES]->(c)",
        "CREATE (:Import {repo:'myrepo',version:'v1.0',file:'src/lib.rs',target:'std::collections::HashMap',line:1})",
        "MATCH (f:File {repo:'myrepo',version:'v1.0',path:'src/lib.rs'}),(i:Import {repo:'myrepo',version:'v1.0'}) CREATE (f)-[:IMPORTS]->(i)",
        "CREATE (:File {repo:'myrepo', version:'v2.0', path:'src/lib.rs', language:'rust'})",
        "MATCH (v:Version {repo:'myrepo',tag:'v2.0'}),(f:File {repo:'myrepo',version:'v2.0',path:'src/lib.rs'}) CREATE (v)-[:HAS_FILE]->(f)",
        "CREATE (:Function {repo:'myrepo',version:'v2.0',file:'src/lib.rs',name:'alpha',signature:'fn alpha(value: i32)',start_line:1,end_line:5,source:'fn alpha(value: i32) { beta(); }'})",
        "CREATE (:Function {repo:'myrepo',version:'v2.0',file:'src/lib.rs',name:'beta', signature:'fn beta()',           start_line:7,end_line:9,source:'fn beta() {}'})",
        "MATCH (f:File {repo:'myrepo',version:'v2.0',path:'src/lib.rs'}),(fn:Function {repo:'myrepo',version:'v2.0',name:'alpha'}) CREATE (f)-[:DEFINES]->(fn)",
        "MATCH (f:File {repo:'myrepo',version:'v2.0',path:'src/lib.rs'}),(fn:Function {repo:'myrepo',version:'v2.0',name:'beta'})  CREATE (f)-[:DEFINES]->(fn)",
    ];
    for stmt in stmts {
        graph.run(query(stmt)).await.unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

fn names_from(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| r["name"].as_str().map(|s| s.to_string()))
        .collect()
}

fn repos_from(rows: &[Value]) -> Vec<String> {
    rows.iter()
        .filter_map(|r| r["repo"].as_str().map(|s| s.to_string()))
        .collect()
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_repositories_returns_ingested_repos() {
    setup!(client, container);
    let tool = ListRepositoriesTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(&tool.execute(json!({})).await.unwrap()).unwrap();
    let repos = repos_from(&result);
    assert!(repos.contains(&"myrepo".to_string()), "repos: {repos:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn list_repositories_empty_graph_returns_empty_array() {
    let container = Neo4j::default().start().await;
    let uri = container.image().bolt_uri_ipv4();
    let user = container.image().user().unwrap_or("neo4j");
    let pass = container.image().password().unwrap_or("neo");
    let client = Arc::new(Neo4jClient::new(&uri, user, pass).await.unwrap());
    let tool = ListRepositoriesTool(client);
    let result: Vec<Value> = serde_json::from_str(&tool.execute(json!({})).await.unwrap()).unwrap();
    assert!(result.is_empty());
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn search_symbols_finds_function_by_name() {
    setup!(client, container);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "alpha"})).await.unwrap()
    ).unwrap();
    let names = names_from(&result);
    assert!(names.iter().any(|n| n == "alpha"), "names: {names:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn search_symbols_repo_filter_limits_results() {
    setup!(client, container);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "alpha", "repo": "myrepo"})).await.unwrap()
    ).unwrap();
    for row in &result {
        assert_eq!(row["repo"].as_str(), Some("myrepo"));
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn search_symbols_version_filter_limits_results() {
    setup!(client, container);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "alpha", "version": "v1.0"})).await.unwrap()
    ).unwrap();
    for row in &result {
        assert_eq!(row["version"].as_str(), Some("v1.0"));
    }
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn search_symbols_unknown_name_returns_empty() {
    setup!(client, container);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "xyzzy_nonexistent"})).await.unwrap()
    ).unwrap();
    assert!(result.is_empty());
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn get_symbol_source_returns_source_for_known_function() {
    setup!(client, container);
    let tool = GetSymbolSourceTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0",
            "file": "src/lib.rs", "name": "alpha"
        })).await.unwrap()
    ).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0]["source"].as_str().unwrap().contains("alpha"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_symbol_source_returns_empty_for_unknown_name() {
    setup!(client, container);
    let tool = GetSymbolSourceTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0",
            "file": "src/lib.rs", "name": "does_not_exist"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty());
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn get_file_symbols_lists_functions_and_class() {
    setup!(client, container);
    let tool = GetFileSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0", "file": "src/lib.rs"
        })).await.unwrap()
    ).unwrap();
    let names = names_from(&result);
    assert!(names.contains(&"alpha".to_string()), "names: {names:?}");
    assert!(names.contains(&"beta".to_string()),  "names: {names:?}");
    assert!(names.contains(&"MyStruct".to_string()), "names: {names:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_file_symbols_does_not_include_source_text() {
    setup!(client, container);
    let tool = GetFileSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0", "file": "src/lib.rs"
        })).await.unwrap()
    ).unwrap();
    for row in &result {
        assert!(row.get("source").is_none(), "source text should not appear in file symbols");
    }
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn find_callers_returns_alpha_as_caller_of_beta() {
    setup!(client, container);
    let tool = FindCallersTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0", "function_name": "beta"
        })).await.unwrap()
    ).unwrap();
    let callers: Vec<_> = result.iter()
        .filter_map(|r| r["caller"].as_str())
        .collect();
    assert!(callers.contains(&"alpha"), "callers: {callers:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn find_callers_returns_empty_for_uncalled_function() {
    setup!(client, container);
    let tool = FindCallersTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0", "function_name": "alpha"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty(), "expected no callers for alpha, got: {result:?}");
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn find_callees_returns_beta_as_callee_of_alpha() {
    setup!(client, container);
    let tool = FindCalleesTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0",
            "file": "src/lib.rs", "function_name": "alpha"
        })).await.unwrap()
    ).unwrap();
    let callees: Vec<_> = result.iter()
        .filter_map(|r| r["callee"].as_str())
        .collect();
    assert!(callees.contains(&"beta"), "callees: {callees:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn find_callees_returns_empty_for_leaf_function() {
    setup!(client, container);
    let tool = FindCalleesTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0",
            "file": "src/lib.rs", "function_name": "beta"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty(), "beta calls nothing, got: {result:?}");
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn get_imports_returns_seeded_import() {
    setup!(client, container);
    let tool = GetImportsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0", "file": "src/lib.rs"
        })).await.unwrap()
    ).unwrap();
    let targets: Vec<_> = result.iter()
        .filter_map(|r| r["target"].as_str())
        .collect();
    assert!(targets.contains(&"std::collections::HashMap"), "targets: {targets:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn get_imports_returns_empty_for_file_with_no_imports() {
    setup!(client, container);
    let tool = GetImportsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v2.0", "file": "src/lib.rs"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty());
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn compare_symbol_returns_both_versions() {
    setup!(client, container);
    let tool = CompareSymbolAcrossVersionsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version_a": "v1.0", "version_b": "v2.0",
            "file": "src/lib.rs", "name": "alpha"
        })).await.unwrap()
    ).unwrap();
    let versions: Vec<_> = result.iter()
        .filter_map(|r| r["version"].as_str())
        .collect();
    assert!(versions.contains(&"v1.0"), "versions: {versions:?}");
    assert!(versions.contains(&"v2.0"), "versions: {versions:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn compare_symbol_sources_differ_between_versions() {
    setup!(client, container);
    let tool = CompareSymbolAcrossVersionsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version_a": "v1.0", "version_b": "v2.0",
            "file": "src/lib.rs", "name": "alpha"
        })).await.unwrap()
    ).unwrap();
    let v1_source = result.iter().find(|r| r["version"] == "v1.0")
        .and_then(|r| r["source"].as_str()).unwrap_or("");
    let v2_source = result.iter().find(|r| r["version"] == "v2.0")
        .and_then(|r| r["source"].as_str()).unwrap_or("");
    assert_ne!(v1_source, v2_source, "sources should differ between versions");
    assert!(v1_source.contains("x: i32"),     "v1 should have param 'x'");
    assert!(v2_source.contains("value: i32"), "v2 should have param 'value'");
}


#[tokio::test]
#[ignore = "requires Docker"]
async fn run_cypher_basic_read_query_works() {
    setup!(client, container);
    let tool = RunCypherTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "query": "MATCH (r:Repository) RETURN r.name AS name"
        })).await.unwrap()
    ).unwrap();
    let names: Vec<_> = result.iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(names.contains(&"myrepo"), "names: {names:?}");
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_cypher_with_params() {
    setup!(client, container);
    let tool = RunCypherTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "query": "MATCH (fn:Function {name: $name, version: $ver}) RETURN fn.name AS name",
            "params": { "name": "alpha", "ver": "v1.0" }
        })).await.unwrap()
    ).unwrap();
    assert!(!result.is_empty());
    assert_eq!(result[0]["name"].as_str(), Some("alpha"));
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn run_cypher_returns_empty_for_no_matches() {
    setup!(client, container);
    let tool = RunCypherTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "query": "MATCH (n:NonExistentLabel) RETURN n"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty());
}
