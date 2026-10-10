use std::str::FromStr;
use std::time::Duration;

use harvest_db::test_support::TestDb;
use harvest_db::{Db, DbOptions};
use serde_json::json;

fn admin_url() -> String {
    std::env::var(harvest_db::test_support::TEST_DATABASE_URL_VAR).expect("HARVEST_TEST_DATABASE_URL")
}

fn with_dead_first_host(url: &str) -> String {
    let config = tokio_postgres::Config::from_str(url).unwrap();
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(h)) => h.clone(),
        _ => "127.0.0.1".to_string(),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    let user = config.get_user().unwrap_or("postgres");
    let password = config.get_password().map(|p| String::from_utf8_lossy(p).to_string()).unwrap_or_default();
    let dbname = config.get_dbname().unwrap_or("postgres");
    format!("postgres://{user}:{password}@127.0.0.1:1,{host}:{port}/{dbname}?target_session_attrs=read-write&connect_timeout=2")
}

#[test]
fn default_options_use_a_pool_of_sixteen_without_tls() {
    let options = DbOptions::new("postgres://u:p@localhost:5432/db");
    assert_eq!(options.pool_size, 16);
    assert!(options.ca_file.is_none());
    assert!(!options.requires_tls().unwrap());
}

#[test]
fn sslmode_require_turns_tls_on() {
    let options = DbOptions::new("postgres://u:p@localhost:5432/db?sslmode=require");
    assert!(options.requires_tls().unwrap());
}

#[test]
fn a_ca_file_turns_tls_on() {
    let options = DbOptions::new("postgres://u:p@localhost:5432/db").with_ca_file(Some("/tmp/ca.pem".into()));
    assert!(options.requires_tls().unwrap());
}

#[test]
fn multi_host_urls_keep_every_host_and_the_read_write_requirement() {
    let options = DbOptions::new("postgres://u:p@10.0.0.1:5432,10.0.0.2:5432,10.0.0.3:5432/db?target_session_attrs=read-write");
    let config = options.parsed().unwrap();
    assert_eq!(config.get_hosts().len(), 3);
    assert_eq!(config.get_ports(), &[5432, 5432, 5432]);
    assert_eq!(config.get_target_session_attrs(), tokio_postgres::config::TargetSessionAttrs::ReadWrite);
}

#[test]
fn a_missing_ca_file_is_reported_when_building_the_pool() {
    let options = DbOptions::new("postgres://u:p@localhost:5432/db?sslmode=require")
        .with_ca_file(Some("/nonexistent/ca.pem".into()));
    let error = Db::connect_with_without_migrating(&options).err().expect("expected an error");
    assert!(format!("{error:#}").contains("/nonexistent/ca.pem"), "{error:#}");
}

#[test]
fn a_valid_ca_file_builds_a_tls_pool() {
    let dir = std::env::temp_dir().join(format!("harvest-db-ca-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let ca = rcgen::generate_simple_self_signed(vec!["db.internal".to_string()]).unwrap();
    let path = dir.join("ca.pem");
    std::fs::write(&path, ca.cert.pem()).unwrap();
    let options = DbOptions::new("postgres://u:p@localhost:5432/db?sslmode=require").with_ca_file(Some(path));
    assert!(Db::connect_with_without_migrating(&options).is_ok());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pool_size_is_configurable() {
    let options = DbOptions::new("postgres://u:p@localhost:5432/db").with_pool_size(3);
    let db = Db::connect_with_without_migrating(&options).unwrap();
    assert_eq!(db.max_pool_size(), 3);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn ping_succeeds_against_a_live_database() {
    let t = TestDb::new().await;
    t.db.ping().await.unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn ping_fails_when_no_host_is_reachable() {
    let options = DbOptions::new("postgres://u:p@127.0.0.1:1/db?connect_timeout=1");
    let db = Db::connect_with_without_migrating(&options).unwrap();
    assert!(db.ping().await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn multi_host_url_skips_an_unreachable_host() {
    let t = TestDb::new().await;
    let options = DbOptions::new(with_dead_first_host(&t.url));
    let db = Db::connect_with(&options).await.unwrap();
    let rows = db.query("SELECT 1 AS one", json!({})).await.unwrap();
    assert_eq!(rows[0]["one"], 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn pool_replaces_connections_killed_by_the_server() {
    let t = TestDb::new().await;
    let db = Db::connect_with(&DbOptions::new(&t.url).with_pool_size(1)).await.unwrap();
    let pid = db.query("SELECT pg_backend_pid() AS pid", json!({})).await.unwrap()[0]["pid"].as_i64().unwrap();
    let (admin, connection) = tokio_postgres::connect(&admin_url(), tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    admin.execute("SELECT pg_terminate_backend($1)", &[&(pid as i32)]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rows = db.query("SELECT pg_backend_pid() AS pid", json!({})).await.unwrap();
    assert_ne!(rows[0]["pid"].as_i64().unwrap(), pid);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn listener_receives_notifications_from_a_dedicated_client() {
    let t = TestDb::new().await;
    let channel = format!("harvest_test_{}", uuid::Uuid::new_v4().simple());
    let mut listener = t.db.listen(&[channel.as_str()]).await.unwrap();
    let publisher = t.db.dedicated_client().await.unwrap();
    publisher.execute("SELECT pg_notify($1, $2)", &[&channel, &"hello"]).await.unwrap();
    let notification = tokio::time::timeout(Duration::from_secs(5), listener.recv()).await.unwrap().unwrap();
    assert_eq!(notification.channel, channel);
    assert_eq!(notification.payload, "hello");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn listener_ends_when_its_connection_is_terminated() {
    let t = TestDb::new().await;
    let channel = format!("harvest_test_{}", uuid::Uuid::new_v4().simple());
    let mut listener = t.db.listen(&[channel.as_str()]).await.unwrap();
    let pid = listener.backend_pid();
    let (admin, connection) = tokio_postgres::connect(&admin_url(), tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    admin.execute("SELECT pg_terminate_backend($1)", &[&pid]).await.unwrap();
    let next = tokio::time::timeout(Duration::from_secs(5), listener.recv()).await.unwrap();
    assert!(next.is_none());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn schema_version_reports_the_latest_migration_after_migrating() {
    let t = TestDb::new().await;
    assert_eq!(t.db.schema_version().await.unwrap(), harvest_db::LATEST_SCHEMA_VERSION);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (set HARVEST_TEST_DATABASE_URL)"]
async fn schema_version_is_zero_before_any_migration() {
    let t = TestDb::new().await;
    let name = format!("harvest_empty_{}", uuid::Uuid::new_v4().simple());
    let (admin, connection) = tokio_postgres::connect(&admin_url(), tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(connection);
    admin.batch_execute(&format!("CREATE DATABASE {name}")).await.unwrap();
    let url = t.url.rsplit_once('/').map(|(base, _)| format!("{base}/{name}")).unwrap();
    let db = Db::connect_without_migrating(&url).unwrap();
    assert_eq!(db.schema_version().await.unwrap(), 0);
    drop(db);
    admin.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).await.unwrap();
}
