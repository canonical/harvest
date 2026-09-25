use knowledge_harvester::graph::{
    model::{ClassNode, FunctionNode, ImportNode, ParsedFile},
    writer::GraphWriter,
};
use harvest_db::test_support::TestDb;
use knowledge_harvester::graph::model::CallRef;
use serde_json::json;

macro_rules! setup {
    ($writer:ident, $test_db:ident) => {
        let $test_db = TestDb::new().await;
        let $writer = GraphWriter::new($test_db.db.clone());
    };
}

fn make_file(repo: &str, version: &str, path: &str) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        functions: vec![
            FunctionNode {
                repo: repo.to_string(),
                version: version.to_string(),
                file: path.to_string(),
                name: "alpha".to_string(),
                kind: "function".to_string(),
                signature: "fn alpha()".to_string(),
                start_line: 1,
                end_line: 3,
                source: "fn alpha() {\n    beta();\n}".to_string(),
                impl_type: None,
                calls: vec![],
            },
            FunctionNode {
                repo: repo.to_string(),
                version: version.to_string(),
                file: path.to_string(),
                name: "beta".to_string(),
                kind: "function".to_string(),
                signature: "fn beta()".to_string(),
                start_line: 5,
                end_line: 6,
                source: "fn beta() {}".to_string(),
                impl_type: None,
                calls: vec![],
            },
        ],
        classes: vec![ClassNode {
            repo: repo.to_string(),
            version: version.to_string(),
            file: path.to_string(),
            name: "MyStruct".to_string(),
            kind: "struct".to_string(),
            start_line: 8,
            end_line: 10,
            source: "struct MyStruct { x: i32 }".to_string(),
            bases: vec![],
            traits: vec![],
            embeds: vec![],
            uses: vec![],
        }],
        imports: vec![ImportNode {
            repo: repo.to_string(),
            version: version.to_string(),
            file: path.to_string(),
            target: "std::collections::HashMap".to_string(),
            line: 1,
        }],
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upsert_version_not_yet_ingested() {
    setup!(writer, container);
    writer.upsert_version("myrepo", "v1.0", 1_000_000, false).await.unwrap();
    assert!(!writer.is_ingested("myrepo", "v1.0").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upsert_version_is_idempotent() {
    setup!(writer, container);
    writer.upsert_version("repo", "v1.0", 1_000, false).await.unwrap();
    writer.upsert_version("repo", "v1.0", 1_000, false).await.unwrap();
    assert!(!writer.is_ingested("repo", "v1.0").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn is_ingested_false_before_write_version() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    assert!(!writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn is_ingested_true_after_write_version() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn is_ingested_false_for_unknown_repo() {
    setup!(writer, container);
    assert!(!writer.is_ingested("nonexistent", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn ingested_versions_empty_before_any_ingestion() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    let versions = writer.ingested_versions("r").await.unwrap();
    assert!(versions.is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn ingested_versions_lists_only_completed_versions() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 1_000, false).await.unwrap();
    writer.upsert_version("r", "v2", 2_000, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();

    let versions = writer.ingested_versions("r").await.unwrap();
    assert_eq!(versions, vec!["v1".to_string()]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn write_version_with_no_files_marks_ingested() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn write_version_with_files_marks_ingested() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    let file = make_file("r", "v1", "src/lib.rs");
    writer.write_version("r", "v1", &[file]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn write_version_is_idempotent() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    let file = make_file("r", "v1", "src/lib.rs");
    writer.write_version("r", "v1", &[file.clone()]).await.unwrap();
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[file]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn two_versions_are_tracked_independently() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 1_000, false).await.unwrap();
    writer.upsert_version("r", "v2", 2_000, false).await.unwrap();

    writer.write_version("r", "v1", &[make_file("r", "v1", "src/lib.rs")]).await.unwrap();
    writer.write_version("r", "v2", &[make_file("r", "v2", "src/lib.rs")]).await.unwrap();

    let versions = writer.ingested_versions("r").await.unwrap();
    assert_eq!(versions.len(), 2);
    assert!(versions.contains(&"v1".to_string()));
    assert!(versions.contains(&"v2".to_string()));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn write_version_stores_symbols_calls_and_class_links() {
    setup!(writer, test_db);
    let mut file = make_file("r", "v1", "src/lib.rs");
    file.functions[0].calls = vec![CallRef { callee: "beta".into(), line: 2 }, CallRef { callee: "missing".into(), line: 2 }];
    let mut child = file.classes[0].clone();
    child.name = "Child".into();
    child.bases = vec!["MyStruct".into()];
    child.traits = vec!["Unknown".into()];
    file.classes.push(child);
    let mut duplicate = file.functions[1].clone();
    duplicate.start_line = 40;
    file.functions.push(duplicate);

    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[file.clone()]).await.unwrap();
    writer.write_version("r", "v1", &[file]).await.unwrap();

    let db = &test_db.db;
    let symbols = db.query(
        "SELECT label, name, start_line FROM code_symbols WHERE repo = 'r' AND version = 'v1' ORDER BY label, name",
        json!({}),
    ).await.unwrap();
    let names: Vec<_> = symbols.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Child", "MyStruct", "alpha", "beta"]);
    assert_eq!(symbols[3]["start_line"], 40, "the last same-named function wins");

    let edges = db.query(
        "SELECT relation, src_name, dst_name, line FROM code_edges ORDER BY relation",
        json!({}),
    ).await.unwrap();
    assert_eq!(edges, vec![
        json!({ "relation": "CALLS", "src_name": "alpha", "dst_name": "beta", "line": 2 }),
        json!({ "relation": "INHERITS", "src_name": "Child", "dst_name": "MyStruct", "line": null }),
    ]);

    let imports = db.query("SELECT target, line FROM code_imports", json!({})).await.unwrap();
    assert_eq!(imports, vec![json!({ "target": "std::collections::HashMap", "line": 1 })]);
}
