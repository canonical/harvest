import json
import tomllib
from pathlib import Path

import pytest
from ops import testing

import charm as charm_module
from charm import HarvestServerCharm
from harvest_workload import WorkloadError

CHARM_ROOT = Path(__file__).resolve().parents[2]
LLM = '[[llm]]\nprovider = "openai-compatible"\nbase_url = "https://llm.example.com/v1"\nmodel = "m"\napi_key = "${secret:key}"\n'


class FakeWorkload:
    def __init__(self, service, port, ready_path):
        self.files = {}
        self.calls = []
        self.is_installed = True
        self.is_running = False
        self.is_ready = True
        self.schema = (0, 7)
        self.check_error = None

    def installed(self):
        return self.is_installed

    def install(self, channel, resource):
        self.calls.append(("install", channel))
        self.is_installed = True

    def set_snap_config(self, values):
        self.calls.append(("snap-config", values))

    def version(self):
        return "0.11.0"

    def write_file(self, path, content, mode=0o600):
        changed = self.files.get(str(path)) != content
        self.files[str(path)] = content
        return changed

    def remove_file(self, path):
        self.files.pop(str(path), None)

    def check_config(self):
        if self.check_error:
            raise WorkloadError(self.check_error)

    def schema_versions(self):
        return self.schema

    def migrate(self):
        self.calls.append(("migrate",))
        self.schema = (self.schema[1], self.schema[1])
        return self.schema[0]

    def start(self):
        self.calls.append(("start",))
        self.is_running = True

    def stop(self, services):
        self.calls.append(("stop", tuple(services)))

    def restart(self):
        self.calls.append(("restart",))

    def running(self):
        return self.is_running

    def ready(self):
        return self.is_ready

    def wait_ready(self, timeout):
        return self.is_ready

    def node_id(self):
        return "harvest-server-0-abc"


@pytest.fixture
def workload(monkeypatch):
    holder = {}

    def factory(*args, **kwargs):
        holder["w"] = FakeWorkload(*args, **kwargs)
        return holder["w"]

    monkeypatch.setattr(charm_module, "Workload", factory)
    return holder


@pytest.fixture
def ctx():
    return testing.Context(HarvestServerCharm, charm_root=CHARM_ROOT)


def database(endpoints="10.0.0.1:5432", read_only="10.0.0.2:5432", tls=None):
    data = {
        "database": "harvest",
        "endpoints": endpoints,
        "read-only-endpoints": read_only,
        "username": "relation-5",
        "password": "pw",
    }
    if tls:
        data.update({"tls": "True", "tls-ca": tls})
    return testing.Relation(endpoint="database", interface="postgresql_client", remote_app_name="postgresql", remote_app_data=data)


def llm_secret():
    return testing.Secret(tracked_content={"key": "sk-123"}, owner=None)


def base_state(*, leader=True, relations=(), config=None, secrets=(), planned_units=1):
    secret = llm_secret()
    cfg = {"llm-providers": LLM, "llm-secret": secret.id}
    cfg.update(config or {})
    return testing.State(
        leader=leader,
        config=cfg,
        relations=[testing.PeerRelation(endpoint="harvest-peers"), testing.PeerRelation(endpoint="restart"), *relations],
        secrets=[secret, *secrets],
        planned_units=planned_units,
    )


def server_toml(workload):
    return tomllib.loads(workload["w"].files["/var/snap/harvest/common/server.toml"])


def test_blocks_without_a_database_relation(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state())
    assert out.unit_status == testing.BlockedStatus("requires a database relation (postgresql)")


def test_blocks_without_llm_providers(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()], config={"llm-providers": ""}))
    assert isinstance(out.unit_status, testing.BlockedStatus)
    assert "llm-providers" in out.unit_status.message


def test_leader_generates_cluster_secrets_once(ctx, workload):
    out = ctx.run(ctx.on.leader_elected(), base_state(relations=[database()]))
    peer = out.get_relations("harvest-peers")[0]
    secret_id = peer.local_app_data["cluster-secret-id"]
    secret = out.get_secret(id=secret_id)
    content = secret.latest_content
    assert len(content["user-key-encryption-key"]) == 64
    assert content["jwt-secret"] and content["cluster-secret"]


