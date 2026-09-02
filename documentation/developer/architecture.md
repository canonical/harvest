# Architecture Overview

Harvest is a four-component system for extracting, storing, querying, and acting on structural knowledge from versioned source code repositories — and for driving infrastructure deployments from the same agent loop.

```
┌──────────────────────────────────────────────────────────────────────┐
│                        knowledge-harvester                           │
│                                                                      │
│  harvester.toml ──► repo list ──► git clone / fetch                  │
│                                       │                             │
│                                  walk git refs                      │
│                                       │                             │
│                             for each ref (version):                 │
│                               checkout ──► tree-sitter parse        │
│                                                 │                   │
│                                    functions, classes,              │
│                                    calls, relationships ──► Neo4j   │
│                                                                      │
│  document <repo:version> ──► LLM pipeline ──► Diataxis markdown     │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                              Neo4j DB + docs/
                                   │
┌──────────────────────────────────────────────────────────────────────┐
│                         knowledge-server                             │
│                                                                      │
│  POST /query[/stream]  ──► agentic loop: intent → LLM ◄──► tools    │
│                               └──► answer + [repo:version:file:line] │
│                                                                      │
│  GET /graph/:repo/:version  ──► cached symbol graph (JSON)           │
│  GET /docs/:repo/:version   ──► Diataxis documentation pages         │
│  GET /llm/providers         ──► chat-page model picker               │
│                                                                      │
│  /auth/*        ──► JWT + Google OAuth + OIDC, user/group management │
│  /projects/*    ──► workspaces, conversations, secrets, skills,      │
│                     artifacts, overview                              │
│  /projects/:pid/deployments/* ──► IaC pipeline: design, provision,   │
│                     execution plan, runs, proposals                  │
│  /groups/:gid/templates  ──► reusable product templates              │
│  /agents/*      ──► harvest-agent registry, SSE, command/terraform   │
│                     execution, console & tunnel WebSockets,          │
│                     port-forwards, optional LXD provisioning         │
│  /admin/*       ──► user role management, group CRUD, global skills  │
└──────────────────────────────────────────────────────────────────────┘
                          │                │
              ┌───────────▼──┐    ┌────────▼───────┐
              │   web-ui     │    │ harvest-agent  │
              │  (Vue 3 SPA) │    │ (Rust daemon)  │
              └──────────────┘    └────────────────┘
```

All three Rust components communicate via Neo4j and HTTP. The graph store is **Neo4j**. The agentic workflow and documentation pipeline can use **Claude** (Anthropic API), **Gemini** (Google), or any **OpenAI-compatible provider** (Groq, local Ollama, etc.); multiple providers can be configured with priority-based fallback.

---

## Components

### knowledge-harvester

A CLI tool and long-running daemon responsible for ingesting repositories and optionally generating documentation. It:

