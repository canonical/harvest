use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use harvest_db::Db;

use crate::cluster::bus::ClusterBus;
use crate::cluster::node::ClusterNode;
use crate::cluster::peer::PeerClient;
use crate::config::{ClusterBackend, ClusterConfig};

pub struct ClusterRuntime {
    pub node: Arc<ClusterNode>,
    pub bus: Arc<ClusterBus>,
    pub peers: Option<Arc<PeerClient>>,
}

pub async fn start(
    db: Arc<Db>,
    cluster_config: &ClusterConfig,
    internal_addr: Option<SocketAddr>,
) -> Result<ClusterRuntime> {
    match cluster_config.backend {
        ClusterBackend::Postgres => start_postgres(db, cluster_config, internal_addr).await,
        ClusterBackend::Quorium => {
            bail!("cluster.backend=quorium is configured but Quorium runtime is not implemented")
        }
    }
}

async fn start_postgres(
    db: Arc<Db>,
    cluster_config: &ClusterConfig,
    internal_addr: Option<SocketAddr>,
) -> Result<ClusterRuntime> {
    let node = ClusterNode::new(Arc::clone(&db), cluster_config);
    node.heartbeat()
        .await
        .context("registering this node in the cluster")?;
    node.spawn_heartbeat();
    tracing::info!(node_id = node.node_id(), internal_url = ?node.internal_url(), "cluster node registered");

    let bus = ClusterBus::start(
        Arc::clone(&db),
        node.node_id().to_string(),
        Duration::from_millis(cluster_config.bus_batch_window_ms),
    );
    let peers = match (&cluster_config.shared_secret, internal_addr) {
        (Some(secret), Some(_)) => Some(PeerClient::new(node.node_id(), secret.clone())),
        _ => None,
    };

    Ok(ClusterRuntime { node, bus, peers })
}
