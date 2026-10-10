import json
from pathlib import Path

import pytest
from ops import testing

import charm as charm_module
from charm import HarvestWebUiCharm

CHARM_ROOT = Path(__file__).resolve().parents[2]


class FakeWorkload:
    def __init__(self, service, port, ready_path):
        self.port = port
        self.ready_path = ready_path
        self.calls = []
        self.is_installed = True
        self.is_running = False
        self.is_ready = True
        self.snap_version = "0.11.0"

    def installed(self):
        return self.is_installed

    def install(self, channel, resource):
        self.calls.append(("install", channel))

    def set_snap_config(self, values):
        self.calls.append(("snap-config", values))

    def version(self):
        return self.snap_version

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
    return testing.Context(HarvestWebUiCharm, charm_root=CHARM_ROOT)


def api(version="0.11.0"):
    return testing.Relation(
        endpoint="harvest-api", interface="harvest_api", remote_app_name="harvest-server",
        remote_app_data={"version": version, "schema-version": "7", "public-url": "https://harvest.example.com",
                         "api-paths": json.dumps(["/auth", "/projects"])},
    )


def test_blocks_until_related_to_harvest_server(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), testing.State(leader=True))
    assert out.unit_status == testing.BlockedStatus("requires a harvest-api relation to harvest-server")


def test_serves_static_files_only_on_the_configured_port(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), testing.State(leader=True, relations=[api()]))
    calls = workload["w"].calls
    assert ("snap-config", {"config.managed": "true", "ui.mode": "static", "ui.host": "0.0.0.0", "ui.port": "8080"}) in calls
    assert ("stop", ("server",)) in calls
    assert ("start",) in calls
    assert workload["w"].ready_path == "/ui-health"
    assert testing.TCPPort(8080) in out.opened_ports
    assert out.unit_status == testing.ActiveStatus("serving 0.11.0")


def test_waits_while_the_server_runs_a_different_version(ctx, workload):
    out = ctx.run(ctx.on.config_changed(), testing.State(leader=True, relations=[api(version="0.12.0")]))
    assert out.unit_status == testing.WaitingStatus("harvest-server runs 0.12.0, this unit serves 0.11.0; refresh to match")


def test_haproxy_route_is_a_catch_all_with_ui_health_checks(ctx, workload):
    route = testing.Relation(endpoint="haproxy-route", interface="haproxy-route", remote_app_name="haproxy")
    out = ctx.run(ctx.on.relation_changed(route), testing.State(leader=True, relations=[api(), route], config={"hostname": "harvest.example.com"}))
    data = out.get_relation(route.id).local_app_data
    assert json.loads(data["service"]) == "harvest-web-ui"
    assert json.loads(data["ports"]) == [8080]
    assert json.loads(data.get("paths", "[]")) == []
    assert json.loads(data["check"])["path"] == "/ui-health"
    assert json.loads(data["hostname"]) == "harvest.example.com"


def test_a_port_change_restarts_the_running_ui(ctx, workload, monkeypatch):
    original = FakeWorkload.__init__

    def running(self, *args, **kwargs):
        original(self, *args, **kwargs)
        self.is_running = True

    monkeypatch.setattr(FakeWorkload, "__init__", running)
    applied = {"config.managed": "true", "ui.mode": "static", "ui.host": "0.0.0.0", "ui.port": "8080"}
    stored = testing.StoredState(owner_path="HarvestWebUiCharm", content={"applied": applied})
    ctx.run(ctx.on.update_status(), testing.State(leader=True, relations=[api()], stored_states=[stored]))
    assert ("restart",) not in workload["w"].calls, "an unchanged configuration never restarts the UI"
    ctx.run(ctx.on.config_changed(), testing.State(leader=True, relations=[api()], config={"port": 9090}, stored_states=[stored]))
    assert ("restart",) in workload["w"].calls
    assert ("snap-config", {"config.managed": "true", "ui.mode": "static", "ui.host": "0.0.0.0", "ui.port": "9090"}) in workload["w"].calls
