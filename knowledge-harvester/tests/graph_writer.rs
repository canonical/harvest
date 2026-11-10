use harvest_db::test_support::TestDb;
use knowledge_harvester::graph::model::CallRef;
use knowledge_harvester::graph::{
    model::{ClassNode, FunctionNode, ImportNode, ParsedFile},
    writer::{GraphWriter, IngestionStatus},
};
use knowledge_harvester::parser::PARSER_VERSION;
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
                docstring: None,
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
                docstring: None,
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
            docstring: None,
            decorators: vec![],
        }],
        imports: vec![ImportNode {
            repo: repo.to_string(),
            version: version.to_string(),
            file: path.to_string(),
            target: "std::collections::HashMap".to_string(),
            line: 1,
        }],
        ..ParsedFile::default()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upsert_version_not_yet_ingested() {
    setup!(writer, container);
    writer
        .upsert_version("myrepo", "v1.0", 1_000_000, false)
        .await
        .unwrap();
    assert!(!writer.is_ingested("myrepo", "v1.0").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn upsert_version_is_idempotent() {
    setup!(writer, container);
    writer
        .upsert_version("repo", "v1.0", 1_000, false)
        .await
        .unwrap();
    writer
        .upsert_version("repo", "v1.0", 1_000, false)
        .await
        .unwrap();
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
    writer
        .upsert_version("r", "v1", 1_000, false)
        .await
        .unwrap();
    writer
        .upsert_version("r", "v2", 2_000, false)
        .await
        .unwrap();
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
    writer
        .write_version("r", "v1", &[file.clone()])
        .await
        .unwrap();
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[file]).await.unwrap();
    assert!(writer.is_ingested("r", "v1").await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn two_versions_are_tracked_independently() {
    setup!(writer, container);
    writer
        .upsert_version("r", "v1", 1_000, false)
        .await
        .unwrap();
    writer
        .upsert_version("r", "v2", 2_000, false)
        .await
        .unwrap();

    writer
        .write_version("r", "v1", &[make_file("r", "v1", "src/lib.rs")])
        .await
        .unwrap();
    writer
        .write_version("r", "v2", &[make_file("r", "v2", "src/lib.rs")])
        .await
        .unwrap();

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
    file.functions[0].calls = vec![
        CallRef {
            callee: "beta".into(),
            line: 2,
        },
        CallRef {
            callee: "missing".into(),
            line: 2,
        },
    ];
    let mut child = file.classes[0].clone();
    child.name = "Child".into();
    child.bases = vec!["MyStruct".into()];
    child.traits = vec!["Unknown".into()];
    file.classes.push(child);
    let mut duplicate = file.functions[1].clone();
    duplicate.start_line = 40;
    file.functions.push(duplicate);

    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer
        .write_version("r", "v1", &[file.clone()])
        .await
        .unwrap();
    writer.write_version("r", "v1", &[file]).await.unwrap();

    let db = &test_db.db;
    let symbols = db.query(
        "SELECT label, name, start_line FROM code_symbols WHERE repo = 'r' AND version = 'v1' ORDER BY label, name",
        json!({}),
    ).await.unwrap();
    let names: Vec<_> = symbols
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Child", "MyStruct", "alpha", "beta"]);
    assert_eq!(
        symbols[3]["start_line"], 40,
        "the last same-named function wins"
    );

    let edges = db
        .query(
            "SELECT relation, src_name, dst_name, line FROM code_edges ORDER BY relation",
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(
        edges,
        vec![
            json!({ "relation": "CALLS", "src_name": "alpha", "dst_name": "beta", "line": 2 }),
            json!({ "relation": "INHERITS", "src_name": "Child", "dst_name": "MyStruct", "line": null }),
        ]
    );

    let imports = db
        .query("SELECT target, line FROM code_imports", json!({}))
        .await
        .unwrap();
    assert_eq!(
        imports,
        vec![json!({ "target": "std::collections::HashMap", "line": 1 })]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn version_ingested_by_an_older_parser_is_stale_until_reingested() {
    setup!(writer, test_db);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();
    assert_eq!(
        writer.ingestion_status("r", "v1").await.unwrap(),
        IngestionStatus::Current
    );

    test_db
        .db
        .execute(
            "UPDATE versions SET parser_version = $pv",
            json!({ "pv": PARSER_VERSION - 1 }),
        )
        .await
        .unwrap();
    assert_eq!(
        writer.ingestion_status("r", "v1").await.unwrap(),
        IngestionStatus::Stale {
            parser_version: PARSER_VERSION - 1
        },
    );
    assert!(
        !writer.is_ingested("r", "v1").await.unwrap(),
        "a stale version must be harvested again"
    );

    writer.write_version("r", "v1", &[]).await.unwrap();
    assert_eq!(
        writer.ingestion_status("r", "v1").await.unwrap(),
        IngestionStatus::Current
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn reingesting_keeps_the_old_data_visible_until_the_new_data_is_written() {
    setup!(writer, test_db);
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    writer.write_version("r", "v1", &[]).await.unwrap();
    writer.upsert_version("r", "v1", 0, false).await.unwrap();
    let rows = test_db
        .db
        .query(
            "SELECT ingested FROM code_versions WHERE repo = 'r' AND tag = 'v1'",
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(rows[0]["ingested"], json!(true));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn python_sources_produce_units_and_typed_edges() {
    use knowledge_harvester::parser::LanguageParser;
    use std::path::Path;

    setup!(writer, test_db);
    let driver = r#"from oslo_config import cfg

volume_opts = [cfg.IntOpt('retry_limit', default=3)]


class VolumeDriver(object):
    def initialize_connection(self, volume):
        raise NotImplementedError()


class LVMVolumeDriver(VolumeDriver):
    def initialize_connection(self, volume):
        if CONF.retry_limit > 3:
            raise exception.VolumeBusy()
        return {}


class VolumeBusy(Exception):
    pass
"#;
    let parser = knowledge_harvester::parser::language::PythonParser;
    let mut parsed = parser.parse(driver, Path::new("cinder/volume/driver.py"), "r", "v1");
    knowledge_harvester::units::add_code_units(&mut parsed);
    let docs = ParsedFile {
        path: "doc/drivers.md".into(),
        language: "text".into(),
        units: knowledge_harvester::units::file_units(
            Path::new("doc/drivers.md"),
            "# Drivers\nSubclass `VolumeDriver` and set `retry_limit`.\n",
        ),
        ..ParsedFile::default()
    };
    writer.upsert_repository("r", "u").await.unwrap();
    writer
        .write_version("r", "v1", &[parsed, docs])
        .await
        .unwrap();

    let db = &test_db.db;
    let methods = db
        .query(
            "SELECT impl_type FROM symbols WHERE name = 'initialize_connection' ORDER BY impl_type",
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(
        methods.len(),
        2,
        "same-named methods of different classes are both stored"
    );
    assert_eq!(methods[0]["impl_type"], "LVMVolumeDriver");

    let edges = db
        .query(
            "SELECT relation, src_name, dst_name FROM code_edges ORDER BY relation, src_name",
            json!({}),
        )
        .await
        .unwrap();
    let has = |relation: &str, src: &str, dst: &str| {
        edges
            .iter()
            .any(|e| e["relation"] == relation && e["src_name"] == src && e["dst_name"] == dst)
    };
    assert!(
        has("INHERITS", "LVMVolumeDriver", "VolumeDriver"),
        "{edges:?}"
    );
    assert!(
        has(
            "OVERRIDES",
            "initialize_connection",
            "initialize_connection"
        ),
        "{edges:?}"
    );
    assert!(
        has("RAISES", "initialize_connection", "VolumeBusy"),
        "{edges:?}"
    );

    let units = db
        .query(
            "SELECT kind, name FROM code_units ORDER BY kind, name",
            json!({}),
        )
        .await
        .unwrap();
    let kinds: Vec<String> = units
        .iter()
        .map(|u| {
            format!(
                "{}:{}",
                u["kind"].as_str().unwrap(),
                u["name"].as_str().unwrap()
            )
        })
        .collect();
    assert!(
        kinds.contains(&"config:retry_limit".to_string()),
        "{kinds:?}"
    );
    assert!(kinds.contains(&"doc:Drivers".to_string()), "{kinds:?}");
    assert!(
        kinds.contains(&"module:cinder/volume/driver.py".to_string()),
        "{kinds:?}"
    );

    let unit_edges = db
        .query(
            "SELECT e.relation, coalesce(u.name, s.name) AS src, coalesce(du.name, ds.name) AS dst
             FROM unit_edges e
             LEFT JOIN code_units u ON u.id = e.src_unit_id
             LEFT JOIN symbols s ON s.id = e.src_symbol_id
             LEFT JOIN code_units du ON du.id = e.dst_unit_id
             LEFT JOIN symbols ds ON ds.id = e.dst_symbol_id",
            json!({}),
        )
        .await
        .unwrap();
    let has_unit = |relation: &str, src: &str, dst: &str| {
        unit_edges
            .iter()
            .any(|e| e["relation"] == relation && e["src"] == src && e["dst"] == dst)
    };
    assert!(
        has_unit("READS_CONFIG", "initialize_connection", "retry_limit"),
        "{unit_edges:?}"
    );
    assert!(
        has_unit("DOCUMENTS", "Drivers", "VolumeDriver"),
        "{unit_edges:?}"
    );
}
