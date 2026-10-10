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

pub struct HealthState {
    pub db:       Arc<Db>,
    pub live:     Arc<ProjectLive>,
    pub registry: Arc<MachineRegistry>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Readiness {
    pub database: bool,
    pub bus:      bool,
    pub draining: bool,
}

impl Readiness {
    pub fn ready(&self) -> bool {
        self.database && self.bus && !self.draining
    }
}

pub async fn readiness(state: &HealthState) -> Readiness {
    Readiness {
        database: state.db.ping().await.is_ok(),
        bus:      state.live.bus().is_listening(),
        draining: state.live.node().is_draining(),
    }
}

pub async fn ready(State(state): State<Arc<HealthState>>) -> Response {
    let readiness = readiness(&state).await;
    let status = if readiness.ready() { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (status, Json(json!({
        "status":   if readiness.ready() { "ready" } else { "unavailable" },
        "database": readiness.database,
        "bus":      readiness.bus,
        "draining": readiness.draining,
        "node_id":  state.live.node().node_id(),
    }))).into_response()
}

pub async fn version(State(state): State<Arc<HealthState>>) -> Response {
    let schema = state.db.schema_version().await.ok();
    Json(json!({
        "version":         VERSION,
        "schema_version":  schema,
        "binary_schema":   harvest_db::LATEST_SCHEMA_VERSION,
        "node_id":         state.live.node().node_id(),
    })).into_response()
}

pub async fn metrics(State(state): State<Arc<HealthState>>) -> Response {
    let readiness = readiness(&state).await;
    let node_id = state.live.node().node_id().to_string();
    let local_turns = state.live.local_turn_count().await;
    let local_agents = state.registry.agents.len();
    let pool_size = state.db.max_pool_size();
    let mut out = String::new();
    let _ = writeln!(out, "# TYPE harvest_node_info gauge");
    let _ = writeln!(out, "harvest_node_info{{node_id=\"{node_id}\",version=\"{VERSION}\"}} 1");
    let _ = writeln!(out, "# TYPE harvest_ready gauge");
    let _ = writeln!(out, "harvest_ready {}", u8::from(readiness.ready()));
    let _ = writeln!(out, "# TYPE harvest_database_up gauge");
    let _ = writeln!(out, "harvest_database_up {}", u8::from(readiness.database));
    let _ = writeln!(out, "# TYPE harvest_bus_listening gauge");
    let _ = writeln!(out, "harvest_bus_listening {}", u8::from(readiness.bus));
    let _ = writeln!(out, "# TYPE harvest_draining gauge");
    let _ = writeln!(out, "harvest_draining {}", u8::from(readiness.draining));
    let _ = writeln!(out, "# TYPE harvest_local_active_turns gauge");
    let _ = writeln!(out, "harvest_local_active_turns {local_turns}");
    let _ = writeln!(out, "# TYPE harvest_local_connected_agents gauge");
    let _ = writeln!(out, "harvest_local_connected_agents {local_agents}");
    let _ = writeln!(out, "# TYPE harvest_db_pool_max_size gauge");
    let _ = writeln!(out, "harvest_db_pool_max_size {pool_size}");
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], out).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_requires_database_bus_and_no_drain() {
        assert!(Readiness { database: true, bus: true, draining: false }.ready());
        assert!(!Readiness { database: false, bus: true, draining: false }.ready());
        assert!(!Readiness { database: true, bus: false, draining: false }.ready());
        assert!(!Readiness { database: true, bus: true, draining: true }.ready());
    }
}
