//! Kept in its own test binary: these tests flip the process-wide `run_sql` availability flag.

use std::sync::Arc;

use harvest_db::test_support::TestDb;
use knowledge_server::agent::graph_tools::{all_tools, probe_run_sql, run_read_only_sql, set_run_sql_available};
use serde_json::json;

fn tool_names(db: &Arc<harvest_db::Db>) -> Vec<String> {
    all_tools(Arc::clone(db)).iter().map(|t| t.definition().name).collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn run_sql_is_offered_only_when_available() {
    let test_db = TestDb::new().await;
    let db = Arc::new(test_db.db.clone());

    set_run_sql_available(false);
    assert!(!tool_names(&db).contains(&"run_sql".to_string()), "an unusable run_sql must not be offered");

    set_run_sql_available(true);
    assert!(tool_names(&db).contains(&"run_sql".to_string()));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn probe_agrees_with_whether_run_sql_can_execute() {
    let test_db = TestDb::new().await;
    let probed = probe_run_sql(&test_db.db).await;
    let works = run_read_only_sql(&test_db.db, "SELECT 1 AS one", json!({})).await.is_ok();
    assert_eq!(probed, works);
}
