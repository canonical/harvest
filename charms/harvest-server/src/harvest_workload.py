import hashlib
import json
import logging
import subprocess
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Callable, Optional

from charms.operator_libs_linux.v2 import snap

logger = logging.getLogger(__name__)

SNAP_NAME = "harvest"
COMMON = Path("/var/snap/harvest/common")
CONFIG_PATH = COMMON / "server.toml"
CA_PATH = COMMON / "db-ca.pem"
DIGEST_PATH = COMMON / ".charm-resource-digest"
AGENT_BINARY = "/snap/harvest/current/bin/harvest-agent"
SECRET_HEADER = "x-harvest-cluster-secret"
SNAP_ATTEMPTS = 5
SNAP_BACKOFF = 10


class WorkloadError(Exception):
    pass


def _usable(resource: Optional[Path]) -> bool:
    return resource is not None and resource.exists() and resource.stat().st_size > 0


def _digest(resource: Path) -> str:
    return hashlib.sha256(resource.read_bytes()).hexdigest()


def _with_retries(action: Callable[[], None]) -> None:
    for attempt in range(1, SNAP_ATTEMPTS + 1):
        try:
            action()
            return
        except snap.SnapError as e:
            if attempt == SNAP_ATTEMPTS:
                raise WorkloadError(f"snap operation failed: {e}") from e
            logger.warning("snap operation failed (attempt %d of %d): %s", attempt, SNAP_ATTEMPTS, e)
            time.sleep(SNAP_BACKOFF * attempt)


class Workload:
    def __init__(self, service: str, port: int, ready_path: str):
        self.service = service
        self.port = port
        self.ready_path = ready_path

    def _snap(self) -> snap.Snap:
        return snap.SnapCache()[SNAP_NAME]

    def install(self, channel: str, resource: Optional[Path]) -> None:
        if _usable(resource):
            _with_retries(lambda: snap.install_local(str(resource), dangerous=True))
            DIGEST_PATH.write_text(_digest(resource))
        else:
            _with_retries(lambda: self._snap().ensure(snap.SnapState.Latest, channel=channel))
        _with_retries(lambda: self._snap().hold())

    def needs_install(self, resource: Optional[Path]) -> bool:
        if not self.installed():
            return True
        if not _usable(resource):
            return False
        current = DIGEST_PATH.read_text().strip() if DIGEST_PATH.exists() else ""
        return current != _digest(resource)

    def installed(self) -> bool:
        try:
            return self._snap().present
        except snap.SnapError:
            return False

    def version(self) -> str:
        try:
            return self._snap().version or ""
        except snap.SnapError:
            return ""

    def set_snap_config(self, values: dict) -> None:
        _with_retries(lambda: self._snap().set({k: str(v) for k, v in values.items()}))

    def write_file(self, path: Path, content: str, mode: int = 0o600) -> bool:
        current = path.read_text() if path.exists() else None
        if current == content:
            return False
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        path.chmod(mode)
        return True

    def remove_file(self, path: Path) -> None:
        if path.exists():
            path.unlink()

    def config_hash(self) -> str:
        if not CONFIG_PATH.exists():
            return ""
        return hashlib.sha256(CONFIG_PATH.read_bytes()).hexdigest()

    def _admin(self, *args: str) -> str:
        try:
            result = subprocess.run(
                ["snap", "run", f"{SNAP_NAME}.admin", *args],
                check=True,
                capture_output=True,
                text=True,
                timeout=300,
            )
        except subprocess.CalledProcessError as e:
            raise WorkloadError(e.stderr.strip() or str(e)) from e
        except subprocess.TimeoutExpired as e:
            raise WorkloadError(f"timed out running {args}") from e
        return result.stdout.strip()

    def check_config(self) -> None:
        self._admin("check-config")

    def migrate(self) -> int:
        return int(self._admin("migrate").splitlines()[-1])

    def schema_versions(self) -> tuple:
        database, binary = self._admin("schema-version").split()
        return int(database), int(binary)

    def start(self) -> None:
        _with_retries(lambda: self._snap().start([self.service], enable=True))

    def stop(self, services: list) -> None:
        _with_retries(lambda: self._snap().stop(services, disable=True))

    def restart(self) -> None:
        _with_retries(lambda: self._snap().restart([self.service]))

    def running(self) -> bool:
        try:
            services = self._snap().services
        except snap.SnapError:
            return False
        return bool(services.get(self.service, {}).get("active"))

    def enabled(self) -> bool:
        try:
            services = self._snap().services
        except snap.SnapError:
            return False
        return bool(services.get(self.service, {}).get("enabled"))

    def ready(self) -> bool:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{self.port}{self.ready_path}", timeout=3) as response:
                return response.status == 200
        except (urllib.error.URLError, OSError, ValueError):
            return False

    def wait_ready(self, timeout: float) -> bool:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.ready():
                return True
            time.sleep(2)
        return False

    def node_id(self) -> Optional[str]:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/version", timeout=3) as response:
                return json.loads(response.read()).get("node_id")
        except (urllib.error.URLError, OSError, ValueError):
            return None


class ClusterAdmin:
    def __init__(self, base_url: str, secret: str, timeout: float = 10):
        self.base_url = base_url.rstrip("/")
        self.secret = secret
        self.timeout = timeout

    def call(self, method: str, path: str, body: Optional[dict] = None) -> dict:
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(f"{self.base_url}{path}", data=data, method=method)
        request.add_header(SECRET_HEADER, self.secret)
        if data is not None:
            request.add_header("content-type", "application/json")
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                raw = response.read()
        except urllib.error.HTTPError as e:
            detail = e.read().decode(errors="replace")
            raise WorkloadError(f"{method} {path} failed with {e.code}: {detail}") from e
        except (urllib.error.URLError, OSError) as e:
            raise WorkloadError(f"{method} {path} failed: {e}") from e
        return json.loads(raw) if raw else {}

    def status(self) -> dict:
        return self.call("GET", "/internal/cluster/status")

    def has_quorum(self) -> bool:
        try:
            return bool(self.status().get("membership", {}).get("quorum"))
        except WorkloadError:
            return False

    def servers(self) -> list:
        return (self.status().get("quorium") or {}).get("servers") or []

    def remove_server(self, address: str) -> None:
        self.call("DELETE", f"/internal/cluster/servers/{urllib.parse.quote(address, safe='')}")

    def evacuate(self) -> dict:
        return self.call("POST", "/internal/cluster/evacuate")

    def restore(self) -> dict:
        return self.call("POST", "/internal/cluster/restore")

