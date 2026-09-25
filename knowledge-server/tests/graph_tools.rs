use std::sync::Arc;
use serde_json::{json, Value};

use harvest_db::test_support::TestDb;
use knowledge_harvester::graph::model::{CallRef, ClassNode, FunctionNode, ImportNode, ParsedFile};
use knowledge_harvester::graph::writer::GraphWriter;
use knowledge_server::agent::graph_tools::*;
use knowledge_server::agent::tool::Tool as _;

macro_rules! setup {
    ($client:ident, $test_db:ident) => {
        let $test_db = TestDb::new().await;
        seed_graph(&GraphWriter::new($test_db.db.clone())).await;
        let $client = Arc::new($test_db.db.clone());
    };
}

fn function(version: &str, name: &str, signature: &str, lines: (u32, u32), source: &str, calls: Vec<CallRef>) -> FunctionNode {
    FunctionNode {
        repo: "myrepo".into(), version: version.into(), file: "src/lib.rs".into(),
        name: name.into(), kind: "function".into(), signature: signature.into(),
        start_line: lines.0, end_line: lines.1, source: source.into(), impl_type: None, calls,
    }
}

async fn seed_graph(writer: &GraphWriter) {
    writer.upsert_repository("myrepo", "https://example.com/myrepo.git").await.unwrap();

    let v1 = ParsedFile {
        path: "src/lib.rs".into(),
        language: "rust".into(),
        functions: vec![
            function("v1.0", "alpha", "fn alpha(x: i32)", (1, 5), "fn alpha(x: i32) { beta(); }",
                     vec![CallRef { callee: "beta".into(), line: 2 }]),
            function("v1.0", "beta", "fn beta()", (7, 9), "fn beta() {}", vec![]),
        ],
        classes: vec![ClassNode {
            repo: "myrepo".into(), version: "v1.0".into(), file: "src/lib.rs".into(),
            name: "MyStruct".into(), kind: "struct".into(), start_line: 11, end_line: 13,
            source: "struct MyStruct { x: i32 }".into(),
            bases: vec![], traits: vec![], embeds: vec![], uses: vec![],
        }],
        imports: vec![ImportNode {
            repo: "myrepo".into(), version: "v1.0".into(), file: "src/lib.rs".into(),
            target: "std::collections::HashMap".into(), line: 1,
        }],
    };
    let v2 = ParsedFile {
        path: "src/lib.rs".into(),
        language: "rust".into(),
        functions: vec![
            function("v2.0", "alpha", "fn alpha(value: i32)", (1, 5), "fn alpha(value: i32) { beta(); }", vec![]),
            function("v2.0", "beta", "fn beta()", (7, 9), "fn beta() {}", vec![]),
        ],
        classes: vec![],
        imports: vec![],
    };

    writer.upsert_version("myrepo", "v1.0", 1000, false).await.unwrap();
    writer.write_version("myrepo", "v1.0", &[v1]).await.unwrap();
    writer.upsert_version("myrepo", "v2.0", 2000, false).await.unwrap();
    writer.write_version("myrepo", "v2.0", &[v2]).await.unwrap();
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_repositories_returns_ingested_repos() {
    setup!(client, test_db);
    let tool = ListRepositoriesTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(&tool.execute(json!({})).await.unwrap()).unwrap();
    let repos = repos_from(&result);
    assert!(repos.contains(&"myrepo".to_string()), "repos: {repos:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn list_repositories_empty_graph_returns_empty_array() {
    let test_db = TestDb::new().await;
    let tool = ListRepositoriesTool(Arc::new(test_db.db.clone()));
    let result: Vec<Value> = serde_json::from_str(&tool.execute(json!({})).await.unwrap()).unwrap();
    assert!(result.is_empty());
}


#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_finds_function_by_name() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "alpha"})).await.unwrap()
    ).unwrap();
    let names = names_from(&result);
    assert!(names.iter().any(|n| n == "alpha"), "names: {names:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_repo_filter_limits_results() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "alpha", "repo": "myrepo"})).await.unwrap()
    ).unwrap();
    for row in &result {
        assert_eq!(row["repo"].as_str(), Some("myrepo"));
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_version_filter_limits_results() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "alpha", "version": "v1.0"})).await.unwrap()
    ).unwrap();
    for row in &result {
        assert_eq!(row["version"].as_str(), Some("v1.0"));
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_unknown_name_returns_empty() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result = tool.execute(json!({"query": "xyzzy_nonexistent"})).await.unwrap();
    assert!(result.starts_with("No symbols found"), "result: {result}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_matches_fragments_and_ranks_exact_names_first() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "MyStr", "kind": "class"})).await.unwrap()
    ).unwrap();
    assert_eq!(names_from(&result), vec!["MyStruct".to_string()]);
    assert_eq!(result[0]["kind"], "class");

    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "beta", "version": "v1.0"})).await.unwrap()
    ).unwrap();
    assert_eq!(result[0]["name"], "beta");
    assert_eq!(result[0]["score"], 1000.0);
}


