# Architecture

```text
Browser ──▶ web UI (harvest.ui) ──▶ knowledge-server (harvest.server) ──▶ PostgreSQL
                                        │      │        │
                                        │      │        └──▶ model providers
                                        │      └──▶ LXD: session containers, previews, managed agents
                                        └──◀── harvest-agent on your machines (outbound)
```

| Component | Role |
|---|---|
| `web-ui` | Vue 3 single-page app styled with Vanilla; in `proxy` mode also forwards `/api` to the server |
| `knowledge-server` | HTTP API, authentication, the agent and its tools, Code and Deploy orchestration, preview proxy, background jobs |
| `knowledge-harvester` | ingestion: clone, parse, write the code graph (also used by the server for web-UI ingestion) |
| `harvest-db` | schema and migrations |
| `harvest-search` | search over the code graph |
| `harvest-agent` | machine agent for Deploy |
| `harvest` (`harvest-cli`) | operator CLI for the snap |
| `harvest-tui` | terminal chat client |
| `quorium` | optional Raft membership for clusters |

## Request paths

- **A chat message**: UI → `/query/stream` → a turn on the server → model and tools (database queries for the graph) → server-sent events back.
- **A coding agent step**: same, with Workspace tools executed in the session's LXD container through the LXD API.
- **A preview request**: `p-<id>.<domain>` → reverse proxy → server's preview proxy → the app in the container (or the viewer's copy).
- **A deployment step**: server → event to the connected agent → command or Terraform on the machine → results back.

## Background jobs

The server runs, under cluster leases: ingestion jobs, the code keeper (images, warm pools, idle sessions, re-indexing), preview and copy expiry, repository access re-checks, enrichment and embedding backfills.

Contributor details: [developer/architecture.md](../developer/architecture.md).
