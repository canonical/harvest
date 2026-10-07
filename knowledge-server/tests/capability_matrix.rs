
use std::sync::Arc;

use harvest_db::test_support::TestDb;
use knowledge_server::agent::graph_tools::GetCapabilityMatrixTool;
use knowledge_server::agent::tool::Tool;
use serde_json::{json, Value};

const REPO: &str = "capability_fixture";
const VERSION: &str = "v1";

struct Seeded {
    _db: TestDb,
    db: Arc<harvest_db::Db>,
}

async fn seed() -> Seeded {
    let test_db = TestDb::new().await;
    let db = Arc::new(test_db.db.clone());

    let version_id: i64 = db
        .query(
            "WITH r AS (
                 INSERT INTO repositories (name) VALUES ($repo)
                 ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
                 RETURNING id
             )
             INSERT INTO versions (repository_id, tag) SELECT id, $tag FROM r RETURNING id",
            json!({ "repo": REPO, "tag": VERSION }),
        )
        .await
        .unwrap()
        .first()
        .and_then(|r| r["id"].as_i64())
        .expect("version id");

    let files = ["pkg/base.py", "pkg/naive.py", "pkg/active.py", "pkg/stale.py", "pkg/external.py"];
    let mut file_ids = Vec::new();
    for path in files {
        let id: i64 = db
            .query(
                "INSERT INTO files (version_id, path, language) VALUES ($vid, $path, 'python') RETURNING id",
                json!({ "vid": version_id, "path": path }),
            )
            .await
            .unwrap()
            .first()
            .and_then(|r| r["id"].as_i64())
            .expect("file id");
        file_ids.push(id);
    }

    let base_source = "class RootDriver(object):\n    SUPPORTS_ACTIVE_ACTIVE = False\n";
    let naive_source = "class NaiveDriver(RootDriver):\n    VERSION = '1'\n";
    let active_source =
        "class ActiveDriver(RootDriver):\n    SUPPORTS_ACTIVE_ACTIVE = True\n    OTHER_FLAG = 1\n";

    let classes = [
        ("RootDriver", file_ids[0], base_source.to_string(), Vec::<String>::new()),
        ("NaiveDriver", file_ids[1], naive_source.to_string(), vec!["RootDriver".to_string()]),
        ("ActiveDriver", file_ids[2], active_source.to_string(), vec!["RootDriver".to_string()]),
        ("StaleDriver", file_ids[3], "class StaleDriver(base.RootDriver):\n    pass\n".to_string(), Vec::new()),
        ("ExternalDriver", file_ids[4], "class ExternalDriver(lib.RemoteDriver):\n    pass\n".to_string(), vec!["RemoteDriver".to_string()]),
    ];

    for (name, file_id, source, bases) in classes {
        db.execute(
            "INSERT INTO symbols (file_id, version_id, label, name, kind, start_line, end_line, source, bases)
             VALUES ($fid, $vid, 'Class', $name, 'class', 1, 10, $source, $bases)",
            json!({ "fid": file_id, "vid": version_id, "name": name, "source": source, "bases": bases }),
        )
        .await
        .unwrap();
    }

    Seeded { _db: test_db, db }
}

async fn run_matrix(db: &Arc<harvest_db::Db>, classes: &[&str], capability: &str) -> Vec<Value> {
    let tool = GetCapabilityMatrixTool(Arc::clone(db));
    let out = tool
        .execute(json!({
            "repo": REPO,
            "version": VERSION,
            "classes": classes,
            "capability": capability,
        }))
        .await
        .expect("tool should succeed");
    serde_json::from_str(&out).expect("tool output should be JSON rows")
}

