use std::fmt::Write as _;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use harvest_db::Db;
use serde_json::json;

use crate::cluster::node::VERSION;
use crate::machines::MachineRegistry;
use crate::projects::live::ProjectLive;
use crate::turns::admission::TurnAdmission;
use crate::turns::metrics::TURNS;

pub struct HealthState {
    pub db: Arc<Db>,
    pub live: Arc<ProjectLive>,
    pub registry: Arc<MachineRegistry>,
    pub admission: Arc<TurnAdmission>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Readiness {
    pub database: bool,
    pub bus: bool,
    pub draining: bool,
    pub cluster: bool,
}

impl Readiness {
    pub fn ready(&self) -> bool {
        self.database && self.bus && !self.draining && self.cluster
    }
}

pub async fn readiness(state: &HealthState) -> Readiness {
    let node = state.live.node();
    Readiness {
        database: state.db.ping().await.is_ok(),
        bus: state.live.bus().is_listening(),
        draining: node.is_draining(),
        cluster: !node.is_fenced() && node.membership_health().quorum,
    }
}

pub async fn ready(State(state): State<Arc<HealthState>>) -> Response {
    let readiness = readiness(&state).await;
    let status = if readiness.ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status":   if readiness.ready() { "ready" } else { "unavailable" },
            "database": readiness.database,
            "bus":      readiness.bus,
            "draining": readiness.draining,
            "cluster":  readiness.cluster,
            "membership": state.live.node().membership_health(),
            "node_id":  state.live.node().node_id(),
        })),
    )
        .into_response()
}

pub async fn version(State(state): State<Arc<HealthState>>) -> Response {
    let schema = state.db.schema_version().await.ok();
    Json(json!({
        "version":         VERSION,
        "schema_version":  schema,
        "binary_schema":   harvest_db::LATEST_SCHEMA_VERSION,
        "node_id":         state.live.node().node_id(),
    }))
    .into_response()
}

pub async fn metrics(State(state): State<Arc<HealthState>>) -> Response {
    let readiness = readiness(&state).await;
    let node_id = state.live.node().node_id().to_string();
    let local_turns = state.live.local_turn_count().await;
    let local_agents = state.registry.agents.len();
    let pool_size = state.db.max_pool_size();
    let mut out = String::new();
    let _ = writeln!(out, "# TYPE harvest_node_info gauge");
    let _ = writeln!(
        out,
        "harvest_node_info{{node_id=\"{node_id}\",version=\"{VERSION}\"}} 1"
    );
    let _ = writeln!(out, "# TYPE harvest_ready gauge");
    let _ = writeln!(out, "harvest_ready {}", u8::from(readiness.ready()));
    let _ = writeln!(out, "# TYPE harvest_database_up gauge");
    let _ = writeln!(out, "harvest_database_up {}", u8::from(readiness.database));
    let _ = writeln!(out, "# TYPE harvest_bus_listening gauge");
    let _ = writeln!(out, "harvest_bus_listening {}", u8::from(readiness.bus));
    let _ = writeln!(out, "# TYPE harvest_draining gauge");
    let _ = writeln!(out, "harvest_draining {}", u8::from(readiness.draining));
    let membership = state.live.node().membership_health();
    let _ = writeln!(out, "# TYPE harvest_cluster_quorum gauge");
    let _ = writeln!(
        out,
        "harvest_cluster_quorum{{backend=\"{}\"}} {}",
        membership.backend,
        u8::from(membership.quorum)
    );
    let _ = writeln!(out, "# TYPE harvest_cluster_fenced gauge");
    let _ = writeln!(
        out,
        "harvest_cluster_fenced {}",
        u8::from(state.live.node().is_fenced())
    );
    let _ = writeln!(out, "# TYPE harvest_cluster_voters gauge");
    let _ = writeln!(out, "harvest_cluster_voters {}", membership.voters);
    let _ = writeln!(out, "# TYPE harvest_local_active_turns gauge");
    let _ = writeln!(out, "harvest_local_active_turns {local_turns}");
    let _ = writeln!(out, "# TYPE harvest_local_connected_agents gauge");
    let _ = writeln!(out, "harvest_local_connected_agents {local_agents}");
    let _ = writeln!(out, "# TYPE harvest_db_pool_max_size gauge");
    let _ = writeln!(out, "harvest_db_pool_max_size {pool_size}");
    let _ = writeln!(out, "# TYPE harvest_admitted_turns gauge");
    let _ = writeln!(out, "harvest_admitted_turns {}", state.admission.active());
    TURNS.render(&mut out);
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_requires_database_bus_and_no_drain() {
        assert!(Readiness {
            database: true,
            bus: true,
            draining: false,
            cluster: true
        }
        .ready());
        assert!(!Readiness {
            database: false,
            bus: true,
            draining: false,
            cluster: true
        }
        .ready());
        assert!(!Readiness {
            database: true,
            bus: false,
            draining: false,
            cluster: true
        }
        .ready());
        assert!(!Readiness {
            database: true,
            bus: true,
            draining: true,
            cluster: true
        }
        .ready());
        assert!(!Readiness {
            database: true,
            bus: true,
            draining: false,
            cluster: false
        }
        .ready());
    }
}
