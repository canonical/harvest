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
        start_line: lines.0, end_line: lines.1, source: source.into(), impl_type: None, docstring: None, calls,
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
            bases: vec![], traits: vec![], embeds: vec![], uses: vec![], docstring: None, decorators: vec![],
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
async fn list_repositories_flags_versions_indexed_by_an_older_parser() {
    setup!(client, test_db);
    let tool = ListRepositoriesTool(Arc::clone(&client));
    let fresh: Vec<Value> = serde_json::from_str(&tool.execute(json!({})).await.unwrap()).unwrap();
    let myrepo = fresh.iter().find(|r| r["repo"] == "myrepo").unwrap();
    assert!(myrepo.get("stale_versions").is_none(), "freshly written versions are current: {myrepo}");

    test_db.db.execute(
        "UPDATE versions SET parser_version = 0 WHERE tag = 'v1.0'",
        json!({}),
    ).await.unwrap();
    let stale: Vec<Value> = serde_json::from_str(&tool.execute(json!({})).await.unwrap()).unwrap();
    let myrepo = stale.iter().find(|r| r["repo"] == "myrepo").unwrap();
    assert_eq!(myrepo["stale_versions"], json!(["v1.0"]));
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
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
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
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
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
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
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
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let result = tool.execute(json!({"query": "xyzzy_nonexistent"})).await.unwrap();
    assert!(result.starts_with("No symbols found"), "result: {result}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_matches_fragments_and_ranks_exact_names_first() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
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

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_matches_docstring_prose() {
    setup!(client, test_db);
    let writer = GraphWriter::new(client.as_ref().clone());
    writer.upsert_repository("docrepo", "https://example.com/docrepo.git").await.unwrap();
    writer.upsert_version("docrepo", "v1.0", 1, false).await.unwrap();
    writer.write_version("docrepo", "v1.0", &[ParsedFile {
        path: "src/volume.py".into(),
        language: "python".into(),
        functions: vec![FunctionNode {
            repo: "docrepo".into(), version: "v1.0".into(), file: "src/volume.py".into(),
            name: "provision".into(), kind: "function".into(),
            signature: "def provision()".into(), start_line: 1, end_line: 3,
            source: "def provision():\n    return 1".into(),
            impl_type: None,
            docstring: Some("Provisions a block volume over iSCSI.".into()),
            calls: vec![],
        }],
        classes: vec![],
        imports: vec![],
    }]).await.unwrap();

    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "iscsi", "repo": "docrepo", "version": "v1.0"})).await.unwrap()
    ).unwrap();
    assert_eq!(result.len(), 1, "prose search should find the symbol: {result:?}");
    let matched = result[0]["matched_on"].as_array().unwrap();
    assert!(matched.iter().any(|v| v == "docstring"), "matched_on: {matched:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_matches_capability_constant_inside_a_class() {
    setup!(client, test_db);
    let writer = GraphWriter::new(client.as_ref().clone());
    writer.upsert_repository("caprepo", "https://example.com/caprepo.git").await.unwrap();
    writer.upsert_version("caprepo", "v1.0", 1, false).await.unwrap();
    writer.write_version("caprepo", "v1.0", &[ParsedFile {
        path: "pkg/driver.py".into(),
        language: "python".into(),
        functions: vec![],
        classes: vec![ClassNode {
            repo: "caprepo".into(), version: "v1.0".into(), file: "pkg/driver.py".into(),
            name: "NfsDriver".into(), kind: "class".into(), start_line: 1, end_line: 4,
            source: "class NfsDriver(Base):\n    SUPPORTS_ACTIVE_ACTIVE = True\n".into(),
            bases: vec!["Base".into()], traits: vec![], embeds: vec![], uses: vec![], docstring: None, decorators: vec![],
        }],
        imports: vec![],
    }]).await.unwrap();

    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "SUPPORTS_ACTIVE_ACTIVE", "repo": "caprepo", "version": "v1.0"}))
            .await.unwrap()
    ).unwrap();
    assert_eq!(result.len(), 1, "capability constant should be findable: {result:?}");
    assert_eq!(result[0]["name"], "NfsDriver");
    let matched = result[0]["matched_on"].as_array().unwrap();
    assert!(matched.iter().any(|v| v == "capability"), "matched_on: {matched:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_ranks_capability_above_path_match() {
    setup!(client, test_db);
    let writer = GraphWriter::new(client.as_ref().clone());
    writer.upsert_repository("rankrepo", "https://example.com/rankrepo.git").await.unwrap();
    writer.upsert_version("rankrepo", "v1.0", 1, false).await.unwrap();
    writer.write_version("rankrepo", "v1.0", &[ParsedFile {
        path: "pkg/other.py".into(),
        language: "python".into(),
        functions: vec![],
        classes: vec![ClassNode {
            repo: "rankrepo".into(), version: "v1.0".into(), file: "pkg/other.py".into(),
            name: "NfsDriver".into(), kind: "class".into(), start_line: 1, end_line: 3,
            source: "class NfsDriver(Base):\n    SUPPORTS_ACTIVE_ACTIVE = True\n".into(),
            bases: vec![], traits: vec![], embeds: vec![], uses: vec![], docstring: None, decorators: vec![],
        }],
        imports: vec![],
    }, ParsedFile {
        path: "pkg/SUPPORTS_ACTIVE_ACTIVE_helper.py".into(),
        language: "python".into(),
        functions: vec![],
        classes: vec![ClassNode {
            repo: "rankrepo".into(), version: "v1.0".into(),
            file: "pkg/SUPPORTS_ACTIVE_ACTIVE_helper.py".into(),
            name: "Helper".into(), kind: "class".into(), start_line: 1, end_line: 2,
            source: "class Helper:\n    pass".into(),
            bases: vec![], traits: vec![], embeds: vec![], uses: vec![], docstring: None, decorators: vec![],
        }],
        imports: vec![],
    }]).await.unwrap();

    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({"query": "SUPPORTS_ACTIVE_ACTIVE", "repo": "rankrepo", "version": "v1.0"}))
            .await.unwrap()
    ).unwrap();
    assert!(result.len() >= 2, "both should match: {result:?}");
    assert_eq!(result[0]["name"], "NfsDriver", "capability match must outrank a path-only match");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_returns_bounded_previews_not_full_source() {
    setup!(client, test_db);
    let writer = GraphWriter::new(client.as_ref().clone());
    writer.upsert_repository("bigrepo", "https://example.com/bigrepo.git").await.unwrap();
    writer.upsert_version("bigrepo", "v1.0", 1, false).await.unwrap();
    let long: String = std::iter::repeat("z").take(20_000).collect();
    writer.write_version("bigrepo", "v1.0", &[ParsedFile {
        path: "src/big.rs".into(),
        language: "rust".into(),
        functions: vec![FunctionNode {
            repo: "bigrepo".into(), version: "v1.0".into(), file: "src/big.rs".into(),
            name: "huge".into(), kind: "function".into(),
            signature: "fn huge()".into(), start_line: 1, end_line: 2,
            source: format!("fn huge() {{ {long} }}"), impl_type: None, docstring: None, calls: vec![],
        }],
        classes: vec![],
        imports: vec![],
    }]).await.unwrap();

    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let raw = tool.execute(json!({"query": "huge", "repo": "bigrepo", "version": "v1.0"})).await.unwrap();
    let result: Vec<Value> = serde_json::from_str(&raw).unwrap();
    let preview = result[0]["preview"].as_str().unwrap();
    assert!(preview.chars().count() <= 420, "preview must stay bounded: {}", preview.chars().count());
    assert!(result[0].get("source").is_none(), "full source must not be returned in search results");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_tolerates_a_query_with_regex_metacharacters() {
    setup!(client, test_db);
    seed_graph(&GraphWriter::new(client.as_ref().clone())).await;
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    for query in ["fn alpha(", "a(b", "alpha[1]", "a.*", "x$"] {
        let out = tool.execute(json!({ "query": query, "repo": "myrepo", "version": "v1.0" })).await;
        assert!(out.is_ok(), "query {query:?} must not error: {:?}", out.err());
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn read_sources_batches_by_file_and_symbol_name() {
    setup!(client, test_db);
    seed_graph(&GraphWriter::new(client.as_ref().clone())).await;
    let tool = ReadSourcesTool(Arc::clone(&client));
    let out = tool.execute(json!({
        "repo": "myrepo", "version": "v1.0", "files": ["src/lib.rs"]
    })).await.unwrap();
    let rows: Vec<Value> = serde_json::from_str(&out).unwrap();
    assert!(rows.len() >= 3, "should return the file's symbols: {}", rows.len());
    assert!(rows.iter().any(|r| r["name"] == "alpha"));
    assert!(rows.iter().any(|r| r["name"] == "MyStruct"));

    let by_name = tool.execute(json!({
        "repo": "myrepo", "version": "v1.0", "names": ["beta"]
    })).await.unwrap();
    let rows: Vec<Value> = serde_json::from_str(&by_name).unwrap();
    assert_eq!(rows.len(), 1, "name lookup should be exact: {rows:?}");
    assert_eq!(rows[0]["name"], "beta");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn read_sources_clamps_the_limit() {
    setup!(client, test_db);
    seed_graph(&GraphWriter::new(client.as_ref().clone())).await;
    let tool = ReadSourcesTool(Arc::clone(&client));
    let out = tool.execute(json!({
        "repo": "myrepo", "version": "v1.0", "files": ["src/lib.rs"], "limit": 1
    })).await.unwrap();
    let rows: Vec<Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(rows.len(), 1, "limit should be respected: {rows:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_evidence_pack_returns_matrix_and_sources_together() {
    setup!(client, test_db);
    seed_graph(&GraphWriter::new(client.as_ref().clone())).await;
    let tool = GetEvidencePackTool(Arc::clone(&client));
    let out = tool.execute(json!({
        "repo": "myrepo", "version": "v1.0", "symbols": ["alpha"]
    })).await.unwrap();
    assert!(out.contains("capability_matrix"), "pack should carry the matrix section: {out}");
    assert!(out.contains("## symbols"), "pack should carry symbol sources: {out}");
    assert!(out.contains("alpha"), "pack should include the requested symbol: {out}");
    assert!(out.chars().count() <= EVIDENCE_PACK_BUDGET_CHARS + 400, "pack must respect its budget");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn get_evidence_pack_requires_repo_and_version() {
    setup!(client, test_db);
    let tool = GetEvidencePackTool(Arc::clone(&client));
    assert!(tool.execute(json!({ "repo": "myrepo" })).await.is_err());
    assert!(tool.execute(json!({ "version": "v1.0" })).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn all_tools_includes_the_evidence_pack_and_matrix() {
    setup!(client, test_db);
    let names: Vec<String> = knowledge_server::agent::graph_tools::all_tools(Arc::clone(&client))
        .iter().map(|t| t.definition().name).collect();
    for expected in ["get_evidence_pack", "get_capability_matrix", "read_sources", "search_symbols"] {
        assert!(names.iter().any(|n| n == expected), "missing tool {expected} in {names:?}");
    }
}

fn semantic_handle(fail: bool) -> Arc<SemanticHandle> {
    use knowledge_server::agent::semantic::FnEmbedder;
    let embedder: Arc<dyn knowledge_server::agent::semantic::Embedder> = if fail {
        Arc::new(FnEmbedder::failing("text-embedding-004", 768))
    } else {
        Arc::new(FnEmbedder::constant("text-embedding-004", 768, 0.1))
    };
    Arc::new(SemanticHandle::configured(embedder, &knowledge_server::config::SemanticConfig::default()))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_falls_back_to_lexical_when_the_embedding_column_is_absent() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool::with_semantic(Arc::clone(&client), semantic_handle(false));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({ "query": "alpha" })).await.unwrap()
    ).unwrap();
    let names = names_from(&result);
    assert!(names.iter().any(|n| n == "alpha"), "names: {names:?}");
    for row in &result {
        assert!(
            row["matched_on"].as_array().map(|m| m.iter().all(|s| s != "semantic")).unwrap_or(true),
            "the semantic signal must not appear without an embedding column: {row}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_falls_back_to_lexical_when_the_embedder_fails() {
    setup!(client, test_db);
    let tool = SearchSymbolsTool::with_semantic(Arc::clone(&client), semantic_handle(true));
    let result: Vec<Value> = serde_json::from_str(
        &tool.execute(json!({ "query": "alpha" })).await.unwrap()
    ).unwrap();
    assert!(names_from(&result).iter().any(|n| n == "alpha"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn semantic_search_reports_itself_unavailable_without_pgvector() {
    let test_db = TestDb::new().await;
    let available = knowledge_server::agent::semantic::semantic_available(&test_db.db).await.unwrap();
    assert!(!available, "this test database is expected to have no embedding column");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn backfill_refuses_to_run_without_embedding_storage() {
    use knowledge_server::agent::semantic::FnEmbedder;
    let test_db = TestDb::new().await;
    let err = knowledge_server::agent::semantic::backfill_embeddings(
        &test_db.db,
        &FnEmbedder::constant("text-embedding-004", 768, 0.1),
        &knowledge_server::agent::semantic::BackfillOptions {
            model: "text-embedding-004".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("not available"), "unexpected error: {err}");
}

const SEMANTIC_CTE: &str = "), semantic_scored AS (\n    SELECT s.id,\n           (1 - (s.embedding <=> $qvec::vector))::float8 AS semantic_score\n    FROM symbols s\n    WHERE s.embedding IS NOT NULL\n      AND s.embedding_model = $qmodel\n      AND ($repo = '' OR s.repo = $repo)\n      AND ($version = '' OR s.version = $version)\n)";
const SEMANTIC_CTE_STUB: &str = "), semantic_scored AS (\n    SELECT s.id, NULL::float8 AS semantic_score FROM symbols s WHERE false\n)";

async fn hybrid_sql_for(client: &harvest_db::Db) -> String {
    let available: bool = client
        .query(
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector') AS present",
            json!({}),
        )
        .await
        .ok()
        .and_then(|rows| rows.into_iter().next())
        .and_then(|r| r["present"].as_bool())
        .unwrap_or(false);
    if available {
        return HYBRID_SEARCH_SQL.to_string();
    }
    assert!(
        HYBRID_SEARCH_SQL.contains(SEMANTIC_CTE),
        "the semantic CTE must match the stub target"
    );
    HYBRID_SEARCH_SQL.replace(SEMANTIC_CTE, SEMANTIC_CTE_STUB)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn hybrid_sql_executes_the_full_lexical_cte_chain() {
    setup!(client, _test_db);
    let sql = hybrid_sql_for(&client).await;
    let params = json!({
        "query": "alpha", "repo": "myrepo", "version": "", "kind": "any", "limit": 10,
        "path_prefix": "", "offset": 0, "decorator": "",
        "qvec": "[0.0]", "qmodel": "text-embedding-004", "lexical": 0.6, "semantic": 0.4,
    });
    let rows = client
        .query(&sql, params)
        .await
        .expect("the hybrid CTE chain must parse and execute");
    assert!(names_from(&rows).contains(&"alpha".to_string()));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn hybrid_sql_reports_capability_matches() {
    setup!(client, _test_db);
    let sql = hybrid_sql_for(&client).await;
    let params = json!({
        "query": "i32", "repo": "myrepo", "version": "", "kind": "any", "limit": 10,
        "path_prefix": "", "offset": 0, "decorator": "",
        "qvec": "[0.0]", "qmodel": "text-embedding-004", "lexical": 0.6, "semantic": 0.4,
    });
    let rows = client
        .query(&sql, params)
        .await
        .expect("the hybrid CTE chain must parse and execute");
    let my_struct = rows
        .iter()
        .find(|r| r["name"] == "MyStruct")
        .expect("the class should match on the i32 capability in its source");
    let matched: Vec<&str> = my_struct["matched_on"]
        .as_array()
        .expect("matched_on must be an array")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        matched.contains(&"capability"),
        "the capability CTE must contribute to matched_on, got {matched:?}"
    );
    assert!(
        my_struct["score"].as_f64().unwrap_or(0.0) > 0.0,
        "a capability-only match must still score above zero"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn disabling_semantic_search_leaves_the_tool_set_unchanged() {
    setup!(client, _test_db);
    let plain: Vec<String> = all_tools(Arc::clone(&client))
        .iter()
        .map(|t| t.definition().name)
        .collect();
    let with_none: Vec<String> = all_tools_with_semantic(Arc::clone(&client), None)
        .iter()
        .map(|t| t.definition().name)
        .collect();
    assert_eq!(plain, with_none);
    assert!(plain.contains(&"search_symbols".to_string()));
}

fn py_class(repo: &str, file: &str, name: &str, line: u32, bases: &[&str]) -> ClassNode {
    py_decorated_class(repo, file, name, line, bases, &[])
}

fn py_decorated_class(repo: &str, file: &str, name: &str, line: u32, bases: &[&str], decorators: &[&str]) -> ClassNode {
    let declared = if bases.is_empty() { String::new() } else { format!("(mod.{})", bases.join(", mod.")) };
    ClassNode {
        repo: repo.into(), version: "v1".into(), file: file.into(),
        name: name.into(), kind: "class".into(), start_line: line, end_line: line + 2,
        source: format!("class {name}{declared}:\n    pass\n"),
        bases: bases.iter().map(|b| b.to_string()).collect(),
        traits: vec![], embeds: vec![], uses: vec![], docstring: None,
        decorators: decorators.iter().map(|d| d.to_string()).collect(),
    }
}

async fn seed_hierarchy(client: &Arc<harvest_db::Db>) {
    let writer = GraphWriter::new(client.as_ref().clone());
    writer.upsert_repository("tree", "https://example.com/tree.git").await.unwrap();
    writer.upsert_version("tree", "v1", 1, false).await.unwrap();
    let file = |path: &str, classes: Vec<ClassNode>| ParsedFile {
        path: path.into(), language: "python".into(), functions: vec![], classes, imports: vec![],
    };
    let registered = &["registry.driver"];
    writer.write_version("tree", "v1", &[
        file("pkg/root.py", vec![py_class("tree", "pkg/root.py", "Root", 1, &[])]),
        file("pkg/base.py", vec![py_class("tree", "pkg/base.py", "Base", 1, &["Root"])]),
        file("pkg/drivers/direct.py", vec![py_decorated_class("tree", "pkg/drivers/direct.py", "Direct", 2, &["Root"], registered)]),
        file("pkg/drivers/mid.py", vec![py_class("tree", "pkg/drivers/mid.py", "Mid", 3, &["Base"])]),
        file("pkg/drivers/leaf.py", vec![
            py_decorated_class("tree", "pkg/drivers/leaf.py", "LeafA", 5, &["Mid"], registered),
            py_decorated_class("tree", "pkg/drivers/leaf.py", "LeafB", 20, &["Mid", "Base"], registered),
        ]),
        file("pkg/other.py", vec![
            py_decorated_class("tree", "pkg/other.py", "Other", 1, &["Base"], registered),
            py_class("tree", "pkg/other.py", "Unrelated", 9, &[]),
        ]),
    ]).await.unwrap();
}

fn subclass_lines(out: &str) -> Vec<&str> {
    out.lines()
        .filter(|l| l.contains(" <- ") && l.split_whitespace().next().is_some_and(|t| t.contains(':')))
        .collect()
}

fn subclass_names(out: &str) -> Vec<String> {
    subclass_lines(out).iter()
        .filter_map(|l| l.split_whitespace().nth(1))
        .map(|n| n.trim_end_matches('+').to_string())
        .collect()
}

async fn subclasses_of(client: &Arc<harvest_db::Db>, params: Value) -> String {
    let mut params = params;
    params["repo"] = json!("tree");
    params["version"] = json!("v1");
    FindSubclassesTool(Arc::clone(client)).execute(params).await.unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_returns_direct_and_indirect_subclasses() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let out = subclasses_of(&client, json!({ "class": "Base" })).await;
    assert!(out.starts_with("4 subclasses of Base"), "{out}");
    let mut names = subclass_names(&out);
    names.sort();
    assert_eq!(names, ["LeafA", "LeafB", "Mid", "Other"], "{out}");
    let lines = subclass_lines(&out);
    assert!(lines.iter().any(|l| l.starts_with("pkg/drivers/leaf.py:20 LeafB <- Base @registry.driver")), "nearest path wins: {out}");
    assert!(lines.iter().any(|l| l.starts_with("pkg/drivers/leaf.py:5 LeafA <- Mid")), "{out}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_marks_classes_that_have_subclasses() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let out = subclasses_of(&client, json!({ "class": "Root" })).await;
    let lines = subclass_lines(&out);
    assert!(lines.iter().any(|l| l.contains(" Mid+ <- ")), "{out}");
    assert!(lines.iter().any(|l| l.contains(" Base+ <- ")), "{out}");
    assert!(lines.iter().any(|l| l.contains(" LeafA <- ")), "{out}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_filters_by_directory_and_accepts_a_module_prefix() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let out = subclasses_of(&client, json!({ "class": "mod.Base", "path_prefix": "pkg/drivers/" })).await;
    let mut names = subclass_names(&out);
    names.sort();
    assert_eq!(names, ["LeafA", "LeafB", "Mid"], "{out}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_filters_by_decorator() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    for decorator in ["registry.driver", "driver", "@registry.driver"] {
        let out = subclasses_of(&client, json!({ "class": "Root", "decorator": decorator })).await;
        let mut names = subclass_names(&out);
        names.sort();
        assert_eq!(names, ["Direct", "LeafA", "LeafB", "Other"], "{decorator}: {out}");
    }
    let out = subclasses_of(&client, json!({ "class": "Root", "decorator": "river" })).await;
    assert!(subclass_names(&out).is_empty(), "a partial segment must not match: {out}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_points_at_a_broader_ancestor() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let out = subclasses_of(&client, json!({ "class": "Base", "path_prefix": "pkg/drivers/" })).await;
    assert!(
        out.contains("Base inherits from Root, which has 1 more subclasses under pkg/drivers/ that do not go through Base"),
        "{out}",
    );
    let out = subclasses_of(&client, json!({ "class": "Root" })).await;
    assert!(!out.contains("Note:"), "the root has no broader ancestor: {out}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_pages_with_offset() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let first = subclasses_of(&client, json!({ "class": "Root", "limit": 2 })).await;
    assert!(first.starts_with("6 subclasses of Root"), "{first}");
    assert!(first.contains("(showing 1-2)"), "{first}");
    assert!(first.contains("call again with offset=2"), "{first}");
    let mut all = subclass_names(&first);
    for offset in [2, 4] {
        let page = subclasses_of(&client, json!({ "class": "Root", "limit": 2, "offset": offset })).await;
        all.extend(subclass_names(&page));
    }
    all.sort();
    assert_eq!(all, ["Base", "Direct", "LeafA", "LeafB", "Mid", "Other"]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn find_subclasses_explains_an_empty_result() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let out = subclasses_of(&client, json!({ "class": "Unrelated" })).await;
    assert!(out.starts_with("No subclasses"), "{out}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_lists_classes_by_decorator() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let rows: Vec<Value> = serde_json::from_str(&tool.execute(json!({
        "query": "", "decorator": "registry.driver", "repo": "tree", "version": "v1", "kind": "class",
    })).await.unwrap()).unwrap();
    let mut names = names_from(&rows);
    names.sort();
    assert_eq!(names, ["Direct", "LeafA", "LeafB", "Other"]);
    assert!(rows.iter().all(|r| r["decorators"] == json!(["registry.driver"])), "{rows:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_lists_a_directory_page_by_page() {
    setup!(client, _test_db);
    seed_hierarchy(&client).await;
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    let page = |offset: i64| {
        let tool = &tool;
        async move {
            let out = tool.execute(json!({
                "query": "", "path_prefix": "pkg/drivers/", "repo": "tree", "version": "v1",
                "kind": "class", "limit": 2, "offset": offset,
            })).await.unwrap();
            serde_json::from_str::<Vec<Value>>(&out).unwrap()
        }
    };
    let first = page(0).await;
    assert_eq!(names_from(&first).len(), 2, "{first:?}");
    assert_eq!(first.last().unwrap()["next_offset"], 2, "a further page must be announced: {first:?}");
    let second = page(2).await;
    assert_eq!(names_from(&second).len(), 2, "{second:?}");
    assert!(second.iter().all(|r| r.get("more_results").is_none()), "{second:?}");

    let mut all = names_from(&first);
    all.extend(names_from(&second));
    all.sort();
    assert_eq!(all, ["Direct", "LeafA", "LeafB", "Mid"], "every class under the directory exactly once");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn search_symbols_requires_a_query_or_a_path_prefix() {
    setup!(client, _test_db);
    let tool = SearchSymbolsTool::new(Arc::clone(&client));
    assert!(tool.execute(json!({ "query": "" })).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn all_tools_includes_find_subclasses() {
    setup!(client, _test_db);
    let names: Vec<String> = all_tools(Arc::clone(&client)).iter().map(|t| t.definition().name).collect();
    assert!(names.contains(&"find_subclasses".to_string()), "{names:?}");
}