fn row_for<'a>(rows: &'a [Value], class: &str) -> &'a Value {
    rows.iter()
        .find(|r| r["class"] == class)
        .unwrap_or_else(|| panic!("no row for {class} in {rows:?}"))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn resolves_inherited_capability_value_from_ancestor() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["NaiveDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;

    let row = row_for(&rows, "NaiveDriver");
    assert_eq!(row["value"], "False");
    assert_eq!(row["declared_by"], "RootDriver");
    assert_eq!(row["declared_in"], "pkg/base.py");
    assert_eq!(row["inherited"], true);
    assert_eq!(row["inherited_from_depth"], 1);
    assert_eq!(row["ambiguous"], false);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn own_capability_value_wins_over_inherited() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["ActiveDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;

    let row = row_for(&rows, "ActiveDriver");
    assert_eq!(row["value"], "True");
    assert_eq!(row["declared_by"], "ActiveDriver");
    assert_eq!(row["declared_in"], "pkg/active.py");
    assert_eq!(row["inherited"], false);
    assert_eq!(row["inherited_from_depth"], 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn matrix_splits_siblings_that_disagree() {
    let seeded = seed().await;
    let rows = run_matrix(
        &seeded.db,
        &["ActiveDriver", "NaiveDriver", "RootDriver"],
        "SUPPORTS_ACTIVE_ACTIVE",
    )
    .await;

    assert_eq!(rows.iter().filter(|r| r["class"].is_string()).count(), 3);
    assert_eq!(row_for(&rows, "ActiveDriver")["value"], "True");
    assert_eq!(row_for(&rows, "NaiveDriver")["value"], "False");
    assert_eq!(row_for(&rows, "RootDriver")["value"], "False");
    assert_eq!(row_for(&rows, "RootDriver")["inherited"], false);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn reports_undeclared_capability_rather_than_guessing() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["ActiveDriver"], "NO_SUCH_FLAG").await;

    let row = row_for(&rows, "ActiveDriver");
    assert_eq!(row["value"], Value::Null);
    assert_eq!(row["declared_by"], Value::Null);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn reports_missing_classes_instead_of_silence() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["ActiveDriver", "NoSuchDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;

    let warning = rows
        .iter()
        .find(|r| r["warning"] == "classes_not_found")
        .expect("missing class should be reported");
    assert_eq!(warning["classes"], json!(["NoSuchDriver"]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn matrix_supports_multiple_capabilities() {
    let seeded = seed().await;
    let tool = GetCapabilityMatrixTool(Arc::clone(&seeded.db));
    let out = tool
        .execute(json!({
            "repo": REPO,
            "version": VERSION,
            "classes": ["ActiveDriver"],
            "capability": ["SUPPORTS_ACTIVE_ACTIVE", "OTHER_FLAG"],
        }))
        .await
        .expect("tool should succeed");
    let rows: Vec<Value> = serde_json::from_str(&out).unwrap();

    assert_eq!(row_for(&rows, "ActiveDriver")["capability"], "SUPPORTS_ACTIVE_ACTIVE");
    let other = rows
        .iter()
        .find(|r| r["capability"] == "OTHER_FLAG")
        .expect("second capability should be reported");
    assert_eq!(other["value"], "1");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn rejects_capability_names_that_are_not_identifiers() {
    let seeded = seed().await;
    let tool = GetCapabilityMatrixTool(Arc::clone(&seeded.db));
    let result = tool
        .execute(json!({
            "repo": REPO,
            "version": VERSION,
            "classes": ["ActiveDriver"],
            "capability": "SUPPORTS_X = 1; DROP TABLE symbols",
        }))
        .await;
    assert!(result.is_err(), "non-identifier capability must be rejected");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn cycles_in_bases_terminate() {
    let test_db = TestDb::new().await;
    let db = Arc::new(test_db.db.clone());
    let version_id: i64 = db
        .query(
            "WITH r AS (
                 INSERT INTO repositories (name) VALUES ('cycle_fixture')
                 ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name RETURNING id
             )
             INSERT INTO versions (repository_id, tag) SELECT id, 'v1' FROM r RETURNING id",
            json!({}),
        )
        .await
        .unwrap()
        .first()
        .and_then(|r| r["id"].as_i64())
        .unwrap();
    let file_id: i64 = db
        .query(
            "INSERT INTO files (version_id, path, language) VALUES ($vid, 'p.py', 'python') RETURNING id",
            json!({ "vid": version_id }),
        )
        .await
        .unwrap()
        .first()
        .and_then(|r| r["id"].as_i64())
        .unwrap();

    for (name, bases) in [("A", vec!["B"]), ("B", vec!["A"])] {
        db.execute(
            "INSERT INTO symbols (file_id, version_id, label, name, kind, start_line, end_line, source, bases)
             VALUES ($fid, $vid, 'Class', $name, 'class', 1, 5, $src, $bases)",
            json!({ "fid": file_id, "vid": version_id, "name": name, "src": format!("class {name}:\n    pass\n"), "bases": bases }),
        )
        .await
        .unwrap();
    }

    let tool = GetCapabilityMatrixTool(Arc::clone(&db));
    let out = tool
        .execute(json!({
            "repo": "cycle_fixture", "version": "v1",
            "classes": ["A"], "capability": "NOPE"
        }))
        .await
        .expect("cyclic bases must not hang or error");
    let rows: Vec<Value> = serde_json::from_str(&out).unwrap();
    assert_eq!(row_for(&rows, "A")["value"], Value::Null);
    assert!(rows.iter().all(|r| r["warning"].is_null()), "a cycle is not a missing parent: {rows:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn warns_when_declared_bases_were_not_recorded() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["StaleDriver", "NaiveDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;

    assert_eq!(row_for(&rows, "StaleDriver")["value"], Value::Null);
    let warning = rows
        .iter()
        .find(|r| r["warning"] == "bases_not_recorded")
        .expect("a class whose source names parents but whose index has none must be flagged");
    assert_eq!(warning["classes"], json!(["StaleDriver"]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn warns_when_a_parent_is_not_in_the_repository() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["ExternalDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;

    let warning = rows
        .iter()
        .find(|r| r["warning"] == "parents_not_indexed")
        .expect("an unresolvable parent must be reported");
    assert_eq!(warning["parents"], json!(["RemoteDriver"]));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn complete_hierarchies_carry_no_inheritance_warning() {
    let seeded = seed().await;
    let rows = run_matrix(&seeded.db, &["ActiveDriver", "NaiveDriver", "RootDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;
    assert!(rows.iter().all(|r| r["warning"].is_null()), "unexpected warning in {rows:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn no_inheritance_warning_when_the_class_declares_the_value_itself() {
    let seeded = seed().await;
    seeded.db.execute(
        "UPDATE symbols SET source = $src WHERE name = 'StaleDriver'",
        json!({ "src": "class StaleDriver(base.RootDriver):\n    SUPPORTS_ACTIVE_ACTIVE = True\n" }),
    ).await.unwrap();
    let rows = run_matrix(&seeded.db, &["StaleDriver"], "SUPPORTS_ACTIVE_ACTIVE").await;
    assert_eq!(row_for(&rows, "StaleDriver")["value"], "True");
    assert!(rows.iter().all(|r| r["warning"].is_null()), "the value does not depend on the missing bases: {rows:?}");
}
