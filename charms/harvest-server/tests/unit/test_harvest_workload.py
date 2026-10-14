import pytest

import harvest_workload
from harvest_workload import Workload, WorkloadError


def workload(monkeypatch, tmp_path, installed=True):
    monkeypatch.setattr(harvest_workload, "DIGEST_PATH", tmp_path / "digest")
    monkeypatch.setattr(harvest_workload.snap, "install_local", lambda *args, **kwargs: None)
    w = Workload(service="server", port=8080, ready_path="/health/ready")
    monkeypatch.setattr(w, "installed", lambda: installed)
    monkeypatch.setattr(w, "_snap", lambda: type("S", (), {"hold": lambda self: None})())
    return w


def test_a_missing_snap_needs_installing(monkeypatch, tmp_path):
    assert workload(monkeypatch, tmp_path, installed=False).needs_install(None)


def test_an_installed_store_snap_is_left_alone(monkeypatch, tmp_path):
    assert not workload(monkeypatch, tmp_path).needs_install(None)


def test_a_new_resource_is_installed_once(monkeypatch, tmp_path):
    w = workload(monkeypatch, tmp_path)
    resource = tmp_path / "harvest.snap"
    resource.write_bytes(b"first")
    assert w.needs_install(resource)
    w.install("latest/edge", resource)
    assert not w.needs_install(resource)
    resource.write_bytes(b"second")
    assert w.needs_install(resource)


def test_a_snapd_change_conflict_is_retried(monkeypatch, tmp_path):
    w = workload(monkeypatch, tmp_path)
    monkeypatch.setattr(harvest_workload.time, "sleep", lambda seconds: None)
    attempts = []

    def flaky(*args, **kwargs):
        attempts.append(1)
        if len(attempts) < 3:
            raise harvest_workload.snap.SnapError('snap "snapd" has "auto-refresh" change in progress')

    monkeypatch.setattr(harvest_workload.snap, "install_local", flaky)
    resource = tmp_path / "harvest.snap"
    resource.write_bytes(b"snap")
    w.install("latest/edge", resource)
    assert len(attempts) == 3
    assert not w.needs_install(resource)


def test_a_persistent_snap_failure_becomes_a_workload_error(monkeypatch, tmp_path):
    w = workload(monkeypatch, tmp_path)
    monkeypatch.setattr(harvest_workload.time, "sleep", lambda seconds: None)

    def broken(*args, **kwargs):
        raise harvest_workload.snap.SnapError("cannot install")

    monkeypatch.setattr(harvest_workload.snap, "install_local", broken)
    resource = tmp_path / "harvest.snap"
    resource.write_bytes(b"snap")
    with pytest.raises(WorkloadError):
        w.install("latest/edge", resource)
    assert w.needs_install(resource)
