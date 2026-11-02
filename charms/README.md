# Harvest charms

Machine charms deploy Harvest highly available next to the existing Charmed PostgreSQL
and HAProxy charms. Integrator charms provide LLM and authentication providers over
Juju relations.

| Charm | Runs | Relations |
|---|---|---|
| `harvest-server` | `harvest.server` from the harvest snap (API, chat turns, agents) | `database` (postgresql_client), `haproxy-route`, `llm-provider`, `auth-provider`, `harvest-api` (provides), `cos-agent`, peers |
| `harvest-web-ui` | `harvest.ui` from the harvest snap in static mode (single page app only) | `haproxy-route`, `harvest-api` (requires) |
| `harvest-llm-provider-openai-compatible` | Integrator | `llm-provider` (provides) |
| `harvest-llm-provider-anthropic` | Integrator | `llm-provider` (provides) |
| `harvest-llm-provider-opencode` | Integrator | `llm-provider` (provides) |
| `harvest-auth-provider-oidc` | Integrator | `auth-provider` (provides) |
| `harvest-auth-provider-google` | Integrator | `auth-provider` (provides) |

```
              VIP (hacluster) or DNS
                      │ https
          ┌──────────────────────────┐
          │  haproxy 2.8/stable ×3    │◄── certificates ── self-signed-certificates / lego
          └──────────────────────────┘
  haproxy-route │ API paths           │ haproxy-route │ everything else
          ┌────────────────┐    ┌─────────────────┐
          │ harvest-server │◄───│ harvest-web-ui  │  harvest-api (version skew check)
          │      ×3        │    │      ×2         │
          └────────────────┘    └─────────────────┘
       peers │  :8081 node to node (agents, turn snapshots)
    database │
          ┌──────────────────────────┐
          │ postgresql 16/stable ×3   │  plugin_pg_trgm_enable, plugin_vector_enable
          └──────────────────────────┘
```

## Deploy

```bash
juju deploy postgresql --channel 16/stable -n 3 \
  --config plugin_pg_trgm_enable=true --config plugin_vector_enable=true
juju deploy self-signed-certificates
juju deploy haproxy --channel 2.8/stable -n 3 --config external-hostname=harvest.example.com --config vip=10.0.0.100
juju deploy hacluster --channel 2.4/edge --base ubuntu@24.04
juju integrate hacluster haproxy
juju integrate haproxy:certificates self-signed-certificates

juju deploy ./harvest-server_amd64.charm -n 3 \
  --resource harvest-snap=./harvest.snap \
  --config hostname=harvest.example.com
juju deploy ./harvest-llm-provider-openai-compatible_amd64.charm \
  --config model=gpt-4.1-mini --config base-url=https://api.openai.com/v1
juju deploy ./harvest-web-ui_amd64.charm -n 2 --resource harvest-snap=./harvest.snap

juju integrate harvest-server:database postgresql:database
juju integrate harvest-server:haproxy-route haproxy:haproxy-route
juju integrate harvest-llm-provider-openai-compatible:llm-provider harvest-server:llm-provider
juju integrate harvest-web-ui:haproxy-route haproxy:haproxy-route
juju integrate harvest-web-ui:harvest-api harvest-server:harvest-api
```

LLM provider charms publish one provider each. Integrate more than one provider charm to build
a fallback list, and set provider priority with each charm's `priority` config.

### Lab bundle

`bundle.yaml` deploys the same topology on a single LXD host without hacluster: three
PostgreSQL units, one haproxy unit sharing its machine with self-signed-certificates, three
server units and two web UI units, routed for `harvest.test`. Charmhub revisions are pinned.

```bash
cd charms
(cd harvest-server && charmcraft pack) && (cd harvest-web-ui && charmcraft pack) && (cd harvest-llm-provider-openai-compatible && charmcraft pack)
cp ../harvest_*.snap harvest.snap
juju add-model harvest
./deploy-bundle.sh harvest
```

`deploy-bundle.sh` turns off the OS upgrade of new machines, preinstalls `core24` while they
boot (`cloudinit-userdata.yaml`), adds every machine before the bundle's charm uploads,
deploys PostgreSQL first because it is the critical path, then deploys the rest and waits
until every unit is active and idle.

Deploy time is dominated by fsync: Juju fsyncs each extracted charm file, and snapd, MongoDB
and dqlite sync constantly. Keep Juju machines on a storage pool backed directly by the host
filesystem (the LXD `dir` driver) rather than a loop file, which made fsync more than ten
times slower here. The charms use the `uv` plugin with a lockfile, so they hold only runtime
dependencies (about 700 files instead of 2200 with the `charm` plugin) and build without a
compiler.

## What the server charm does

* Generates `jwt-secret`, `user-key-encryption-key` and the node-to-node `cluster-secret`
  once, on the leader, in an application secret shared through the peer relation. Set
  `import-secrets` to a secret holding `jwt-secret` and `user-key-encryption-key` to keep
  the keys of an existing deployment (encrypted per-user API keys stay readable).
* Runs the cluster control plane on PostgreSQL by default, or on the embedded quorium raft
  group with `cluster-backend=quorium` (three units or more). The first leader seeds the
  group, the other units join their peers, departed units are removed from the voters, and
  rolling restarts wait until restarting a unit keeps a healthy majority. `cluster-rotate-secrets`
  rotates the node-to-node secret in three phases without downtime. See
  [cluster.md](../documentation/developer/cluster.md).
* Builds the database URL from every PostgreSQL endpoint in a stable order with
  `target_session_attrs=read-write`, so a switchover is followed without a restart. When the
  relation reports TLS the CA is written to `$SNAP_COMMON/db-ca.pem` and `sslmode=require` is set.
  The first start waits up to ten minutes until every related PostgreSQL unit is listed, so
  replicas joining during a fresh deployment do not trigger rolling restarts.
* Renders `$SNAP_COMMON/server.toml` (`config.managed=true` stops the snap hook from
  overwriting it), validates it with `harvest.admin check-config`, lets only the leader run
  `harvest.admin migrate` (every unit starts with `migrate_on_start = false` and waits for the
  schema), and applies later changes with a rolling restart (one unit at a time, each waits
  for `/health/ready`).
* Requests an haproxy route for the API path prefixes with `leastconn`, `/health/ready` checks
  (5 s, rise 2, fall 3), retries with redispatch and a one hour server timeout for streams.
  Draining units fail readiness first, so HAProxy stops sending them work before they exit.
* Publishes the public URL learnt from haproxy to OAuth redirects and the agent installer.
* Exposes `/metrics` and alert rules through `cos-agent`.

## Tests

```bash
cd charms/harvest-server && uv run --with pytest --with ops[testing] env PYTHONPATH=src:lib pytest tests/unit
cd charms/harvest-web-ui && uv run --with pytest --with ops[testing] env PYTHONPATH=src:lib pytest tests/unit
HARVEST_SERVER_CHARM=... HARVEST_UI_CHARM=... HARVEST_LLM_OPENAI_COMPATIBLE_CHARM=... HARVEST_SNAP=... pytest charms/tests/integration
```

The integration tests deploy the full topology with jubilant and check routing, the loss of
a server unit, a PostgreSQL switchover and a rolling configuration change.
