use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Router;
use harvest_db::Db;
use quorium::Quorium;
use tokio::net::TcpListener;
use tower::ServiceExt as _;

use crate::cluster::admin::{self, ClusterAdmin};
use crate::cluster::bus::ClusterBus;
use crate::cluster::membership::Membership;
use crate::cluster::node::{ClusterNode, NodeIdentity};
use crate::cluster::peer::{require_secret, ClusterSecrets};
use crate::cluster::postgres::PostgresMembership;
use crate::cluster::quorium::QuoriumMembership;
use crate::cluster::tls::{self, ClusterTls};
use crate::cluster::{mesh, quorium as quorium_backend, Shutdown};
use crate::config::{BusTransport, ClusterBackend, ClusterConfig};

const REGISTRATION_RETRY: Duration = Duration::from_millis(250);

pub struct ClusterRuntime {
    pub node: Arc<ClusterNode>,
    pub bus: Arc<ClusterBus>,
    pub quorium: Option<Arc<Quorium>>,
    pub internal: Option<InternalServer>,
    pub tls: Option<Arc<ClusterTls>>,
    cluster_routes: Router,
}

impl ClusterRuntime {
    pub fn serve(&mut self, app: Router) -> Option<tokio::task::JoinHandle<()>> {
        let internal = self.internal.take()?;
        internal.complete(app.merge(self.cluster_routes.clone()));
        Some(internal.handle)
    }
}

pub struct InternalServer {
    pub addr: SocketAddr,
    pub handle: tokio::task::JoinHandle<()>,
    late: Arc<OnceLock<Router>>,
}

impl InternalServer {
    fn complete(&self, router: Router) {
        let _ = self.late.set(router);
    }
}

async fn late_bound(State(late): State<Arc<OnceLock<Router>>>, request: Request) -> Response {
    match late.get() {
        Some(router) => match router.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        },
        None => (StatusCode::SERVICE_UNAVAILABLE, "starting").into_response(),
    }
}

fn serve_internal(
    listener: TcpListener,
    early: Router,
    secrets: Arc<ClusterSecrets>,
    tls: Option<&ClusterTls>,
    shutdown: &Shutdown,
) -> Result<InternalServer> {
    let addr = listener.local_addr()?;
    let late = Arc::new(OnceLock::new());
    let fallback = Router::new()
        .fallback(late_bound)
        .with_state(Arc::clone(&late));
    let app = early
        .layer(axum::middleware::from_fn_with_state(
            secrets,
            require_secret,
        ))
        .merge(fallback);
    let stop = shutdown.wait();
    let handle = match tls.map(ClusterTls::server_config).transpose()? {
        Some(config) => tokio::spawn(tls::serve(listener, app, config, stop)),
        None => tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(stop)
            .await;
        }),
    };
    Ok(InternalServer { addr, handle, late })
}

async fn register(node: &ClusterNode, patience: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + patience;
    loop {
        match node.heartbeat().await {
            Ok(()) => return Ok(()),
            Err(e) if tokio::time::Instant::now() < deadline && !node.is_fenced() => {
                tracing::info!(error = %e, "waiting to register this node in the cluster");
                tokio::time::sleep(REGISTRATION_RETRY).await;
            }
            Err(e) => return Err(e.context("registering this node in the cluster")),
        }
    }
}

pub async fn start(
    config: &ClusterConfig,
    db: &Arc<Db>,
    internal_listener: Option<TcpListener>,
    shutdown: &Shutdown,
) -> Result<ClusterRuntime> {
    let identity = NodeIdentity::from_config(config);
    let tls = config.tls.as_ref().map(ClusterTls::load).transpose()?;
    let secret = config.effective_shared_secret().unwrap_or_default();
    let secrets = ClusterSecrets::new(Some(secret.clone()), &config.accepted_secrets);
    let (membership, quorium, early): (Arc<dyn Membership>, Option<Arc<Quorium>>, Router) =
        match config.backend {
            ClusterBackend::Postgres => {
                (PostgresMembership::new(Arc::clone(db)), None, Router::new())
            }
            ClusterBackend::Quorium => {
                let advertise = identity
                    .internal_url
                    .clone()
                    .context("the quorium backend needs an internal url")?;
                let quorium =
                    quorium_backend::start(config, &identity.node_id, &advertise, tls.as_deref())
                        .await?;
                let early = quorium::api::router(Arc::clone(&quorium));
                (
                    QuoriumMembership::new(Arc::clone(&quorium)),
                    Some(quorium),
                    early,
                )
            }
        };
    let internal = internal_listener
        .map(|listener| {
            serve_internal(
                listener,
                early,
                Arc::clone(&secrets),
                tls.as_deref(),
                shutdown,
            )
        })
        .transpose()?;
    if let Some(quorium) = &quorium {
        quorium_backend::bootstrap(quorium, config)
            .await
            .context("joining the quorium cluster")?;
    }

    let node = ClusterNode::assemble(identity, config, membership, Arc::clone(db));
    register(&node, Duration::from_secs(config.join_timeout_secs)).await?;
    node.spawn_heartbeat();
    tracing::info!(
        node_id = node.node_id(),
        internal_url = ?node.internal_url(),
        backend = node.membership_health().backend,
        "cluster node registered"
    );

    let batch_window = Duration::from_millis(config.bus_batch_window_ms);
    let (bus, mesh_routes) = match config.bus_transport() {
        BusTransport::Notify => (
            ClusterBus::start(Arc::clone(db), node.node_id().to_string(), batch_window),
            Router::new(),
        ),
        BusTransport::Mesh => {
            let connector = tls
                .as_deref()
                .map(ClusterTls::websocket_connector)
                .transpose()?;
            let (bus, mesh) =
                mesh::start(Arc::clone(&node), secret.clone(), batch_window, connector);
            (bus, mesh::router(mesh))
        }
    };
    let admin = Arc::new(ClusterAdmin {
        node: Arc::clone(&node),
        quorium: quorium.clone(),
        shared_secret: secret,
    });
    let cluster_routes =
        mesh_routes
            .merge(admin::router(admin))
            .layer(axum::middleware::from_fn_with_state(
                secrets,
                require_secret,
            ));
    Ok(ClusterRuntime {
        node,
        bus,
        quorium,
        internal,
        tls,
        cluster_routes,
    })
}
