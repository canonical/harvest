use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use harvest_db::test_support::TestDb;
use knowledge_server::cluster::{bus::ClusterBus, node::ClusterNode, singleton};
use knowledge_server::config::ClusterConfig;
use serde_json::{json, Value};

fn fast_config(name: &str) -> ClusterConfig {
    ClusterConfig {
        node_name: Some(name.to_string()),
        internal_url: Some(format!("http://{name}.internal:8081")),
        heartbeat_interval_ms: 200,
        node_timeout_ms: 1_500,
        bus_batch_window_ms: 10,
        ..ClusterConfig::default()
    }
}

async fn recv_json(rx: &mut tokio::sync::broadcast::Receiver<String>) -> Value {
    let text = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("timed out").expect("closed");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn node_ids_are_unique_per_start_and_keep_the_configured_name() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterNode::new(Arc::clone(&db), &fast_config("unit-a"));
    let b = ClusterNode::new(Arc::clone(&db), &fast_config("unit-a"));
    assert_ne!(a.node_id(), b.node_id());
    assert!(a.node_id().starts_with("unit-a-"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn heartbeating_nodes_are_alive_and_silent_nodes_expire() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterNode::new(Arc::clone(&db), &fast_config("a"));
    let b = ClusterNode::new(Arc::clone(&db), &fast_config("b"));
    a.heartbeat().await.unwrap();
    b.heartbeat().await.unwrap();
    assert!(a.is_alive(b.node_id()).await.unwrap());
    assert_eq!(a.alive_nodes().await.unwrap().len(), 2);
    let keeper = a.spawn_heartbeat();
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(a.is_alive(a.node_id()).await.unwrap());
    assert!(!a.is_alive(b.node_id()).await.unwrap());
    keeper.abort();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn internal_url_lookup_returns_the_advertised_address_of_live_nodes_only() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterNode::new(Arc::clone(&db), &fast_config("a"));
    let b = ClusterNode::new(Arc::clone(&db), &fast_config("b"));
    b.heartbeat().await.unwrap();
    assert_eq!(a.internal_url_of(b.node_id()).await.unwrap().as_deref(), Some("http://b.internal:8081"));
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    assert_eq!(a.internal_url_of(b.node_id()).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn draining_is_recorded_and_deregistering_removes_the_node() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterNode::new(Arc::clone(&db), &fast_config("a"));
    a.heartbeat().await.unwrap();
    assert!(!a.is_draining());
    a.set_draining().await.unwrap();
    assert!(a.is_draining());
    let rows = db.query("SELECT draining FROM cluster_nodes WHERE node_id = $id", json!({ "id": a.node_id() })).await.unwrap();
    assert_eq!(rows[0]["draining"], true);
    a.deregister().await.unwrap();
    assert!(!a.is_alive(a.node_id()).await.unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn bus_delivers_across_nodes_exactly_once() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterBus::start(Arc::clone(&db), "node-a".into(), Duration::from_millis(10));
    let b = ClusterBus::start(Arc::clone(&db), "node-b".into(), Duration::from_millis(10));
    a.wait_until_listening().await;
    b.wait_until_listening().await;
    let mut on_a = a.subscribe("project:p1");
    let mut on_b = b.subscribe("project:p1");
    a.publish("project:p1", json!({ "type": "text_delta", "text": "hi" }).to_string());
    assert_eq!(recv_json(&mut on_b).await["text"], "hi");
    assert_eq!(recv_json(&mut on_a).await["text"], "hi");
    a.publish("project:p1", json!({ "type": "marker" }).to_string());
    assert_eq!(recv_json(&mut on_a).await["type"], "marker");
    assert_eq!(recv_json(&mut on_b).await["type"], "marker");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn bus_preserves_publish_order_for_a_burst_of_events() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterBus::start(Arc::clone(&db), "node-a".into(), Duration::from_millis(20));
    let b = ClusterBus::start(Arc::clone(&db), "node-b".into(), Duration::from_millis(20));
    a.wait_until_listening().await;
    b.wait_until_listening().await;
    let mut on_b = b.subscribe("project:burst");
    for i in 0..500 {
        a.publish("project:burst", json!({ "i": i, "pad": "x".repeat(40) }).to_string());
    }
    for i in 0..500 {
        assert_eq!(recv_json(&mut on_b).await["i"], i);
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn bus_carries_payloads_larger_than_the_notify_limit() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterBus::start(Arc::clone(&db), "node-a".into(), Duration::from_millis(10));
    let b = ClusterBus::start(Arc::clone(&db), "node-b".into(), Duration::from_millis(10));
    a.wait_until_listening().await;
    b.wait_until_listening().await;
    let mut on_b = b.subscribe("project:big");
    let big = "y".repeat(50_000);
    a.publish("project:big", json!({ "type": "tool_result", "preview": big }).to_string());
    a.publish("project:big", json!({ "type": "after" }).to_string());
    assert_eq!(recv_json(&mut on_b).await["preview"].as_str().unwrap().len(), 50_000);
    assert_eq!(recv_json(&mut on_b).await["type"], "after");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn bus_tells_subscribers_to_resync_after_losing_its_listener_and_keeps_delivering() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ClusterBus::start(Arc::clone(&db), "node-a".into(), Duration::from_millis(10));
    let b = ClusterBus::start(Arc::clone(&db), "node-b".into(), Duration::from_millis(10));
    a.wait_until_listening().await;
    b.wait_until_listening().await;
    let mut on_b = b.subscribe("project:p");
    let pid = b.listener_pid().await.expect("listener pid");
    let admin = db.dedicated_client().await.unwrap();
    admin.execute("SELECT pg_terminate_backend($1)", &[&pid]).await.unwrap();
    assert_eq!(recv_json(&mut on_b).await["type"], "resync");
    b.wait_until_listening().await;
    a.publish("project:p", json!({ "type": "after-reconnect" }).to_string());
    assert_eq!(recv_json(&mut on_b).await["type"], "after-reconnect");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn singleton_jobs_run_on_one_node_at_a_time() {
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let runs = Arc::new(AtomicUsize::new(0));
    let job = |db: Arc<harvest_db::Db>, runs: Arc<AtomicUsize>| async move {
        singleton::run_exclusive(&db, "test-job", || async {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(400)).await;
        }).await.unwrap()
    };
    let (first, second) = tokio::join!(job(Arc::clone(&db), Arc::clone(&runs)), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        job(Arc::clone(&db), Arc::clone(&runs)).await
    });
    assert!(first);
    assert!(!second);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
    assert!(job(Arc::clone(&db), Arc::clone(&runs)).await);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn only_one_node_can_ingest_a_repository_at_a_time() {
    use knowledge_server::ingestion::{IngestionJobs, IngestionStatus};
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let node_a = ClusterNode::new(Arc::clone(&db), &fast_config("a"));
    let node_b = ClusterNode::new(Arc::clone(&db), &fast_config("b"));
    node_a.heartbeat().await.unwrap();
    node_b.heartbeat().await.unwrap();
    let on_a = IngestionJobs::new(Arc::clone(&db), Arc::clone(&node_a));
    let on_b = IngestionJobs::new(Arc::clone(&db), Arc::clone(&node_b));

    let job = on_a.try_start("repo", "ingest").await.unwrap().expect("first start wins");
    assert!(on_b.try_start("repo", "ingest").await.unwrap().is_none());
    assert!(on_b.is_active("repo").await.unwrap());

    on_a.append(&job, &["cloning".into(), "parsing".into()]).await.unwrap();
    let lines = on_b.progress_since(&job, 0).await.unwrap();
    assert_eq!(lines.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), vec!["cloning", "parsing"]);

    on_a.finish(&job, Ok(vec!["v1".into()])).await.unwrap();
    assert_eq!(on_b.status(&job).await.unwrap(), Some(IngestionStatus::Completed { versions: vec!["v1".into()] }));
    assert!(on_b.try_start("repo", "resync").await.unwrap().is_some());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn an_ingestion_owned_by_a_lost_node_is_marked_failed() {
    use knowledge_server::ingestion::{IngestionJobs, IngestionStatus};
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let node_a = ClusterNode::new(Arc::clone(&db), &fast_config("a"));
    let node_b = ClusterNode::new(Arc::clone(&db), &fast_config("b"));
    node_a.heartbeat().await.unwrap();
    let on_a = IngestionJobs::new(Arc::clone(&db), Arc::clone(&node_a));
    let on_b = IngestionJobs::new(Arc::clone(&db), Arc::clone(&node_b));
    let job = on_a.try_start("repo", "ingest").await.unwrap().unwrap();
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    node_b.heartbeat().await.unwrap();
    assert!(!on_b.is_active("repo").await.unwrap());
    match on_b.status(&job).await.unwrap() {
        Some(IngestionStatus::Failed { error }) => assert!(error.contains("interrupted")),
        other => panic!("expected failed, got {other:?}"),
    }
    assert!(on_b.try_start("repo", "ingest").await.unwrap().is_some(), "another node may retry");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn design_pdf_generation_is_leased_to_one_node() {
    use knowledge_server::deployments::design_cache::{release_generation, try_acquire_generation};
    let t = TestDb::new().await;
    let db = &t.db;
    db.execute("INSERT INTO groups (id, name, description, created_at) VALUES ('g', 'g', '', now())", json!({})).await.unwrap();
    db.execute("INSERT INTO projects (id, group_id, name, created_at) VALUES ('p', 'g', 'p', now())", json!({})).await.unwrap();
    db.execute(
        "INSERT INTO deployments (id, project_id, name, environment_description, infra_state, created_at, updated_at)
         VALUES ('d', 'p', 'd', '', 'none', now(), now())",
        json!({}),
    ).await.unwrap();
    assert!(try_acquire_generation(db, "d").await);
    assert!(!try_acquire_generation(db, "d").await);
    release_generation(db, "d").await;
    assert!(try_acquire_generation(db, "d").await);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn paused_confirmations_survive_and_resolve_across_nodes() {
    use knowledge_server::projects::live::ProjectLive;
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let a = ProjectLive::standalone(Arc::clone(&db));
    let b = ProjectLive::standalone(Arc::clone(&db));
    a.save_paused("p", "c", &json!({ "pending": [1, 2], "resolved": 0 })).await.unwrap();

    let first = b.update_paused("p", "c", |payload| {
        payload["resolved"] = json!(1);
        false
    }).await.unwrap().unwrap();
    assert!(!first.1);

    let second = a.update_paused("p", "c", |payload| {
        assert_eq!(payload["resolved"], 1, "updates from another node are visible");
        payload["resolved"] = json!(2);
        true
    }).await.unwrap().unwrap();
    assert!(second.1);
    assert!(b.update_paused("p", "c", |_| true).await.unwrap().is_none(), "a resumed confirmation is consumed once");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn a_conversation_lock_held_by_a_lost_node_can_be_taken_over() {
    use knowledge_server::projects::live::ProjectLive;
    let t = TestDb::new().await;
    let db = Arc::new(t.db.clone());
    let node_a = ClusterNode::new(Arc::clone(&db), &fast_config("a"));
    let node_b = ClusterNode::new(Arc::clone(&db), &fast_config("b"));
    node_a.heartbeat().await.unwrap();
    node_b.heartbeat().await.unwrap();
    let a = ProjectLive::new(Arc::clone(&db), Arc::clone(&node_a), ClusterBus::local("a"), None, knowledge_server::cluster::Shutdown::new());
    let b = ProjectLive::new(Arc::clone(&db), Arc::clone(&node_b), ClusterBus::local("b"), None, knowledge_server::cluster::Shutdown::new());

    assert!(a.try_lock("p", "c", "t1", "ann", "ann", "q", &[]).await.unwrap());
    assert!(!b.try_lock("p", "c", "t2", "bob", "bob", "q", &[]).await.unwrap());
    assert_eq!(b.locks("p").await.unwrap(), vec![("c".to_string(), "ann".to_string())]);

    tokio::time::sleep(Duration::from_millis(2_000)).await;
    node_b.heartbeat().await.unwrap();
    let mut events = b.subscribe("p");
    assert_eq!(b.reap_dead_nodes().await.unwrap(), 1);
    assert_eq!(serde_json::from_str::<Value>(&events.recv().await.unwrap()).unwrap()["type"], "turn_aborted");
    assert!(b.try_lock("p", "c", "t2", "bob", "bob", "q", &[]).await.unwrap());
}
