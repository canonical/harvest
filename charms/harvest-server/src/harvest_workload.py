import hashlib
import json
import logging
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Optional

from charms.operator_libs_linux.v2 import snap

logger = logging.getLogger(__name__)

SNAP_NAME = "harvest"
COMMON = Path("/var/snap/harvest/common")
CONFIG_PATH = COMMON / "server.toml"
CA_PATH = COMMON / "db-ca.pem"
AGENT_BINARY = "/snap/harvest/current/bin/harvest-agent"


class WorkloadError(Exception):
    pass


class Workload:
    def __init__(self, service: str, port: int, ready_path: str):
        self.service = service
        self.port = port
        self.ready_path = ready_path

    def _snap(self) -> snap.Snap:
        return snap.SnapCache()[SNAP_NAME]

    def install(self, channel: str, resource: Optional[Path]) -> None:
        if resource is not None and resource.exists() and resource.stat().st_size > 0:
            snap.install_local(str(resource), dangerous=True)
        else:
            self._snap().ensure(snap.SnapState.Latest, channel=channel)
        self._snap().hold()

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
        self._snap().set({k: str(v) for k, v in values.items()})

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
        self._snap().start([self.service], enable=True)

    def stop(self, services: list) -> None:
        self._snap().stop(services, disable=True)

    def restart(self) -> None:
        self._snap().restart([self.service])

    def running(self) -> bool:
        try:
            services = self._snap().services
        except snap.SnapError:
            return False
        return bool(services.get(self.service, {}).get("active"))

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
