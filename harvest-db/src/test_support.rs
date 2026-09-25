//! Throwaway databases for integration tests.
//!
//! `HARVEST_TEST_DATABASE_URL` must point at a server where the role may `CREATE DATABASE`,
//! e.g. `postgres://harvest:harvest@127.0.0.1:5432/postgres`. Each test gets its own database,
//! cloned from a migrated template so setup stays fast.

use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio_postgres::NoTls;

use crate::{migrations, Db};

pub const TEST_DATABASE_URL_VAR: &str = "HARVEST_TEST_DATABASE_URL";

const TEMPLATE_LOCK_KEY: i64 = 0x4841_5256_5445_5354; // "HARVTEST"

pub struct TestDb {
    pub db: Db,
    pub url: String,
    admin_url: String,
    name: String,
}

impl TestDb {
    pub async fn new() -> Self {
        Self::try_new().await.expect("failed to create test database")
    }

    pub async fn try_new() -> Result<Self> {
        let admin_url = std::env::var(TEST_DATABASE_URL_VAR)
            .with_context(|| format!("{TEST_DATABASE_URL_VAR} is not set"))?;
        let template = ensure_template(&admin_url).await?;
        let name = format!("harvest_test_{}", uuid::Uuid::new_v4().simple());
        let admin = connect(&admin_url).await?;
        let mut attempts = 0;
        loop {
            match admin.batch_execute(&format!("CREATE DATABASE {name} TEMPLATE {template}")).await {
                Ok(()) => break,
                Err(e) if attempts < 50 && e.to_string().contains("being accessed by other users") => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
        let url = url_for_database(&admin_url, &name)?;
        let db = Db::connect(&url).await?;
        Ok(Self { db, url, admin_url, name })
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let admin_url = self.admin_url.clone();
        let name = self.name.clone();
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
            runtime.block_on(async {
                if let Ok(admin) = connect(&admin_url).await {
                    let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).await;
                }
            });
        }).join();
    }
}

async fn ensure_template(admin_url: &str) -> Result<String> {
    let template = format!("harvest_test_template_{:016x}", migrations::fingerprint());
    let admin = connect(admin_url).await?;
    admin.execute("SELECT pg_advisory_lock($1)", &[&TEMPLATE_LOCK_KEY]).await?;
    let result = async {
        let exists = admin
            .query_opt("SELECT 1 FROM pg_database WHERE datname = $1", &[&template])
            .await?
            .is_some();
        if !exists {
            admin.batch_execute(&format!("CREATE DATABASE {template}")).await?;
            let (mut client, connection) = tokio_postgres::connect(&url_for_database(admin_url, &template)?, NoTls).await?;
            let task = tokio::spawn(connection);
            let migrated = migrations::run(&mut client).await;
            drop(client);
            let _ = task.await;
            if let Err(e) = migrated {
                admin.batch_execute(&format!("DROP DATABASE IF EXISTS {template}")).await?;
                return Err(e);
            }
        }
        Ok(())
    }.await;
    admin.execute("SELECT pg_advisory_unlock($1)", &[&TEMPLATE_LOCK_KEY]).await?;
    result.map(|()| template)
}

async fn connect(url: &str) -> Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await
        .context("could not connect to the test database server")?;
    tokio::spawn(async move { let _ = connection.await; });
    Ok(client)
}

fn url_for_database(admin_url: &str, dbname: &str) -> Result<String> {
    let config = tokio_postgres::Config::from_str(admin_url)?;
    let user = config.get_user().unwrap_or("postgres");
    let password = config.get_password()
        .map(|p| format!(":{}", String::from_utf8_lossy(p)))
        .unwrap_or_default();
    let host = match config.get_hosts().first() {
        Some(tokio_postgres::config::Host::Tcp(h)) => h.clone(),
        #[cfg(unix)]
        Some(tokio_postgres::config::Host::Unix(p)) => p.display().to_string(),
        None => "localhost".into(),
    };
    let port = config.get_ports().first().copied().unwrap_or(5432);
    Ok(format!("postgres://{user}{password}@{host}:{port}/{dbname}"))
}
