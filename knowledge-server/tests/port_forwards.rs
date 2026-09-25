use std::sync::Arc;

use serde_json::json;

use knowledge_server::machines::port_forwards::{self, PortForwardError};
use harvest_db::Db;

#[cfg(test)]
mod db_tests {
    use super::*;
    use harvest_db::test_support::TestDb;

    async fn start_db() -> (Arc<Db>, TestDb) {
        let test_db = TestDb::new().await;
        (Arc::new(test_db.db.clone()), test_db)
    }

    async fn seed_project(db: &Db) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        db.execute(
            "INSERT INTO groups (id, name) VALUES ($gid, 'g1') ON CONFLICT DO NOTHING",
            json!({ "gid": "g1" }),
        ).await.unwrap();
        db.execute(
            "INSERT INTO projects (id, name, group_id, created_by, created_at)
             VALUES ($id, 'test', 'g1', 'u1', '2026-01-01T00:00:00+00:00')",
            json!({ "id": id }),
        ).await.unwrap();
        db.execute(
            "INSERT INTO machines (id, project_id, hostname, created_at)
             SELECT agent, $id, agent, now() FROM unnest($agents::text[]) AS agent",
            json!({ "id": id, "agents": ["agent-1", "agent-2"] }),
        ).await.unwrap();
        id
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn create_and_list_round_trip() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let created = port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        assert_eq!(created.port, 8080);
        assert_eq!(created.route_name, "app");

        let listed = port_forwards::list_for_agent(&db, &project_id, "agent-1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], created);
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn create_rejects_duplicate_route_name_for_same_agent() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let err = port_forwards::create(&db, &project_id, "agent-1", 9090, "app").await.unwrap_err();
        assert!(matches!(err, PortForwardError::DuplicateRouteName));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn create_allows_same_route_name_on_different_agents() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let ok = port_forwards::create(&db, &project_id, "agent-2", 8080, "app").await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn create_rejects_invalid_port_before_hitting_db() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let err = port_forwards::validate_port(70000);
        assert!(err.is_err());
        let _ = project_id;
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn list_for_agent_isolates_by_agent_id() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        port_forwards::create(&db, &project_id, "agent-2", 9090, "other").await.unwrap();

        let a1 = port_forwards::list_for_agent(&db, &project_id, "agent-1").await.unwrap();
        assert_eq!(a1.len(), 1);
        assert_eq!(a1[0].agent_id, "agent-1");

        let a2 = port_forwards::list_for_agent(&db, &project_id, "agent-2").await.unwrap();
        assert_eq!(a2.len(), 1);
        assert_eq!(a2[0].agent_id, "agent-2");
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn update_changes_port_and_route_name() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let created = port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let updated = port_forwards::update(
            &db, &project_id, "agent-1", &created.id,
            Some(9090), Some("app2".to_string()),
        ).await.unwrap();

        assert_eq!(updated.port, 9090);
        assert_eq!(updated.route_name, "app2");
        assert_eq!(updated.id, created.id);
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn update_rejects_rename_to_taken_route_name() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let second = port_forwards::create(&db, &project_id, "agent-1", 9090, "other").await.unwrap();

        let err = port_forwards::update(
            &db, &project_id, "agent-1", &second.id,
            None, Some("app".to_string()),
        ).await.unwrap_err();
        assert!(matches!(err, PortForwardError::DuplicateRouteName));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn update_allows_renaming_to_its_own_current_name() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let created = port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let updated = port_forwards::update(
            &db, &project_id, "agent-1", &created.id,
            Some(8081), Some("app".to_string()),
        ).await.unwrap();
        assert_eq!(updated.port, 8081);
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn update_unknown_id_returns_not_found() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let err = port_forwards::update(
            &db, &project_id, "agent-1", "nonexistent",
            Some(8081), None,
        ).await.unwrap_err();
        assert!(matches!(err, PortForwardError::NotFound));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn update_cross_agent_id_returns_not_found() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let created = port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let err = port_forwards::update(
            &db, &project_id, "agent-2", &created.id,
            Some(9090), None,
        ).await.unwrap_err();
        assert!(matches!(err, PortForwardError::NotFound));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn delete_removes_forward() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let created = port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        port_forwards::delete(&db, &project_id, "agent-1", &created.id).await.unwrap();

        let listed = port_forwards::list_for_agent(&db, &project_id, "agent-1").await.unwrap();
        assert!(listed.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn delete_unknown_id_returns_not_found() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let err = port_forwards::delete(&db, &project_id, "agent-1", "nonexistent").await.unwrap_err();
        assert!(matches!(err, PortForwardError::NotFound));
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn get_by_route_finds_created_forward() {
        let (db, _test_db) = start_db().await;
        let project_id = seed_project(&db).await;

        let created = port_forwards::create(&db, &project_id, "agent-1", 8080, "app").await.unwrap();
        let found = port_forwards::get_by_route(&db, "agent-1", "app").await.unwrap().unwrap();
        assert_eq!(found.id, created.id);
    }

    #[tokio::test]
    #[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
    async fn get_by_route_returns_none_for_unknown_route() {
        let (db, _test_db) = start_db().await;
        assert!(port_forwards::get_by_route(&db, "agent-1", "nope").await.unwrap().is_none());
    }
}
