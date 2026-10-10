#!/usr/bin/env python3
import logging
import secrets
from pathlib import Path
from typing import Optional

import ops
from charms.data_platform_libs.v0.data_interfaces import DatabaseRequires
from charms.grafana_agent.v0.cos_agent import COSAgentProvider
from charms.haproxy.v2.haproxy_route import HaproxyRouteRequirer, LoadBalancingAlgorithm
from charms.harvest_server.v0.harvest_api import HarvestApi, HarvestApiProvider
from charms.rolling_ops.v0.rollingops import RollingOpsManager

from harvest_config import (
    API_PATHS,
    INTERNAL_PORT,
    SERVER_PORT,
    DatabaseInfo,
    ServerSettings,
    build_database_url,
    render_server_toml,
    substitute_secrets,
)
from harvest_workload import AGENT_BINARY, CA_PATH, CONFIG_PATH, Workload, WorkloadError

logger = logging.getLogger(__name__)

PEER = "harvest-peers"
SECRET_LABEL = "harvest-cluster"
SECRET_ID_KEY = "cluster-secret-id"
SCHEMA_KEY = "schema-version"
READY_TIMEOUT = 240


class Blocked(Exception):
    pass


class Waiting(Exception):
    pass


class HarvestServerCharm(ops.CharmBase):
    def __init__(self, framework: ops.Framework):
        super().__init__(framework)
        self.workload = Workload(service="server", port=SERVER_PORT, ready_path="/health/ready")
        self.database = DatabaseRequires(self, relation_name="database", database_name="harvest")
        self.route = HaproxyRouteRequirer(
            self,
            "haproxy-route",
            service="harvest-server",
            ports=[SERVER_PORT],
            paths=API_PATHS,
            hostname=self.config.get("hostname") or None,
            check_path="/health/ready",
            check_interval=5,
            check_rise=2,
            check_fall=3,
            load_balancing_algorithm=LoadBalancingAlgorithm.LEASTCONN,
            retry_count=2,
            retry_redispatch=True,
            server_timeout=3600,
            connect_timeout=10,
            queue_timeout=60,
        )
        self.api = HarvestApiProvider(self, "harvest-api")
        self.cos = COSAgentProvider(
            self,
            metrics_endpoints=[{"path": "/metrics", "port": SERVER_PORT}],
            metrics_rules_dir="./src/prometheus_alert_rules",
        )
        self.restart_manager = RollingOpsManager(self, relation="restart", callback=self._restart)

        for event in (
            self.on.install,
            self.on.upgrade_charm,
            self.on.config_changed,
            self.on.leader_elected,
            self.on.update_status,
            self.on.secret_changed,
            self.on[PEER].relation_changed,
            self.on[PEER].relation_departed,
            self.on["database"].relation_broken,
            self.on["harvest-api"].relation_joined,
            self.database.on.database_created,
            self.database.on.endpoints_changed,
            self.database.on.read_only_endpoints_changed,
            self.route.on.ready,
            self.route.on.removed,
        ):
            framework.observe(event, self._reconcile)
        framework.observe(self.on.run_migrations_action, self._on_run_migrations)
        framework.observe(self.on.show_status_action, self._on_show_status)
        framework.observe(self.on.stop, self._on_stop)

    @property
    def _peer(self) -> Optional[ops.Relation]:
        return self.model.get_relation(PEER)

    def _resource(self) -> Optional[Path]:
        try:
            return Path(self.model.resources.fetch("harvest-snap"))
        except (ops.ModelError, NameError):
            return None

    def _read_secret(self, option: str) -> Optional[dict]:
        secret_id = self.config.get(option)
        if not secret_id:
            return None
        try:
            return self.model.get_secret(id=secret_id).get_content(refresh=True)
        except (ops.SecretNotFoundError, ops.ModelError) as e:
            raise Blocked(f"cannot read secret {option}: grant it to the application") from e

    def _cluster_secrets(self) -> dict:
        peer = self._peer
        if peer is None:
            raise Waiting("waiting for the peer relation")
        secret_id = peer.data[self.app].get(SECRET_ID_KEY)
        if secret_id:
            content = self.model.get_secret(id=secret_id).get_content(refresh=True)
            imported = self._read_secret("import-secrets")
            if imported and self.unit.is_leader():
                merged = dict(content)
                merged.update({k: imported[k] for k in ("jwt-secret", "user-key-encryption-key") if k in imported})
                if merged != content:
                    self.model.get_secret(id=secret_id).set_content(merged)
                    content = merged
            return content
        if not self.unit.is_leader():
            raise Waiting("waiting for the leader to create cluster secrets")
        content = {
            "jwt-secret": secrets.token_urlsafe(48),
            "user-key-encryption-key": secrets.token_hex(32),
            "cluster-secret": secrets.token_urlsafe(32),
        }
        imported = self._read_secret("import-secrets")
        if imported:
            content.update({k: imported[k] for k in ("jwt-secret", "user-key-encryption-key") if k in imported})
        secret = self.app.add_secret(content, label=SECRET_LABEL)
        peer.data[self.app][SECRET_ID_KEY] = secret.id
        return content

    def _database_info(self) -> tuple:
        relation = self.model.get_relation("database")
        if relation is None:
            raise Blocked("requires a database relation (postgresql)")
        data = self.database.fetch_relation_data().get(relation.id, {})
        if not data.get("endpoints") or not data.get("username") or not data.get("password"):
            raise Waiting("waiting for database credentials")
        return DatabaseInfo(
            endpoints=data["endpoints"],
            read_only_endpoints=data.get("read-only-endpoints", ""),
            username=data["username"],
            password=data["password"],
            database=data.get("database") or "harvest",
            tls=str(data.get("tls", "")).lower() == "true",
        ), data.get("tls-ca")

    def _public_url(self) -> Optional[str]:
        endpoints = self.route.get_proxied_endpoints()
        if endpoints:
            return str(endpoints[0]).rstrip("/")
        return None

    def _internal_url(self) -> str:
        binding = self.model.get_binding(PEER)
        address = binding.network.ingress_address if binding else None
        if address is None:
            raise Waiting("waiting for a network address")
        return f"http://{address}:{INTERNAL_PORT}"

    def _auth_providers(self, public_url: Optional[str]) -> tuple:
        oidc = google = None
        if self.config.get("oidc-issuer-url") and self.config.get("oidc-client-id"):
            content = self._read_secret("oidc-secret") or {}
            if not public_url:
                raise Waiting("waiting for the public URL from haproxy before enabling OIDC")
            oidc = {
                "issuer_url": self.config["oidc-issuer-url"],
                "client_id": self.config["oidc-client-id"],
                "client_secret": content.get("client-secret", ""),
                "redirect_uri": f"{public_url}/auth/oidc/callback",
                "display_name": self.config.get("oidc-display-name") or "",
            }
        if self.config.get("google-client-id"):
            content = self._read_secret("google-secret") or {}
            if not public_url:
                raise Waiting("waiting for the public URL from haproxy before enabling Google login")
            google = {
                "client_id": self.config["google-client-id"],
                "client_secret": content.get("client-secret", ""),
                "redirect_uri": f"{public_url}/auth/google/callback",
            }
        return oidc, google

    def _settings(self, cluster: dict, info: DatabaseInfo, ca: Optional[str]) -> ServerSettings:
        llm = (self.config.get("llm-providers") or "").strip()
        if not llm:
            raise Blocked("set llm-providers to at least one [[llm]] block")
        try:
            llm = substitute_secrets(llm, self._read_secret("llm-secret") or {})
        except KeyError as e:
            raise Blocked(f"llm-providers references {e.args[0]} which llm-secret does not contain") from e
        public_url = self._public_url()
        oidc, google = self._auth_providers(public_url)
        return ServerSettings(
            database_url=build_database_url(info),
            database_ca_file=str(CA_PATH) if ca else None,
            pool_size=int(self.config.get("pool-size", 16)),
            node_name=self.unit.name.replace("/", "-"),
            internal_url=self._internal_url(),
            cluster_secret=cluster["cluster-secret"],
            jwt_secret=cluster["jwt-secret"],
            user_key_encryption_key=cluster["user-key-encryption-key"],
            public_url=public_url,
            llm_providers=llm,
            allow_local_login=bool(self.config.get("allow-local-login", True)),
            max_iterations=int(self.config.get("max-iterations", 25)),
            drain_timeout_secs=int(self.config.get("drain-timeout", 120)),
            binary_path=AGENT_BINARY,
            oidc=oidc,
            google=google,
            extra_toml=self.config.get("extra-toml") or "",
        )

    def _ensure_installed(self) -> None:
        if self.workload.installed():
            return
        self.unit.status = ops.MaintenanceStatus("installing the harvest snap")
        self.workload.install(self.config.get("snap-channel", "latest/edge"), self._resource())
        self.workload.set_snap_config({"config.managed": "true", "ui.mode": "static"})
        self.workload.stop(["ui"])

    def _migrate_if_leader(self) -> None:
        peer = self._peer
        if not self.unit.is_leader() or peer is None:
            return
        database_version, binary_version = self.workload.schema_versions()
        if database_version < binary_version:
            self.unit.status = ops.MaintenanceStatus("running database migrations")
            database_version = self.workload.migrate()
        peer.data[self.app][SCHEMA_KEY] = str(database_version)

    def _publish(self, public_url: Optional[str]) -> None:
        if self.model.get_relation("haproxy-route"):
            try:
                self.route.update_relation_data()
            except Exception as e:
                logger.warning("could not publish haproxy-route data: %s", e)
        peer = self._peer
        schema = int(peer.data[self.app].get(SCHEMA_KEY, "0")) if peer else 0
        self.api.publish(HarvestApi(
            version=self.workload.version(),
            schema_version=schema,
            public_url=public_url or "",
            api_paths=API_PATHS,
        ))

    def _reconcile(self, _event: ops.EventBase) -> None:
        try:
            self._ensure_installed()
            self.unit.open_port("tcp", SERVER_PORT)
            self.unit.open_port("tcp", INTERNAL_PORT)
            if not self.config.get("active-active", True) and self.app.planned_units() > 1:
                raise Blocked("active-active is false: scale to one unit")
            cluster = self._cluster_secrets()
            info, ca = self._database_info()
            settings = self._settings(cluster, info, ca)
            if ca:
                self.workload.write_file(CA_PATH, ca, 0o644)
            else:
                self.workload.remove_file(CA_PATH)
            changed = self.workload.write_file(CONFIG_PATH, render_server_toml(settings))
            try:
                self.workload.check_config()
            except WorkloadError as e:
                raise Blocked(f"invalid configuration: {e}") from e
            self._migrate_if_leader()
            if not self.workload.running():
                self.unit.status = ops.MaintenanceStatus("starting harvest")
                self.workload.start()
            elif changed:
                self.on[self.restart_manager.name].acquire_lock.emit()
            self._publish(settings.public_url)
            if self.workload.ready():
                node = self.workload.node_id()
                self.unit.status = ops.ActiveStatus(f"ready ({node})" if node else "ready")
            else:
                self.unit.status = ops.WaitingStatus("waiting for harvest to become ready")
        except Blocked as e:
            self.unit.status = ops.BlockedStatus(str(e))
        except Waiting as e:
            self.unit.status = ops.WaitingStatus(str(e))
        except WorkloadError as e:
            logger.error("workload error: %s", e)
            self.unit.status = ops.BlockedStatus(f"workload error: {e}")

    def _restart(self, _event: ops.EventBase) -> None:
        self.unit.status = ops.MaintenanceStatus("restarting harvest")
        self.workload.restart()
        if not self.workload.wait_ready(READY_TIMEOUT):
            self.unit.status = ops.WaitingStatus("harvest did not become ready after restart")
            return
        self.unit.status = ops.ActiveStatus("ready")

    def _on_stop(self, _event: ops.StopEvent) -> None:
        try:
            self.workload.stop(["server"])
        except Exception as e:
            logger.warning("could not stop harvest: %s", e)

    def _on_run_migrations(self, event: ops.ActionEvent) -> None:
        try:
            version = self.workload.migrate()
        except WorkloadError as e:
            event.fail(str(e))
            return
        if self.unit.is_leader() and self._peer is not None:
            self._peer.data[self.app][SCHEMA_KEY] = str(version)
        event.set_results({"schema-version": version})

    def _on_show_status(self, event: ops.ActionEvent) -> None:
        try:
            database, binary = self.workload.schema_versions()
        except WorkloadError as e:
            event.fail(str(e))
            return
        event.set_results({
            "node-id": self.workload.node_id() or "",
            "ready": self.workload.ready(),
            "database-schema": database,
            "binary-schema": binary,
            "version": self.workload.version(),
        })


if __name__ == "__main__":
    ops.main(HarvestServerCharm)