1. Reads a configuration file listing target repositories.
2. Clones each repository locally (or fetches updates if already cached).
3. Walks the configured git refs (tags by default, or an explicit `refs` list).
4. For each `(repo, ref)` pair not yet in the graph, checks out the ref and parses the source with **tree-sitter**.
5. Writes functions, classes, imports, call edges, and class relationship edges (inherits, implements, embeds, uses) into Neo4j.
6. Optionally generates [Diataxis](https://diataxis.fr/)-structured documentation for any ingested version via the `document` subcommand, which calls an LLM pipeline and writes markdown files to disk.

See [harvester.md](harvester.md) for the detailed pipeline, graph schema, and documentation workflow.

### knowledge-server

An HTTP API server that answers natural-language questions about the harvested code, manages users and projects, drives IaC deployments, and coordinates remote agents. It:

1. Accepts queries via `POST /query` (batch) or `POST /query/stream` (SSE).
2. Runs an **agentic loop**: the LLM classifies the user's intent, then is given tools (graph, machine, skill, infra, secret) and iterates until it has gathered enough context. Destructive tool calls can pause the loop for explicit user approval (confirm-action).
3. Returns a structured response with inline source citations in `[repo:version:file:line]` format and a `provider_used` block identifying which LLM provider/model answered.
4. Serves the full symbol graph for any `(repo, version)` pair via `GET /graph/:repo/:version`, backed by an in-memory cache pre-warmed at startup.
5. Serves Diataxis documentation pages produced by the harvester.
6. Manages users, groups, and projects. Each project can have multiple conversations, a secret store, skills, artifacts, connected agent machines, and an environment overview.
7. Maintains an in-memory registry of connected `harvest-agent` daemons via SSE. The LLM can `run_command`, run terraform/terragrunt, open interactive consoles and reverse tunnels, and manage port-forwards on any connected agent.
8. Runs a full IaC **deployment pipeline** per project: an LLM interview captures environment requirements, a design doc and terraform bundle are generated, an execution-plan DAG is defined and executed on an agent, and each run records captured output.
9. Generates AI-powered environment status dashboards (the "overview pipeline") by analysing project conversation history and querying agents.
10. Optionally provisions and tears down agents itself as LXD containers, when the server is configured with credentials for an LXD cluster.

See [server.md](server.md) for the full API reference, tool definitions, and LLM provider configuration.

### web-ui

A Vue 3 single-page application (Vite, Pinia, Vue Router, Canonical Vanilla Framework) providing multiple views:

- **Chat** — streaming query interface with intent/phase/thinking indicators, tool-call step timeline, Mermaid diagrams, inline `harvest-graph` snippets, source citations, file attachments, `ask_user` choice cards, and confirm-action approval gating.
- **Explore** — interactive symbol graph for any `(repo, version)` pair rendered with Cytoscape.js and an off-thread fcose layout; supports full-text and AI-powered symbol search with a source panel.
- **Document** — Diataxis documentation browser for AI-generated docs organised into Tutorials, How-to Guides, Explanations, and Reference.
- **Repositories** — list ingested repositories and versions.
- **Deploy & Design** — create and manage IaC deployments; generate design docs and terraform bundles; run plan/apply/destroy with streamed output; edit and approve proposed bundle changes; capture design decisions.
- **Artifacts / Skills** — manage the project's generated artifacts and skill playbooks.
- **Agents** — manage connected `harvest-agent` daemons; view online status, open an xterm.js console, run commands, rotate install tokens, manage port-forwards, and (when an LXD cluster is configured) provision or delete Harvest-managed agents running as LXD containers.
- **Overview** — per-project environment status dashboard generated by the LLM pipeline.
- **Admin** — user role management, group and membership administration, and global skill management.

The web UI source under `web-ui/src/` (views, components, stores, composables) is the authoritative reference for the SPA.

### harvest-agent

A lightweight Rust daemon that runs on any machine and connects back to the knowledge-server via a long-lived SSE stream. The server pushes commands; the agent carries them out and posts results back. It handles five message types:

- **`Execute`** — run a bash one-liner with a timeout and post stdout/stderr/exit-code.
- **`RunTerraform`** — write a terraform or terragrunt bundle to a temp dir, run `plan`/`apply`/`destroy`, stream each output line back as it is produced, then post the final result.
- **`OpenShell`** — upgrade to a WebSocket and serve an interactive PTY session (consumed by the web UI's xterm.js console).
- **`OpenTunnel`** — upgrade to a WebSocket and open a reverse tunnel so the server can reach a port on the agent's machine (used by port-forwards).
- **`Uninstall`** — trigger the uninstall script and exit.

This lets the project agent (and through it, the LLM) inspect and control connected machines in real time, run IaC bundles, and expose local services.

The agent authenticates with a short-lived install token on first connection and is issued a permanent hashed token (`agent_token_hash` stored in Neo4j). The config file never stores the project ID — the server derives project membership from the token hash.

The daemon itself doesn't know or care whether it's running on a machine an admin set up by hand or inside a container the server provisioned via the LXD API — it's the same binary and the same install flow either way. See [server.md](server.md#remote-agents-harvest-agent) for how the LXD-managed path works.

---

## Authentication and Authorisation

The server uses **JWT cookies** for session management. All protected routes (everything except `/health`, `/auth/*`, and agent-facing endpoints) require a valid JWT.

Roles:
- **admin** — full access to all projects, groups, users, and admin routes. The first registered user is automatically made admin.
- **regular** — can access projects belonging to groups they are a member of.

Google OAuth 2.0 and/or OIDC SSO (e.g. Ubuntu One, Dex, Keycloak) are optionally supported alongside local password authentication. Local password login can be disabled by setting `auth.allow_local_login = false`.

---

## LLM Retry Strategy

All LLM providers (Anthropic, Gemini, and OpenAI-compatible) share a common retry strategy implemented in `llm/retry.rs`:

- **Timeout errors** — exponential backoff (2, 4, 8, 16, 32 seconds, capped at 32).
- **429 rate limit** — honours the `retry-after` header if present; falls back to exponential backoff.
- **Overload (529/503 for Anthropic, 502/503 for OpenAI, 503 for Gemini)** — fixed 5-second delay.
- **Other errors** — returned immediately without retrying.

When multiple providers are configured, a `FallbackProvider` tries them in ascending `priority` order on rate-limit errors; the chat page's model picker can also target a specific provider/model directly via `ProviderSelection`.

---

## Agentic Loop

The `Agent` type in `agent/mod.rs` implements the agentic loop. `query_streaming` is the primary method; `query` is a thin wrapper that collects all events from an internal channel and returns the final `QueryResponse`.

The loop:
1. **Classifies intent** — `conversational`, `research`, `action`, or `hybrid`. Conversational turns skip tools entirely (only `ask_user` is available); the others get the full tool set. Classification uses simple heuristics for first-turn action verbs, otherwise a single LLM call.
2. Optionally compacts the conversation history if it exceeds `compaction_threshold_chars` (summarises old turns with one LLM call).
3. Appends the system prompt, compacted history, and the current user message.
4. Streams the LLM response, forwarding `thinking_delta`, `text_delta`, and `tool_call` events to the client as they arrive.
5. If the response is a tool-use batch, partitions it into confirmable and automatic calls:
   - confirmable tools (those flagged `requires_confirmation`) emit `confirm_action` events and **pause** the loop until the user approves via `POST /projects/:pid/conversations/:cid/confirm-action/resume`;
   - automatic tools execute concurrently with `join_all`.
6. Repeats until the LLM returns a plain text message (`end_turn`), calls `ask_user` (emits a `question` event and ends the turn), or `max_iterations` is reached (at which point a final synthesis call is made).
7. Extracts `[repo:version:file:line]` citations from the final answer.

Tools are executed concurrently using `join_all` — multiple tool calls in a single LLM turn run in parallel.

---

## Technology Stack

| Concern              | Choice                                           |
|----------------------|--------------------------------------------------|
| Language (backend)   | Rust (harvester, server, and agent daemon)       |
| HTTP framework       | axum                                             |
| Code parsing         | tree-sitter                                      |
| Graph database       | Neo4j 5 Community Edition                        |
| Neo4j Rust driver    | neo4rs                                           |
| LLM providers        | Claude (Anthropic), Gemini (Google), or OpenAI-compatible |
| LLM routing          | priority-based `FallbackProvider` across providers |
| Authentication       | JWT cookies + optional Google OAuth 2.0 + OIDC   |
| Async runtime        | tokio                                            |
| Configuration        | TOML                                             |
| Web UI framework     | Vue 3 + Pinia + Vue Router                       |
| Web UI build         | Vite                                             |
| Web UI tests         | Vitest (jsdom)                                   |
| Graph rendering      | Cytoscape.js + fcose layout (off-thread worker)  |
| Diagrams             | Mermaid                                          |
| Terminal             | xterm.js (agent consoles)                        |
| CSS framework        | Canonical Vanilla Framework                      |

---

## Monorepo Layout

```
harvest/
├── knowledge-harvester/    # harvester crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs           # TOML config loading
│   │   ├── git.rs              # clone/fetch/ref listing
│   │   ├── parser/             # tree-sitter per-language parsers
│   │   ├── graph/              # graph model and Neo4j writer
│   │   ├── documentation/      # LLM-driven Diataxis doc pipeline
│   │   │   ├── llm.rs          # Anthropic + Gemini + OpenAI-compat clients
│   │   │   ├── retry.rs        # shared exponential-backoff retry
│   │   │   └── workflow.rs     # 4-phase doc generation workflow
│   │   └── pipeline.rs         # orchestrates ingestion
│   └── Cargo.toml
├── knowledge-server/       # server crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs           # TOML config (multi-provider LLM, agent, lxd, …)
│   │   ├── neo4j.rs            # Cypher query helpers
│   │   ├── api/                # axum router and shared state types
│   │   ├── agent/              # agentic loop + all tool definitions
│   │   │   ├── mod.rs          # Agent struct, query/query_streaming, intent
│   │   │   ├── tool.rs         # Tool trait + DEFAULT_PREVIEW_CHARS
│   │   │   ├── graph_tools.rs  # Neo4j graph tools
│   │   │   ├── machine_tools.rs # list_agents, run_command (read-only variant too)
│   │   │   ├── skill_tools.rs  # list_skills, load_skill
│   │   │   ├── lxd_tools.rs    # create_lxd_agent, delete_agent
│   │   │   ├── port_forward_tools.rs # port-forward CRUD
│   │   │   ├── artifact_tools.rs # generate_artifact
│   │   │   ├── terraform_tools.rs # run_terraform_plan/apply/destroy
│   │   │   ├── deployment_tools.rs # deployment-scoped tools
│   │   │   ├── chain.rs        # agent chain helpers
│   │   │   └── prompt.rs       # system prompts (chat, deployment)
│   │   ├── llm/                # LLM provider abstraction
│   │   │   ├── mod.rs          # LlmProvider trait + FallbackProvider + factory
│   │   │   ├── types.rs        # Message, ToolCall, StreamEvent, ProviderSelection, …
│   │   │   ├── anthropic.rs    # Anthropic Messages API
│   │   │   ├── gemini.rs       # Google Gemini API
│   │   │   ├── openai_compat.rs # OpenAI Chat Completions API
│   │   │   └── retry.rs        # shared retry helper
│   │   ├── auth/               # JWT, Google OAuth, OIDC, password hashing
│   │   ├── conversations/      # user conversation history
│   │   ├── projects/           # project/group CRUD, per-project query
│   │   ├── artifacts/          # artifact store + terraform bundle handling
│   │   ├── deployments/        # IaC pipeline: design, provision, runs, execution plan
│   │   ├── skills/             # global + per-project skill store
│   │   ├── lxd/                # LXD REST client (networks, instances, exec)
│   │   ├── machines/           # agent daemon registry + SSE/console/tunnel handlers
│   │   │   ├── lxd_provision.rs # provisions LXD-managed agents
│   │   │   ├── port_forwards.rs # port-forward persistence
│   │   │   └── proxy.rs        # HTTP proxying over port-forwards
│   │   ├── overview/           # environment status pipeline
│   │   └── admin/              # user/group/admin route handlers
│   ├── skills/                 # built-in skill markdown (juju, lxd, ceph, …)
│   └── Cargo.toml
├── agent/                  # harvest-agent daemon crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs       # server_url + agent_token config
│   │   ├── executor.rs     # bash command runner with timeout
│   │   ├── terraform.rs    # terraform/terragrunt bundle runner with streamed output
│   │   ├── console.rs      # interactive PTY console session over WebSocket
│   │   ├── tunnel.rs       # reverse-tunnel session over WebSocket
│   │   ├── ws_url.rs       # WebSocket URL helpers
│   │   └── sse_client.rs   # SSE reconnect loop + ping task
│   └── Cargo.toml
├── web-ui/                 # Vue 3 SPA (Vite + Vitest, Pinia, Vue Router)
│   ├── src/
│   │   ├── views/          # Chat, Explore, Document, Repositories, Deploy,
│   │   │                   #   Design, Artifacts, Skills, Agents, AgentConsole,
│   │   │                   #   Admin, Login, Register
│   │   ├── components/     # chat/, agents/, deployment/, SourcePanel
│   │   ├── stores/         # Pinia stores
│   │   ├── composables/    # Vue composables
│   │   ├── router/         # Vue Router config
│   │   └── lib/            # API client + helpers
│   └── tests/
├── documentation/
│   └── developer/          # this directory
├── docker-compose.yml      # Neo4j + server + web-ui (all-in-one)
└── Cargo.toml              # workspace root
```
