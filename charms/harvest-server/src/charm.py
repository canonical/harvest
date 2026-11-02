#!/usr/bin/env python3
import json
import logging
import secrets
import time
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
    BACKENDS,
    INTERNAL_PORT,
    SERVER_PORT,
    DatabaseInfo,
    ServerSettings,
    build_database_url,
    database_hosts,
    render_server_toml,
)
from harvest_workload import AGENT_BINARY, CA_PATH, CONFIG_PATH, ClusterAdmin, Workload, WorkloadError
from quorium_cluster import (
    PHASE,
    RotationInProgress,
    advance_rotation,
    join_plan,
    rendered_secrets,
    report_fields,
    report_from,
    restart_is_safe,
    stage_rotation,
    stale_servers,
)

logger = logging.getLogger(__name__)

PEER = "harvest-peers"
SECRET_LABEL = "harvest-cluster"
SECRET_ID_KEY = "cluster-secret-id"
SCHEMA_KEY = "schema-version"
EVACUATED_KEY = "evacuated-units"
SEED_KEY = "quorium-seed"
ADDRESS_KEY = "internal-url"
READY_TIMEOUT = 240
MEMBER_WAIT = 600


class Blocked(Exception):
    pass


class Waiting(Exception):
    pass


class HarvestServerCharm(ops.CharmBase):
    _stored = ops.StoredState()

    def __init__(self, framework: ops.Framework):
        super().__init__(framework)
        self._stored.set_default(member_wait_since=0.0, join_addresses=[], restart_pending=False)
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
            self.on["llm-provider"].relation_changed,
            self.on["llm-provider"].relation_broken,
            self.on["auth-provider"].relation_changed,
            self.on["auth-provider"].relation_broken,
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
        framework.observe(self.on.cluster_status_action, self._on_cluster_status)
        framework.observe(self.on.cluster_rebalance_action, self._on_cluster_rebalance)
        framework.observe(self.on.cluster_evacuate_action, self._on_cluster_evacuate)
        framework.observe(self.on.cluster_remove_action, self._on_cluster_remove)
        framework.observe(self.on.cluster_restore_action, self._on_cluster_restore)
        framework.observe(self.on.cluster_rotate_secrets_action, self._on_cluster_rotate_secrets)
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

    def _backend(self) -> str:
        backend = str(self.config.get("cluster-backend", "postgres")).strip().lower()
        if backend not in BACKENDS:
            raise Blocked(f"cluster-backend must be one of: {', '.join(BACKENDS)}")
        return backend

    def _unit_databags(self) -> list:
        relation = self._peer
        if relation is None:
            return []
        return [relation.data[self.unit], *(relation.data[unit] for unit in relation.units)]

    def _peer_addresses(self) -> dict:
        relation = self._peer
        if relation is None:
            return {}
        units = [self.unit, *relation.units]
        addresses = {unit.name: relation.data[unit].get(ADDRESS_KEY, "") for unit in units}
        return {name: url for name, url in addresses.items() if url}

    def _every_peer_published(self) -> bool:
        relation = self._peer
        return relation is not None and len(self._peer_addresses()) == len(relation.units) + 1

    def _seed(self) -> Optional[str]:
        relation = self._peer
        if relation is None:
            return None
        seed = relation.data[self.app].get(SEED_KEY)
        if seed or not self.unit.is_leader():
            return seed
        relation.data[self.app][SEED_KEY] = self.unit.name
        return self.unit.name

    def _join_plan(self, backend: str) -> tuple:
        if backend != "quorium":
            return None, ()
        bootstrap, addresses = join_plan(self.unit.name, self._seed(), self._peer_addresses())
        if bootstrap is None:
            raise Waiting("waiting for the leader to seed the quorium cluster")
        if bootstrap == "join":
            if not self._stored.join_addresses:
                if not addresses:
                    raise Waiting("waiting for a peer address to join")
                self._stored.join_addresses = list(addresses)
            addresses = tuple(self._stored.join_addresses)
        return bootstrap, addresses

    def _secret_handle(self) -> Optional[ops.Secret]:
        peer = self._peer
        secret_id = peer.data[self.app].get(SECRET_ID_KEY) if peer else None
        return self.model.get_secret(id=secret_id) if secret_id else None

    def _advance_rotation(self, cluster: dict) -> dict:
        if not self.unit.is_leader():
            return cluster
        updated = advance_rotation(cluster, (report_from(data) for data in self._unit_databags()))
        if updated is None:
            return cluster
        self._secret_handle().set_content(updated)
        self._peer.data[self.app][PHASE] = updated.get(PHASE, "")
        logger.info("cluster secret rotation advanced to %s", updated.get(PHASE) or "done")
        return updated

    def _local_admin(self, cluster: dict) -> ClusterAdmin:
        return ClusterAdmin(f"http://127.0.0.1:{INTERNAL_PORT}", rendered_secrets(cluster)[0])

    def _prune_departed_servers(self, cluster: dict) -> None:
        if not self.unit.is_leader() or not self._every_peer_published():
            return
        admin = self._local_admin(cluster)
        try:
            for address in stale_servers(admin.servers(), self._peer_addresses().values()):
                logger.info("removing departed quorium server %s", address)
                admin.remove_server(address)
        except WorkloadError as e:
            logger.warning("could not prune departed quorium servers: %s", e)

    def _publish_running_secrets(self, cluster: dict) -> None:
        relation = self._peer
        if relation is None or self._stored.restart_pending:
            return
        primary, accepted = rendered_secrets(cluster)
        relation.data[self.unit].update(report_fields(primary, accepted))

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

    def _wait_for_members(self, info: DatabaseInfo) -> None:
        if self.workload.running():
            return
        relation = self.model.get_relation("database")
        expected = len(relation.units) if relation else 0
        listed = len(database_hosts(info))
        if listed >= expected:
            return
        if not self._stored.member_wait_since:
            self._stored.member_wait_since = time.time()
        if time.time() - self._stored.member_wait_since < MEMBER_WAIT:
            raise Waiting(f"waiting for every PostgreSQL member ({listed}/{expected})")

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
        oidc = None
        google = None
        for relation in self.model.relations.get("auth-provider", []):
            app = relation.app
            if app is None:
                continue
            data = relation.data[app]
            if str(data.get("enabled", "true")).lower() == "false":
                continue
            provider = (data.get("provider") or "").strip().lower()
            secret_id = (data.get("secret_id") or "").strip()
            secret_content = {}
            if secret_id:
                try:
                    secret_content = self.model.get_secret(id=secret_id).get_content(refresh=True)
                except (ops.SecretNotFoundError, ops.ModelError) as e:
                    raise Blocked(f"cannot read auth-provider secret from relation {relation.id}: grant it to the application") from e
            if provider == "oidc":
                issuer_url = (data.get("issuer_url") or "").strip()
                client_id = (data.get("client_id") or "").strip()
                if not issuer_url or not client_id:
                    raise Blocked(f"auth-provider relation {relation.id} for oidc is missing issuer_url or client_id")
                if not public_url:
                    raise Waiting("waiting for the public URL from haproxy before enabling OIDC")
                oidc = {
                    "issuer_url": issuer_url,
                    "client_id": client_id,
                    "client_secret": secret_content.get("client-secret", ""),
                    "redirect_uri": f"{public_url}/auth/oidc/callback",
                    "display_name": (data.get("display_name") or "").strip(),
                }
            elif provider == "google":
                client_id = (data.get("client_id") or "").strip()
                if not client_id:
                    raise Blocked(f"auth-provider relation {relation.id} for google is missing client_id")
                if not public_url:
                    raise Waiting("waiting for the public URL from haproxy before enabling Google login")
                google = {
                    "client_id": client_id,
                    "client_secret": secret_content.get("client-secret", ""),
                    "redirect_uri": f"{public_url}/auth/google/callback",
                }
            else:
                raise Blocked(f"auth-provider relation {relation.id} has unsupported provider {provider or '<empty>'}")
        return oidc, google

    def _llm_providers(self) -> str:
        providers = []
        for relation in self.model.relations.get("llm-provider", []):
            app = relation.app
            if app is None:
                continue
            data = relation.data[app]
            if str(data.get("enabled", "true")).lower() == "false":
                continue
            provider = (data.get("provider") or "").strip()
            provider_id = (data.get("id") or app.name or "").strip()
            provider_name = (data.get("name") or app.name or "").strip()
            model = (data.get("model") or "").strip()
            if not provider or not model:
                raise Blocked(f"llm-provider relation {relation.id} is missing provider or model")
            try:
                priority = int((data.get("priority") or "100").strip())
            except ValueError as e:
                raise Blocked(f"llm-provider relation {relation.id} has invalid priority") from e
            entry = {
                "relation_id": relation.id,
                "provider": provider,
                "id": provider_id,
                "name": provider_name,
                "model": model,
                "base_url": (data.get("base_url") or "").strip(),
                "user_provided_key": str(data.get("user_provided_key", "false")).lower() == "true",
                "timeout_secs": (data.get("timeout_secs") or "").strip(),
                "max_retries": (data.get("max_retries") or "").strip(),
                "priority": priority,
                "api_key": "",
            }
            if entry["timeout_secs"]:
                try:
                    int(entry["timeout_secs"])
                except ValueError as e:
                    raise Blocked(f"llm-provider relation {relation.id} has invalid timeout_secs") from e
            if entry["max_retries"]:
                try:
                    int(entry["max_retries"])
                except ValueError as e:
                    raise Blocked(f"llm-provider relation {relation.id} has invalid max_retries") from e
            secret_id = (data.get("secret_id") or "").strip()
            if secret_id:
                try:
                    secret_content = self.model.get_secret(id=secret_id).get_content(refresh=True)
                except (ops.SecretNotFoundError, ops.ModelError) as e:
                    raise Blocked(f"cannot read llm-provider secret from relation {relation.id}: grant it to the application") from e
                entry["api_key"] = secret_content.get("api-key", "")
            providers.append(entry)
        if not providers:
            raise Blocked("relate at least one llm-provider application")
        providers.sort(key=lambda item: (item["priority"], item["relation_id"]))
        lines = []
        for item in providers:
            lines.extend([
                "[[llm]]",
                f'provider = "{item["provider"]}"',
                f'id = "{item["id"]}"',
                f'name = "{item["name"]}"',
                f'model = "{item["model"]}"',
            ])
            if item["base_url"]:
                lines.append(f'base_url = "{item["base_url"]}"')
            lines.append(f'api_key = "{item["api_key"]}"')
            if item["user_provided_key"]:
                lines.append("user_provided_key = true")
            if item["timeout_secs"]:
                lines.append(f"timeout_secs = {item['timeout_secs']}")
            if item["max_retries"]:
                lines.append(f"max_retries = {item['max_retries']}")
            lines.append("")
        return "\n".join(lines).strip()

    def _settings(self, cluster: dict, info: DatabaseInfo, ca: Optional[str]) -> ServerSettings:
        backend = self._backend()
        bootstrap, join_addresses = self._join_plan(backend)
        primary, accepted = rendered_secrets(cluster)
        llm = self._llm_providers()
        public_url = self._public_url()
        oidc, google = self._auth_providers(public_url)
        allow_local_login = bool(self.config.get("allow-local-login", True))
        if not allow_local_login and not oidc and not google:
            raise Blocked("allow-local-login is false but no auth-provider relation is configured")
        return ServerSettings(
            database_url=build_database_url(info),
            database_ca_file=str(CA_PATH) if ca else None,
            pool_size=int(self.config.get("pool-size", 16)),
            node_name=self.unit.name.replace("/", "-"),
            internal_url=self._internal_url(),
            cluster_secret=primary,
            jwt_secret=cluster["jwt-secret"],
            user_key_encryption_key=cluster["user-key-encryption-key"],
            public_url=public_url,
            llm_providers=llm,
            allow_local_login=allow_local_login,
            max_iterations=int(self.config.get("max-iterations", 25)),
            drain_timeout_secs=int(self.config.get("drain-timeout", 120)),
            binary_path=AGENT_BINARY,
            oidc=oidc,
            google=google,
            extra_toml=self.config.get("extra-toml") or "",
            backend=backend,
            bootstrap=bootstrap,
            join_addresses=join_addresses,
            accepted_secrets=accepted,
        )

    def _ensure_installed(self) -> None:
        resource = self._resource()
        if not self.workload.needs_install(resource):
            return
        self.unit.status = ops.MaintenanceStatus("installing the harvest snap")
        self.workload.install(self.config.get("snap-channel", "latest/edge"), resource)
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
            cluster = self._advance_rotation(self._cluster_secrets())
            self._peer.data[self.unit][ADDRESS_KEY] = self._internal_url()
            info, ca = self._database_info()
            self._wait_for_members(info)
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
                self._stored.restart_pending = False
            elif changed or self._stored.restart_pending:
                self._stored.restart_pending = True
                self.on[self.restart_manager.name].acquire_lock.emit()
            self._publish(settings.public_url)
            if self.workload.ready():
                self._publish_running_secrets(cluster)
                if settings.backend == "quorium":
                    self._prune_departed_servers(cluster)
                self.unit.status = self._ready_status()
            else:
                self.unit.status = ops.WaitingStatus("waiting for harvest to become ready")
        except Blocked as e:
            self.unit.status = ops.BlockedStatus(str(e))
        except Waiting as e:
            self.unit.status = ops.WaitingStatus(str(e))
        except WorkloadError as e:
            logger.error("workload error: %s", e)
            self.unit.status = ops.BlockedStatus(f"workload error: {e}")

    def _restart_is_safe(self, cluster: dict) -> bool:
        if self.config.get("cluster-backend", "postgres") != "quorium" or not self.workload.running():
            return True
        try:
            status = self._local_admin(cluster).status()
        except WorkloadError as e:
            logger.warning("cannot read cluster status before restarting: %s", e)
            return False
        return bool(status.get("membership", {}).get("quorum")) and restart_is_safe(status, self._internal_url())

    def _restart(self, _event: ops.EventBase) -> None:
        cluster = self._cluster_secrets()
        if not self._restart_is_safe(cluster):
            self.unit.status = ops.WaitingStatus("waiting for a quorum-safe moment to restart")
            return
        self.unit.status = ops.MaintenanceStatus("restarting harvest")
        self.workload.restart()
        if not self.workload.wait_ready(READY_TIMEOUT):
            self.unit.status = ops.WaitingStatus("harvest did not become ready after restart")
            return
        self._stored.restart_pending = False
        self._publish_running_secrets(cluster)
        self.unit.status = self._ready_status()

    def _ready_status(self) -> ops.ActiveStatus:
        node = self.workload.node_id()
        return ops.ActiveStatus(f"ready ({node})" if node else "ready")

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

    def _peer_units(self) -> list[str]:
        relation = self._peer
        if relation is None:
            return []
        return sorted(unit.name for unit in relation.units)

    def _evacuated_units(self) -> set[str]:
        relation = self._peer
        if relation is None:
            return set()
        raw = relation.data[self.app].get(EVACUATED_KEY, "")
        return {name for name in raw.split(",") if name}

    def _set_evacuated_units(self, units: set[str]) -> None:
        relation = self._peer
        if relation is None:
            return
        relation.data[self.app][EVACUATED_KEY] = ",".join(sorted(units))

    def _admin_status(self) -> Optional[dict]:
        try:
            return self._local_admin(self._cluster_secrets()).status()
        except (WorkloadError, Waiting, KeyError) as e:
            logger.info("cluster status is unavailable: %s", e)
            return None

    def _on_cluster_status(self, event: ops.ActionEvent) -> None:
        peer = self._peer
        event.set_results({
            "managed": True,
            "backend": str(self.config.get("cluster-backend", "postgres")),
            "local-unit": self.unit.name,
            "node-id": self.workload.node_id() or "",
            "seed": (peer.data[self.app].get(SEED_KEY, "") if peer else ""),
            "peer-units": json.dumps(self._peer_units()),
            "evacuated-units": json.dumps(sorted(self._evacuated_units())),
            "active-active": bool(self.config.get("active-active", True)),
            "cluster": json.dumps(self._admin_status() or {}),
        })

    def _on_cluster_rebalance(self, event: ops.ActionEvent) -> None:
        self._reconcile(event)
        event.set_results({"status": "reconciled"})

    def _target_unit(self, event: ops.ActionEvent, action: str) -> Optional[tuple]:
        if not self.unit.is_leader():
            event.fail(f"{action} must run on the leader")
            return None
        target = str(event.params.get("unit") or "").strip()
        if not target:
            event.fail("unit is required")
            return None
        return target, self._peer_addresses().get(target)

    def _remote_admin(self, address: str) -> ClusterAdmin:
        return ClusterAdmin(address, rendered_secrets(self._cluster_secrets())[0])

    def _on_cluster_evacuate(self, event: ops.ActionEvent) -> None:
        resolved = self._target_unit(event, "cluster-evacuate")
        if resolved is None:
            return
        target, address = resolved
        if address and self.workload.running():
            try:
                self._remote_admin(address).evacuate()
            except WorkloadError as e:
                event.fail(f"could not evacuate {target}: {e}")
                return
        units = self._evacuated_units()
        units.add(target)
        self._set_evacuated_units(units)
        event.set_results({"evacuated": target})

    def _on_cluster_restore(self, event: ops.ActionEvent) -> None:
        resolved = self._target_unit(event, "cluster-restore")
        if resolved is None:
            return
        target, address = resolved
        if address and self.workload.running():
            try:
                self._remote_admin(address).restore()
            except WorkloadError as e:
                event.fail(f"could not restore {target}: {e}")
                return
        units = self._evacuated_units()
        units.discard(target)
        self._set_evacuated_units(units)
        event.set_results({"restored": target})

    def _on_cluster_remove(self, event: ops.ActionEvent) -> None:
        resolved = self._target_unit(event, "cluster-remove")
        if resolved is None:
            return
        target, address = resolved
        if address and self.config.get("cluster-backend") == "quorium" and self.workload.running():
            try:
                self._local_admin(self._cluster_secrets()).remove_server(address)
            except WorkloadError as e:
                event.fail(f"could not remove {target}: {e}")
                return
        units = self._evacuated_units()
        units.discard(target)
        self._set_evacuated_units(units)
        event.set_results({"removed": target})

    def _on_cluster_rotate_secrets(self, event: ops.ActionEvent) -> None:
        if not self.unit.is_leader():
            event.fail("cluster-rotate-secrets must run on the leader")
            return
        secret = self._secret_handle()
        if secret is None:
            event.fail("cluster secret is not initialized yet")
            return
        try:
            staged = stage_rotation(secret.get_content(refresh=True), secrets.token_urlsafe(32))
        except RotationInProgress as e:
            event.fail(str(e))
            return
        secret.set_content(staged)
        self._peer.data[self.app][PHASE] = staged[PHASE]
        event.set_results({"rotated": True, "phase": staged[PHASE]})

if __name__ == "__main__":
    ops.main(HarvestServerCharm)