#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_symbol_source_returns_source_for_known_function() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_symbol_source_returns_empty_for_unknown_name() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_file_symbols_lists_functions_and_class() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_file_symbols_does_not_include_source_text() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_callers_returns_alpha_as_caller_of_beta() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_callers_returns_empty_for_uncalled_function() {
    setup!(client, test_db);
    let tool = FindCallersTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v1.0", "function_name": "alpha"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty(), "expected no callers for alpha, got: {result:?}");
}


#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_callees_returns_beta_as_callee_of_alpha() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_callees_returns_empty_for_leaf_function() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_imports_returns_seeded_import() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_imports_returns_empty_for_file_with_no_imports() {
    setup!(client, test_db);
    let tool = GetImportsTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "repo": "myrepo", "version": "v2.0", "file": "src/lib.rs"
        })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty());
}


#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn compare_symbol_returns_both_versions() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn compare_symbol_sources_differ_between_versions() {
    setup!(client, test_db);
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
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn run_sql_basic_read_query_works() {
    setup!(client, test_db);
    let tool = RunSqlTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({ "query": "SELECT name FROM code_repositories" })).await.unwrap()
    ).unwrap();
    assert_eq!(names_from(&result), vec!["myrepo".to_string()]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn run_sql_with_params() {
    setup!(client, test_db);
    let tool = RunSqlTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({
            "query": "SELECT name, version FROM code_symbols WHERE name = $name AND version = $ver",
            "params": { "name": "alpha", "ver": "v1.0" }
        })).await.unwrap()
    ).unwrap();
    assert_eq!(result, vec![json!({ "name": "alpha", "version": "v1.0" })]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn run_sql_returns_empty_for_no_matches() {
    setup!(client, test_db);
    let tool = RunSqlTool(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({ "query": "SELECT * FROM code_files WHERE path = 'nope'" })).await.unwrap()
    ).unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn run_sql_rejects_writes() {
    setup!(client, test_db);
    let tool = RunSqlTool(Arc::clone(&client));
    for query in [
        "DELETE FROM repositories",
        "WITH gone AS (DELETE FROM repositories RETURNING name) SELECT name FROM gone",
        "SELECT set_config('default_transaction_read_only', 'off', false)",
    ] {
        let _ = tool.execute(json!({ "query": query })).await;
    }
    let rows = client.query("SELECT count(*) AS n FROM repositories", json!({})).await.unwrap();
    assert_eq!(rows[0]["n"], 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn run_sql_cannot_read_application_tables() {
    setup!(client, test_db);
    client.execute(
        "INSERT INTO users (id, email, name, password_hash, provider, role, created_at)
         VALUES ('u1', 'a@b.c', 'Ann', 'secret-hash', 'local', 'admin', now())",
        json!({}),
    ).await.unwrap();
    let tool = RunSqlTool(Arc::clone(&client));
    let err = tool.execute(json!({ "query": "SELECT password_hash FROM users" })).await.unwrap_err();
    assert!(format!("{err:#}").contains("permission denied"), "error: {err:#}");
}