def test_follower_waits_for_cluster_secrets(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state(leader=False, relations=[database()]))
    assert out.unit_status == testing.WaitingStatus("waiting for the leader to create cluster secrets")


def test_imported_secrets_preserve_an_existing_deployment(ctx, workload):
    imported = testing.Secret(tracked_content={"jwt-secret": "old-jwt", "user-key-encryption-key": "ab" * 32}, owner=None)
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()], config={"import-secrets": imported.id}, secrets=[imported]))
    peer = out.get_relations("harvest-peers")[0]
    content = out.get_secret(id=peer.local_app_data["cluster-secret-id"]).latest_content
    assert content["jwt-secret"] == "old-jwt"
    assert content["user-key-encryption-key"] == "ab" * 32
    assert server_toml(workload)["auth"]["jwt_secret"] == "old-jwt"


def test_renders_a_multi_host_read_write_url_and_starts_the_server(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()]))
    toml = server_toml(workload)
    assert toml["database"]["url"].startswith("postgres://relation-5:pw@10.0.0.1:5432,10.0.0.2:5432/harvest?")
    assert "target_session_attrs=read-write" in toml["database"]["url"]
    assert toml["database"]["migrate_on_start"] is False
    assert toml["cluster"]["shared_secret"]
    assert toml["cluster"]["internal_url"].endswith(":8081")
    assert toml["llm"][0]["api_key"] == "sk-123"
    assert ("start",) in workload["w"].calls
    assert out.unit_status == testing.ActiveStatus("ready (harvest-server-0-abc)")
    assert testing.TCPPort(8080) in out.opened_ports and testing.TCPPort(8081) in out.opened_ports


def test_tls_database_writes_the_ca_and_requires_ssl(ctx, workload):
    ctx.run(ctx.on.config_changed(), base_state(relations=[database(tls="-----BEGIN CERTIFICATE-----\nX\n-----END CERTIFICATE-----")]))
    toml = server_toml(workload)
    assert "sslmode=require" in toml["database"]["url"]
    assert toml["database"]["ca_file"] == "/var/snap/harvest/common/db-ca.pem"
    assert "BEGIN CERTIFICATE" in workload["w"].files["/var/snap/harvest/common/db-ca.pem"]


def test_only_the_leader_runs_migrations(ctx, workload):
    ctx.run(ctx.on.config_changed(), base_state(relations=[database()]))
    assert ("migrate",) in workload["w"].calls

    peer = testing.PeerRelation(endpoint="harvest-peers")
    state = base_state(relations=[database()])
    first = ctx.run(ctx.on.leader_elected(), state)
    secret_id = first.get_relations("harvest-peers")[0].local_app_data["cluster-secret-id"]
    follower_state = testing.State(
        leader=False,
        config=state.config,
        relations=[
            testing.PeerRelation(endpoint="harvest-peers", local_app_data={"cluster-secret-id": secret_id}),
            testing.PeerRelation(endpoint="restart"),
            database(),
        ],
        secrets=list(first.secrets),
    )
    ctx.run(ctx.on.config_changed(), follower_state)
    assert ("migrate",) not in workload["w"].calls
    del peer


def test_invalid_configuration_blocks_and_does_not_start(ctx, workload, monkeypatch):
    original = FakeWorkload.__init__

    def failing(self, *args, **kwargs):
        original(self, *args, **kwargs)
        self.check_error = "parsing config TOML: bad"

    monkeypatch.setattr(FakeWorkload, "__init__", failing)
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()]))
    assert out.unit_status == testing.BlockedStatus("invalid configuration: parsing config TOML: bad")
    assert ("start",) not in workload["w"].calls


def test_single_unit_mode_blocks_when_scaled_out(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()], config={"active-active": False}, planned_units=3))
    assert out.unit_status == testing.BlockedStatus("active-active is false: scale to one unit")


def test_active_active_allows_several_units(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()], planned_units=3))
    assert isinstance(out.unit_status, testing.ActiveStatus)


