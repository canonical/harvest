#!/usr/bin/env python3
import ops


class HarvestLlmProviderOpenaiCompatibleCharm(ops.CharmBase):
    def __init__(self, framework: ops.Framework):
        super().__init__(framework)
        for event in (
            self.on.config_changed,
            self.on.secret_changed,
            self.on["llm-provider"].relation_joined,
            self.on["llm-provider"].relation_changed,
            self.on["llm-provider"].relation_departed,
            self.on["llm-provider"].relation_broken,
        ):
            framework.observe(event, self._reconcile)

    def _api_key_secret(self):
        secret_id = (self.config.get("api-key-secret") or "").strip()
        if not secret_id:
            return None
        try:
            return self.model.get_secret(id=secret_id)
        except (ops.SecretNotFoundError, ops.ModelError):
            self.unit.status = ops.BlockedStatus("cannot read api-key-secret")
            return None

    def _reconcile(self, _event: ops.EventBase):
        enabled = bool(self.config.get("enabled", True))
        model = (self.config.get("model") or "").strip()
        base_url = (self.config.get("base-url") or "").strip()
        if enabled and not model:
            self.unit.status = ops.BlockedStatus("set model")
            return
        if enabled and not base_url:
            self.unit.status = ops.BlockedStatus("set base-url")
            return
        api_key_secret = self._api_key_secret()
        if (self.config.get("api-key-secret") or "").strip() and api_key_secret is None:
            return
        timeout_secs = int(self.config.get("timeout-secs", 120))
        max_retries = int(self.config.get("max-retries", 3))
        relations = self.model.relations.get("llm-provider", [])
        for relation in relations:
            if api_key_secret is not None:
                api_key_secret.grant(relation)
            provider_id = self.app.name
            display_name = self.app.name.replace("-", " ").title()
            payload = {
                "provider": "openai-compatible",
                "id": provider_id,
                "name": display_name,
                "enabled": str(enabled).lower(),
                "priority": str(int(self.config.get("priority", 100))),
                "model": model,
                "base_url": base_url,
                "user_provided_key": str(bool(self.config.get("user-provided-key", False))).lower(),
                "timeout_secs": str(timeout_secs),
                "max_retries": str(max_retries),
            }
            if api_key_secret is not None:
                payload["secret_id"] = api_key_secret.id
            relation.data[self.app].update(payload)
        self.unit.status = ops.ActiveStatus(f"published to {len(relations)} relation(s)")


if __name__ == "__main__":
    ops.main(HarvestLlmProviderOpenaiCompatibleCharm)
