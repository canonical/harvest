import json
import ssl
import time
import urllib.request

import jubilant
import pytest

HOSTNAME = "harvest.test"


def haproxy_address(juju: jubilant.Juju) -> str:
    status = juju.status()
    unit = next(iter(status.apps["haproxy"].units.values()))
    return unit.public_address


def get(juju: jubilant.Juju, path: str, timeout: float = 10.0) -> tuple:
    context = ssl.create_default_context()
    context.check_hostname = False
    context.verify_mode = ssl.CERT_NONE
    request = urllib.request.Request(f"https://{haproxy_address(juju)}{path}", headers={"Host": HOSTNAME})
    try:
        with urllib.request.urlopen(request, timeout=timeout, context=context) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def cluster_report(juju: jubilant.Juju, unit: str = "harvest-server/leader") -> dict:
    return json.loads(juju.run(unit, "cluster-status").results["cluster"])


def wait_for_quorum(juju: jubilant.Juju, voters: int, timeout: float = 300) -> dict:
    deadline = time.time() + timeout
    while True:
        report = cluster_report(juju)
        membership = report.get("membership", {})
        if membership.get("quorum") and membership.get("voters") == voters:
            return report
        assert time.time() < deadline, f"no quorum of {voters} voters: {membership}"
        time.sleep(5)


def test_deploy_the_ha_topology(juju: jubilant.Juju, server_charm, ui_charm, llm_openai_compatible_charm, harvest_snap,
                                cluster_backend):
    juju.deploy("postgresql", channel="16/stable", num_units=3,
                config={"plugin_pg_trgm_enable": True, "plugin_vector_enable": True})
    juju.deploy("self-signed-certificates")
    juju.deploy("haproxy", channel="2.8/stable", config={"external-hostname": HOSTNAME})
    juju.integrate("haproxy:certificates", "self-signed-certificates:certificates")
    juju.deploy(server_charm, "harvest-server", num_units=3, resources={"harvest-snap": str(harvest_snap)},
                config={"hostname": HOSTNAME, "cluster-backend": cluster_backend})
    juju.deploy(llm_openai_compatible_charm, "harvest-llm-provider-openai-compatible", config={
        "model": "m",
        "base-url": "http://127.0.0.1:9/v1",
        "user-provided-key": True,
    })
    juju.deploy(ui_charm, "harvest-web-ui", num_units=2, resources={"harvest-snap": str(harvest_snap)},
                config={"hostname": HOSTNAME})
    juju.integrate("harvest-server:database", "postgresql:database")
    juju.integrate("harvest-server:haproxy-route", "haproxy:haproxy-route")
    juju.integrate("harvest-llm-provider-openai-compatible:llm-provider", "harvest-server:llm-provider")
    juju.integrate("harvest-web-ui:haproxy-route", "haproxy:haproxy-route")
    juju.integrate("harvest-web-ui:harvest-api", "harvest-server:harvest-api")
    juju.wait(jubilant.all_active, timeout=3600)


def test_haproxy_routes_api_paths_to_servers_and_the_rest_to_the_ui(juju: jubilant.Juju):
    status, body = get(juju, "/health/ready")
    assert status == 200 and json.loads(body)["status"] == "ready"
    status, body = get(juju, "/")
    assert status == 200 and b"<div id=\"app\">" in body
    status, body = get(juju, "/version")
    assert json.loads(body)["schema_version"] == json.loads(body)["binary_schema"]


def test_every_server_unit_is_registered_in_the_cluster(juju: jubilant.Juju):
    seen = set()
    for _ in range(60):
        status, body = get(juju, "/health/ready")
        assert status == 200
        seen.add(json.loads(body)["node_id"])
        if len(seen) == 3:
            break
    assert len(seen) == 3, f"haproxy spreads requests over every unit, saw {seen}"


def test_losing_a_server_unit_keeps_the_service_available(juju: jubilant.Juju):
    juju.exec("sudo snap stop harvest.server", unit="harvest-server/0")
    time.sleep(20)
    failures = sum(1 for _ in range(50) if get(juju, "/health/ready")[0] != 200)
    assert failures == 0
    juju.exec("sudo snap start harvest.server", unit="harvest-server/0")
    juju.wait(jubilant.all_active)


def test_a_postgresql_switchover_needs_no_restart(juju: jubilant.Juju):
    status = juju.status()
    replica = next(name for name, unit in status.apps["postgresql"].units.items() if "Primary" not in unit.workload_status.message)
    juju.run(replica, "promote-to-primary", {"scope": "unit"})
    deadline = time.time() + 300
    while time.time() < deadline:
        if get(juju, "/health/ready")[0] == 200:
            break
        time.sleep(5)
    failures = sum(1 for _ in range(30) if get(juju, "/health/ready")[0] != 200)
    assert failures == 0


def test_a_configuration_change_rolls_through_units_without_downtime(juju: jubilant.Juju):
    juju.config("harvest-server", {"max-iterations": 12})
    deadline = time.time() + 600
    failures = 0
    while time.time() < deadline:
        failures += get(juju, "/health/ready")[0] != 200
        if jubilant.all_active(juju.status()):
            break
        time.sleep(2)
    assert failures == 0


@pytest.mark.usefixtures("quorium_only")
def test_every_server_unit_is_a_raft_voter(juju: jubilant.Juju):
    report = wait_for_quorum(juju, voters=3)
    servers = report["quorium"]["servers"]
    assert len(servers) == 3 and all(server["voter"] for server in servers)


@pytest.mark.usefixtures("quorium_only")
def test_losing_the_raft_leader_keeps_quorum(juju: jubilant.Juju):
    leader = cluster_report(juju)["quorium"]["leader"]
    unit = next(name for name in juju.status().apps["harvest-server"].units
                if cluster_report(juju, name)["quorium"].get("server_id") == leader)
    juju.exec("sudo snap stop harvest.server", unit=unit)
    time.sleep(20)
    failures = sum(1 for _ in range(50) if get(juju, "/health/ready")[0] != 200)
    assert failures == 0
    juju.exec("sudo snap start harvest.server", unit=unit)
    wait_for_quorum(juju, voters=3)
    juju.wait(jubilant.all_active)


@pytest.mark.usefixtures("quorium_only")
def test_removing_a_unit_removes_its_raft_server(juju: jubilant.Juju):
    juju.add_unit("harvest-server")
    juju.wait(jubilant.all_active)
    newest = max(juju.status().apps["harvest-server"].units, key=lambda name: int(name.split("/")[1]))
    juju.remove_unit(newest)
    juju.wait(lambda status: len(status.apps["harvest-server"].units) == 3 and jubilant.all_active(status))
    report = wait_for_quorum(juju, voters=3)
    assert len(report["quorium"]["servers"]) == 3
