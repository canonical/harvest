use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use harvest_db::test_support::TestDb;
use harvest_search::engine::SearchEngine;
use harvest_search::fallback::PostgresEngine;
use harvest_search::index::corpus::{CorpusSource, Relation, UnitKind};
use harvest_search::index::pg::PgCorpus;
use harvest_search::{Deadline, EvidenceKey, Query, Scope, TierId};

async fn seed(db: &harvest_db::Db) {
    db.batch_execute(
        r#"
        INSERT INTO repositories (id, name) VALUES (1, 'cinder');
        INSERT INTO versions (id, repository_id, tag, ingested) VALUES (1, 1, 'v1', true);
        INSERT INTO files (id, version_id, path, language) VALUES
            (1, 1, 'cinder/volume/driver.py', 'python'),
            (2, 1, 'cinder/volume/drivers/lvm.py', 'python');
        INSERT INTO symbols (id, file_id, version_id, label, name, kind, signature, start_line, end_line, source, impl_type, bases, docstring)
        VALUES
            (10, 1, 1, 'Class', 'VolumeDriver', 'class', 'class VolumeDriver', 1, 9, 'class VolumeDriver:\n    supports_multiattach = False', NULL, '{}', 'Base driver'),
            (11, 1, 1, 'Function', 'initialize_connection', 'method', 'def initialize_connection(self)', 5, 9, 'def initialize_connection(self): pass', 'VolumeDriver', NULL, NULL),
            (20, 2, 1, 'Class', 'LVMVolumeDriver', 'class', 'class LVMVolumeDriver(VolumeDriver)', 1, 30, 'class LVMVolumeDriver(VolumeDriver):\n    supports_multiattach = True', NULL, '{VolumeDriver}', NULL);
        INSERT INTO symbol_edges (src_id, dst_id, relation, line) VALUES (20, 10, 'INHERITS', 1);
        "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn pg_corpus_loads_symbols_edges_and_a_stable_generation() {
    let test_db = TestDb::new().await;
    seed(&test_db.db).await;
    let source = PgCorpus::new(Arc::new(test_db.db.clone()));
    let scope = Scope::new("cinder", "v1");
    let generation = source.generation(&scope).await.unwrap().unwrap();
    assert_eq!(
        source.generation(&scope).await.unwrap().unwrap(),
        generation
    );
    assert!(source
        .generation(&Scope::new("cinder", "nope"))
        .await
        .unwrap()
        .is_none());

    let corpus = source.load(&scope).await.unwrap();
    assert_eq!(corpus.generation, generation);
    assert_eq!(corpus.units.len(), 3);
    let method = corpus
        .units
        .iter()
        .find(|u| u.name == "initialize_connection")
        .unwrap();
    assert_eq!(method.qualified_name, "VolumeDriver.initialize_connection");
    assert_eq!(method.kind(), UnitKind::Method);
    let relations: Vec<Relation> = corpus.edges.iter().map(|e| e.relation).collect();
    assert!(relations.contains(&Relation::Inherits));
    assert!(relations.contains(&Relation::Contains));

    test_db
        .db
        .execute(
            "UPDATE symbols SET source = source WHERE id = 10",
            json!({}),
        )
        .await
        .unwrap();
    test_db
        .db
        .batch_execute("INSERT INTO symbols (file_id, version_id, label, name) VALUES (2, 1, 'Function', 'extra')")
        .await
        .unwrap();
    assert_ne!(
        source.generation(&scope).await.unwrap().unwrap(),
        generation
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn the_postgres_engine_answers_from_sql_when_no_index_is_loaded() {
    let test_db = TestDb::new().await;
    seed(&test_db.db).await;
    let engine = PostgresEngine::new(Arc::new(test_db.db.clone()));
    let result = engine
        .search(
            &Query::new(
                "Where is VolumeDriver.initialize_connection defined?",
                Scope::new("cinder", "v1"),
            ),
            Deadline::after(Duration::from_secs(5)),
        )
        .await
        .unwrap();
    assert_eq!(result.tier_reached, TierId::Fallback);
    assert!(result.evidence.iter().any(|e| e.key
        == EvidenceKey::Symbol {
            file: "cinder/volume/driver.py".into(),
            name: "VolumeDriver.initialize_connection".into()
        }));
}