def test_haproxy_route_requests_api_paths_health_checks_and_long_timeouts(ctx, workload):
    route = testing.Relation(endpoint="haproxy-route", interface="haproxy-route", remote_app_name="haproxy")
    out = ctx.run(ctx.on.relation_changed(route), base_state(relations=[database(), route]))
    data = out.get_relation(route.id).local_app_data
    assert json.loads(data["service"]) == "harvest-server"
    assert json.loads(data["ports"]) == [8080]
    paths = json.loads(data["paths"])
    assert "/projects" in paths and "/agent" in paths and "/health" in paths
    check = json.loads(data["check"])
    assert check["path"] == "/health/ready"
    assert json.loads(data["timeout"])["server"] == 3600
    assert json.loads(data.get("load_balancing", '{"algorithm": "leastconn"}'))["algorithm"] == "leastconn"
    assert json.loads(data["retry"]) == {"count": 2, "redispatch": True}
    unit_data = out.get_relation(route.id).local_unit_data
    assert "address" in unit_data


def test_public_url_from_haproxy_drives_oauth_redirects(ctx, workload):
    route = testing.Relation(
        endpoint="haproxy-route", interface="haproxy-route", remote_app_name="haproxy",
        remote_app_data={"endpoints": json.dumps(["https://harvest.example.com/"])},
    )
    oidc = testing.Secret(tracked_content={"client-secret": "oidc-s"}, owner=None)
    ctx.run(ctx.on.config_changed(), base_state(
        relations=[database(), route],
        config={"oidc-issuer-url": "https://login.example.com", "oidc-client-id": "harvest", "oidc-secret": oidc.id},
        secrets=[oidc],
    ))
    toml = server_toml(workload)
    assert toml["auth"]["public_url"] == "https://harvest.example.com"
    assert toml["auth"]["oidc"]["redirect_uri"] == "https://harvest.example.com/auth/oidc/callback"
    assert toml["auth"]["oidc"]["client_secret"] == "oidc-s"
    assert toml["agents"]["public_url"] == "https://harvest.example.com"


def test_harvest_api_publishes_version_schema_and_paths(ctx, workload):
    api = testing.Relation(endpoint="harvest-api", interface="harvest_api", remote_app_name="harvest-web-ui")
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database(), api]))
    data = out.get_relation(api.id).local_app_data
    assert data["version"] == "0.11.0"
    assert data["schema-version"] == "7"
    assert "/projects" in json.loads(data["api-paths"])


def test_a_configuration_change_on_a_running_unit_requests_a_rolling_restart(ctx, workload, monkeypatch):
    original = FakeWorkload.__init__

    def running(self, *args, **kwargs):
        original(self, *args, **kwargs)
        self.is_running = True

    monkeypatch.setattr(FakeWorkload, "__init__", running)
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()]))
    assert ("start",) not in workload["w"].calls
    assert ("restart",) in workload["w"].calls, "the leader grants the rolling-restart lock and the callback restarts the unit"


def test_the_rolling_restart_callback_restarts_and_waits_for_readiness(ctx, workload):
    charm = None
    with ctx(ctx.on.config_changed(), base_state(relations=[database()])) as manager:
        charm = manager.charm
        manager.run()
        charm._restart(None)
    assert ("restart",) in workload["w"].calls


def test_install_stops_the_ui_service_and_marks_config_managed(ctx, workload, monkeypatch, tmp_path):
    original = FakeWorkload.__init__

    def fresh(self, *args, **kwargs):
        original(self, *args, **kwargs)
        self.is_installed = False

    monkeypatch.setattr(FakeWorkload, "__init__", fresh)
    empty = tmp_path / "harvest.snap"
    empty.write_bytes(b"")
    state = base_state(relations=[database()])
    state = testing.State(
        leader=state.leader, config=state.config, relations=list(state.relations), secrets=list(state.secrets),
        resources=[testing.Resource(name="harvest-snap", path=empty)],
    )
    ctx.run(ctx.on.install(), state)
    calls = workload["w"].calls
    assert ("install", "latest/edge") in calls
    assert ("snap-config", {"config.managed": "true", "ui.mode": "static"}) in calls
    assert ("stop", ("ui",)) in calls


def test_missing_llm_secret_key_blocks(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), base_state(relations=[database()], config={"llm-providers": 'x = "${secret:nope}"'}))
    assert out.unit_status == testing.BlockedStatus("llm-providers references nope which llm-secret does not contain")
