#!/usr/bin/env python3
import logging
from pathlib import Path
from typing import Optional

import ops
from charms.haproxy.v2.haproxy_route import HaproxyRouteRequirer, LoadBalancingAlgorithm
from charms.harvest_server.v0.harvest_api import HarvestApiRequirer

from harvest_workload import Workload

logger = logging.getLogger(__name__)


class HarvestWebUiCharm(ops.CharmBase):
    _stored = ops.StoredState()

    def __init__(self, framework: ops.Framework):
        super().__init__(framework)
        self._stored.set_default(applied={})
        self.port = int(self.config.get("port", 8080))
        self.workload = Workload(service="ui", port=self.port, ready_path="/ui-health")
        self.api = HarvestApiRequirer(self, "harvest-api")
        self.route = HaproxyRouteRequirer(
            self,
            "haproxy-route",
            service="harvest-web-ui",
            ports=[self.port],
            hostname=self.config.get("hostname") or None,
            check_path="/ui-health",
            check_interval=5,
            check_rise=2,
            check_fall=3,
            load_balancing_algorithm=LoadBalancingAlgorithm.ROUNDROBIN,
            retry_count=2,
            retry_redispatch=True,
        )
        for event in (
            self.on.install,
            self.on.upgrade_charm,
            self.on.config_changed,
            self.on.update_status,
            self.on["harvest-api"].relation_changed,
            self.on["harvest-api"].relation_broken,
            self.route.on.ready,
            self.route.on.removed,
        ):
            framework.observe(event, self._reconcile)
        framework.observe(self.on.stop, self._on_stop)

    def _resource(self) -> Optional[Path]:
        try:
            return Path(self.model.resources.fetch("harvest-snap"))
        except (ops.ModelError, NameError):
            return None

    def _reconcile(self, _event: ops.EventBase) -> None:
        if not self.workload.installed():
            self.unit.status = ops.MaintenanceStatus("installing the harvest snap")
            self.workload.install(self.config.get("snap-channel", "latest/edge"), self._resource())
        desired = {
            "config.managed": "true",
            "ui.mode": "static",
            "ui.host": "0.0.0.0",
            "ui.port": str(self.port),
        }
        changed = dict(self._stored.applied) != desired
        if changed:
            self.workload.set_snap_config(desired)
            self.workload.stop(["server"])
        self.unit.open_port("tcp", self.port)

        if not self.workload.running():
            self.workload.start()
        elif changed:
            self.workload.restart()
        self._stored.applied = desired

        if self.model.get_relation("haproxy-route"):
            try:
                self.route.update_relation_data()
            except Exception as e:
                logger.warning("could not publish haproxy-route data: %s", e)

        server = self.api.get()
        version = self.workload.version()
        if server is None:
            self.unit.status = ops.BlockedStatus("requires a harvest-api relation to harvest-server")
            return
        if server.version and version and server.version != version:
            self.unit.status = ops.WaitingStatus(f"harvest-server runs {server.version}, this unit serves {version}; refresh to match")
            return
        if not self.workload.ready():
            self.unit.status = ops.WaitingStatus("waiting for the web UI to respond")
            return
        self.unit.status = ops.ActiveStatus(f"serving {version}")

    def _on_stop(self, _event: ops.StopEvent) -> None:
        try:
            self.workload.stop(["ui"])
        except Exception as e:
            logger.warning("could not stop the web UI: %s", e)


if __name__ == "__main__":
    ops.main(HarvestWebUiCharm)
