# Harvest charms

Two machine charms deploy Harvest highly available next to the existing Charmed PostgreSQL
and HAProxy charms.

| Charm | Runs | Relations |
|---|---|---|
| `harvest-server` | `harvest.server` from the harvest snap (API, chat turns, agents) | `database` (postgresql_client), `haproxy-route`, `harvest-api` (provides), `cos-agent`, peers |
| `harvest-web-ui` | `harvest.ui` from the harvest snap in static mode (single page app only) | `haproxy-route`, `harvest-api` (requires) |

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

juju add-secret harvest-llm key=<api key>
juju deploy ./harvest-server_amd64.charm -n 3 \
  --resource harvest-snap=./harvest.snap \
  --config llm-providers="$(cat llm.toml)" --config llm-secret=secret:<id>
juju grant-secret harvest-llm harvest-server
juju deploy ./harvest-web-ui_amd64.charm -n 2 --resource harvest-snap=./harvest.snap

juju integrate harvest-server:database postgresql:database
juju integrate harvest-server:haproxy-route haproxy:haproxy-route
juju integrate harvest-web-ui:haproxy-route haproxy:haproxy-route
juju integrate harvest-web-ui:harvest-api harvest-server:harvest-api
```

`llm.toml` holds `[[llm]]` blocks. Values can refer to keys of the `llm-secret` secret as
`${secret:<key>}`. With per-user API keys use `user_provided_key = true` and leave `api_key`
empty.

## What the server charm does

* Generates `jwt-secret`, `user-key-encryption-key` and the node-to-node `cluster-secret`
  once, on the leader, in an application secret shared through the peer relation. Set
  `import-secrets` to a secret holding `jwt-secret` and `user-key-encryption-key` to keep
  the keys of an existing deployment (encrypted per-user API keys stay readable).
* Builds the database URL from every PostgreSQL endpoint (primary first, then replicas) with
  `target_session_attrs=read-write`, so a switchover is followed without a restart. When the
  relation reports TLS the CA is written to `$SNAP_COMMON/db-ca.pem` and `sslmode=require` is set.
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
cd charms/harvest-server && PYTHONPATH=src:lib pytest tests/unit
cd charms/harvest-web-ui && PYTHONPATH=src:lib pytest tests/unit
HARVEST_SERVER_CHARM=... HARVEST_UI_CHARM=... HARVEST_SNAP=... pytest charms/tests/integration
```

The integration tests deploy the full topology with jubilant and check routing, the loss of
a server unit, a PostgreSQL switchover and a rolling configuration change.
