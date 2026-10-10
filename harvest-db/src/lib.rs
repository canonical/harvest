mod connection;
pub(crate) mod migrations;
mod params;
mod rows;
pub mod test_support;

use std::ops::DerefMut;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod, Runtime};
use serde_json::Value;
use tokio_postgres::types::ToSql;
use tokio_postgres::NoTls;

pub use tokio_postgres;
pub use connection::{DbOptions, Listener, Notification, DEFAULT_POOL_SIZE};
use connection::Tls;

use params::{bind_named, JsonParam};
use rows::row_to_json;

pub const GRAPH_READER_ROLE: &str = "harvest_graph_reader";

pub const LATEST_SCHEMA_VERSION: i32 = migrations::latest_version();

pub fn now_rfc3339() -> String {
    use chrono::SubsecRound;
    chrono::Utc::now().trunc_subsecs(6).to_rfc3339()
}

const POOL_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
const POOL_CREATE_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_RECYCLE_TIMEOUT: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct Db {
    pool: Pool,
    config: Arc<tokio_postgres::Config>,
    tls: Tls,
}

impl Db {
    pub async fn connect(url: &str) -> Result<Self> {
        Self::connect_with(&DbOptions::new(url)).await
    }

    pub async fn connect_with(options: &DbOptions) -> Result<Self> {
        let db = Self::connect_with_without_migrating(options)?;
        db.migrate().await?;
        Ok(db)
    }

    pub fn connect_without_migrating(url: &str) -> Result<Self> {
        Self::connect_with_without_migrating(&DbOptions::new(url))
    }

    pub fn connect_with_without_migrating(options: &DbOptions) -> Result<Self> {
        let config = options.parsed()?;
        let tls = Tls::from_options(options)?;
        let manager_config = ManagerConfig { recycling_method: RecyclingMethod::Verified };
        let manager = match &tls {
            Tls::Plain => Manager::from_config(config.clone(), NoTls, manager_config),
            Tls::Rustls(connector) => Manager::from_config(config.clone(), connector.clone(), manager_config),
        };
        let pool = Pool::builder(manager)
            .max_size(options.pool_size)
            .runtime(Runtime::Tokio1)
            .wait_timeout(Some(POOL_WAIT_TIMEOUT))
            .create_timeout(Some(POOL_CREATE_TIMEOUT))
            .recycle_timeout(Some(POOL_RECYCLE_TIMEOUT))
            .build()?;
        Ok(Self { pool, config: Arc::new(config), tls })
    }

    pub fn max_pool_size(&self) -> usize {
        self.pool.status().max_size
    }

    pub async fn ping(&self) -> Result<()> {
        tokio::time::timeout(PING_TIMEOUT, async {
            let client = self.client().await?;
            client.simple_query("SELECT 1").await.context("database ping failed")?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("database ping timed out")?
    }

    pub async fn dedicated_client(&self) -> Result<tokio_postgres::Client> {
        connection::connect_dedicated(&self.config, &self.tls, None).await
    }

    pub async fn listen(&self, channels: &[&str]) -> Result<Listener> {
        connection::listen(&self.config, &self.tls, channels).await
    }

    pub async fn schema_version(&self) -> Result<i32> {
        let client = self.client().await?;
        migrations::applied_version(&client).await
    }

    pub async fn migrate(&self) -> Result<()> {
        let mut client = self.client().await?;
        migrations::run(client.deref_mut()).await
    }

    async fn client(&self) -> Result<Object> {
        self.pool.get().await.context("could not get a database connection")
    }

    pub async fn query(&self, sql: &str, params: Value) -> Result<Vec<Value>> {
        let client = self.client().await?;
        query_on(&client, sql, &params).await
    }

    pub async fn execute(&self, sql: &str, params: Value) -> Result<u64> {
        let client = self.client().await?;
        execute_on(&client, sql, &params).await
    }

    pub async fn batch_execute(&self, sql: &str) -> Result<()> {
        self.client().await?.batch_execute(sql).await?;
        Ok(())
    }

    pub async fn begin(&self) -> Result<Tx> {
        let client = self.client().await?;
        client.batch_execute("BEGIN").await?;
        Ok(Tx { client: Some(client) })
    }
}

pub struct Tx {
    client: Option<Object>,
}

impl Tx {
    fn client(&self) -> &Object {
        self.client.as_ref().expect("transaction already finished")
    }

    pub async fn query(&self, sql: &str, params: Value) -> Result<Vec<Value>> {
        query_on(self.client(), sql, &params).await
    }

    pub async fn execute(&self, sql: &str, params: Value) -> Result<u64> {
        execute_on(self.client(), sql, &params).await
    }

    pub async fn query_uncached(&self, sql: &str, params: Value) -> Result<Vec<Value>> {
        let (sql, values) = bind_named(sql, &params)?;
        let wrapped: Vec<JsonParam> = values.iter().map(JsonParam).collect();
        let refs: Vec<&(dyn ToSql + Sync)> = wrapped.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
        let rows = self.client().query(sql.as_str(), &refs).await?;
        rows.iter().map(row_to_json).collect()
    }

    pub async fn batch_execute(&self, sql: &str) -> Result<()> {
        self.client().batch_execute(sql).await?;
        Ok(())
    }

    pub async fn commit(mut self) -> Result<()> {
        let client = self.client.take().expect("transaction already finished");
        client.batch_execute("COMMIT").await?;
        Ok(())
    }

    pub async fn rollback(mut self) -> Result<()> {
        let client = self.client.take().expect("transaction already finished");
        client.batch_execute("ROLLBACK").await?;
        Ok(())
    }
}

impl Drop for Tx {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Err(e) = client.batch_execute("ROLLBACK").await {
                        tracing::warn!(error = %e, "rollback of abandoned transaction failed");
                    }
                });
            }
        }
    }
}

async fn query_on(client: &Object, sql: &str, params: &Value) -> Result<Vec<Value>> {
    let (sql, values) = bind_named(sql, params)?;
    let stmt = client.prepare_cached(&sql).await
        .with_context(|| format!("failed to prepare: {sql}"))?;
    let wrapped: Vec<JsonParam> = values.iter().map(JsonParam).collect();
    let refs: Vec<&(dyn ToSql + Sync)> = wrapped.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
    let rows = client.query(&stmt, &refs).await?;
    rows.iter().map(row_to_json).collect()
}

async fn execute_on(client: &Object, sql: &str, params: &Value) -> Result<u64> {
    let (sql, values) = bind_named(sql, params)?;
    let stmt = client.prepare_cached(&sql).await
        .with_context(|| format!("failed to prepare: {sql}"))?;
    let wrapped: Vec<JsonParam> = values.iter().map(JsonParam).collect();
    let refs: Vec<&(dyn ToSql + Sync)> = wrapped.iter().map(|p| p as &(dyn ToSql + Sync)).collect();
    Ok(client.execute(&stmt, &refs).await?)
}

pub fn is_unique_violation(err: &anyhow::Error) -> bool {
    sql_state(err).as_deref() == Some("23505")
}

pub fn is_foreign_key_violation(err: &anyhow::Error) -> bool {
    sql_state(err).as_deref() == Some("23503")
}

fn sql_state(err: &anyhow::Error) -> Option<String> {
    err.chain()
        .find_map(|e| e.downcast_ref::<tokio_postgres::Error>())
        .and_then(|e| e.code().map(|c| c.code().to_string()))
}
