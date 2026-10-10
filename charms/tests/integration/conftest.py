import os
from pathlib import Path

import jubilant
import pytest


def pytest_addoption(parser):
    parser.addoption("--server-charm", default=os.environ.get("HARVEST_SERVER_CHARM"))
    parser.addoption("--ui-charm", default=os.environ.get("HARVEST_UI_CHARM"))
    parser.addoption("--harvest-snap", default=os.environ.get("HARVEST_SNAP"))
    parser.addoption("--keep-models", action="store_true", default=False)


@pytest.fixture(scope="session")
def server_charm(request):
    return Path(request.config.getoption("--server-charm")).resolve()


@pytest.fixture(scope="session")
def ui_charm(request):
    return Path(request.config.getoption("--ui-charm")).resolve()


@pytest.fixture(scope="session")
def harvest_snap(request):
    return Path(request.config.getoption("--harvest-snap")).resolve()


@pytest.fixture(scope="module")
def juju(request):
    keep = request.config.getoption("--keep-models")
    with jubilant.temp_model(keep=keep) as juju:
        juju.wait_timeout = 3600
        yield juju
