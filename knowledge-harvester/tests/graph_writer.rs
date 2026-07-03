use knowledge_harvester::graph::{
    model::{ClassNode, FunctionNode, ImportNode, ParsedFile},
    writer::GraphWriter,
};
use neo4j_testcontainers::{prelude::*, runners::AsyncRunner as _, Neo4j, Neo4jImageExt as _};

macro_rules! setup {
    ($writer:ident, $container:ident) => {
        let $container = Neo4j::default().start().await;
        let uri = $container.image().bolt_uri_ipv4();
        let user = $container.image().user().unwrap_or("neo4j");
        let pass = $container.image().password().unwrap_or("neo");
        let $writer = GraphWriter::new(&uri, user, pass).await.unwrap();
        $writer.ensure_indexes().await.unwrap();
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
#[ignore = "requires Docker"]
async fn upsert_version_not_yet_ingested() {
    setup!(writer, container);
    writer.upsert_version("myrepo", "v1.0", 1_000_000, false).await.unwrap();
    assert!(!writer.is_ingested("myrepo", "v1.0").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn upsert_version_is_idempotent() {
    setup!(writer, container);
    writer.upsert_version("repo", "v1.0", 1_000, false).await.unwrap();
    writer.upsert_version("repo", "v1.0", 1_000, false).await.unwrap();
    assert!(!writer.is_ingested("repo", "v1.0").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn is_ingested_false_before_write_version() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    assert!(!writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn is_ingested_true_after_write_version() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn is_ingested_false_for_unknown_repo() {
    setup!(writer, container);
    assert!(!writer.is_ingested("nonexistent", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn ingested_versions_empty_before_any_ingestion() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    let versions = writer.ingested_versions("r").await.unwrap();
    assert!(versions.is_empty());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn ingested_versions_lists_only_completed_versions() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 1_000, false).await.unwrap();
    writer.upsert_version("r", "v2", 2_000, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();

    let versions = writer.ingested_versions("r").await.unwrap();
    assert_eq!(versions, vec!["v1".to_string()]);
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn write_version_with_no_files_marks_ingested() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
async fn write_version_with_files_marks_ingested() {
    setup!(writer, container);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    let file = make_file("r", "v1", "src/lib.rs");
    writer.write_version("r", "v1", &[file]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires Docker"]
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
#[ignore = "requires Docker"]
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
