import json
from dataclasses import dataclass
from typing import Optional

from ops import CharmBase, Object

LIBID = "a7c2f6e1d9b84a1f8c3e5d7b9a1c2e3f"
LIBAPI = 0
LIBPATCH = 1


@dataclass(frozen=True)
class HarvestApi:
    version: str
    schema_version: int
    public_url: str
    api_paths: list


class HarvestApiProvider(Object):
    def __init__(self, charm: CharmBase, relation_name: str = "harvest-api"):
        super().__init__(charm, relation_name)
        self.charm = charm
        self.relation_name = relation_name

    def publish(self, api: HarvestApi) -> None:
        if not self.charm.unit.is_leader():
            return
        for relation in self.model.relations[self.relation_name]:
            relation.data[self.charm.app].update({
                "version": api.version,
                "schema-version": str(api.schema_version),
                "public-url": api.public_url,
                "api-paths": json.dumps(api.api_paths),
            })


class HarvestApiRequirer(Object):
    def __init__(self, charm: CharmBase, relation_name: str = "harvest-api"):
        super().__init__(charm, relation_name)
        self.charm = charm
        self.relation_name = relation_name

    def get(self) -> Optional[HarvestApi]:
        relation = self.model.get_relation(self.relation_name)
        if relation is None or relation.app is None:
            return None
        data = relation.data[relation.app]
        if not data.get("version"):
            return None
        try:
            return HarvestApi(
                version=data["version"],
                schema_version=int(data.get("schema-version", "0")),
                public_url=data.get("public-url", ""),
                api_paths=json.loads(data.get("api-paths", "[]")),
            )
        except (ValueError, KeyError):
            return None
