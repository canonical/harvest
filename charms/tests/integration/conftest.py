import os
from pathlib import Path

import jubilant
import pytest

CHARMS = Path(__file__).resolve().parents[2]


def pytest_addoption(parser):
    parser.addoption("--server-charm", default=os.environ.get("HARVEST_SERVER_CHARM"))
    parser.addoption("--ui-charm", default=os.environ.get("HARVEST_UI_CHARM"))
    parser.addoption("--llm-openai-compatible-charm", default=os.environ.get("HARVEST_LLM_OPENAI_COMPATIBLE_CHARM"))
    parser.addoption("--harvest-snap", default=os.environ.get("HARVEST_SNAP"))
    parser.addoption("--keep-models", action="store_true", default=False)
    parser.addoption("--cluster-backend", choices=["postgres", "quorium"],
                     default=os.environ.get("HARVEST_CLUSTER_BACKEND", "postgres"))


@pytest.fixture(scope="session")
def server_charm(request):
    return Path(request.config.getoption("--server-charm")).resolve()


@pytest.fixture(scope="session")
def ui_charm(request):
    return Path(request.config.getoption("--ui-charm")).resolve()


@pytest.fixture(scope="session")
def llm_openai_compatible_charm(request):
    return Path(request.config.getoption("--llm-openai-compatible-charm")).resolve()


@pytest.fixture(scope="session")
def harvest_snap(request):
    return Path(request.config.getoption("--harvest-snap")).resolve()


@pytest.fixture(scope="session")
def cluster_backend(request):
    return request.config.getoption("--cluster-backend")


@pytest.fixture
def quorium_only(cluster_backend):
    if cluster_backend != "quorium":
        pytest.skip("needs --cluster-backend=quorium")


@pytest.fixture(scope="module")
def juju(request):
    keep = request.config.getoption("--keep-models")
    with jubilant.temp_model(keep=keep) as juju:
        juju.wait_timeout = 3600
        juju.model_config({
            "enable-os-upgrade": False,
            "cloudinit-userdata": (CHARMS / "cloudinit-userdata.yaml").read_text(),
        })
        yield juju
