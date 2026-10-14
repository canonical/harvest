#!/usr/bin/env python3
import ops


class HarvestAuthProviderOidcCharm(ops.CharmBase):
    def __init__(self, framework: ops.Framework):
        super().__init__(framework)
        for event in (
            self.on.config_changed,
            self.on.secret_changed,
            self.on["auth-provider"].relation_joined,
            self.on["auth-provider"].relation_changed,
            self.on["auth-provider"].relation_departed,
            self.on["auth-provider"].relation_broken,
        ):
            framework.observe(event, self._reconcile)

    def _client_secret(self):
        secret_id = (self.config.get("client-secret") or "").strip()
        if not secret_id:
            return None
        try:
            return self.model.get_secret(id=secret_id)
        except (ops.SecretNotFoundError, ops.ModelError):
            self.unit.status = ops.BlockedStatus("cannot read client-secret")
            return None

    def _reconcile(self, _event: ops.EventBase):
        enabled = bool(self.config.get("enabled", True))
        issuer_url = (self.config.get("issuer-url") or "").strip()
        client_id = (self.config.get("client-id") or "").strip()
        if enabled and (not issuer_url or not client_id):
            self.unit.status = ops.BlockedStatus("set issuer-url and client-id")
            return
        client_secret = self._client_secret()
        if (self.config.get("client-secret") or "").strip() and client_secret is None:
            return
        relations = self.model.relations.get("auth-provider", [])
        for relation in relations:
            if client_secret is not None:
                client_secret.grant(relation)
            payload = {
                "provider": "oidc",
                "enabled": str(enabled).lower(),
                "issuer_url": issuer_url,
                "client_id": client_id,
                "display_name": (self.config.get("display-name") or "").strip(),
            }
            if client_secret is not None:
                payload["secret_id"] = client_secret.id
            relation.data[self.app].update(payload)
        self.unit.status = ops.ActiveStatus(f"published to {len(relations)} relation(s)")


if __name__ == "__main__":
    ops.main(HarvestAuthProviderOidcCharm)
