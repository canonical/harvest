# Architecture Overview

Harvest is a four-component system for extracting, storing, querying, and acting on structural knowledge from versioned source code repositories.

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
│  POST /query[/stream]  ──► agentic loop: LLM ◄──► Neo4j tools       │
│                               └──► answer + [repo:version:file:line] │
│                                                                      │
│  GET /graph/:repo/:version  ──► cached symbol graph (JSON)           │
│  GET /docs/:repo/:version   ──► Diataxis documentation pages         │
│                                                                      │
│  /auth/*        ──► JWT + Google OAuth, user/group management        │
│  /projects/*    ──► workspaces, conversations, per-project agents    │
│  /agents/*      ──► harvest-agent registry, SSE, command execution   │
│  /admin/*       ──► user role management, group CRUD                 │
└──────────────────────────────────────────────────────────────────────┘
                          │                │
              ┌───────────▼──┐    ┌────────▼───────┐
              │   web-ui     │    │ harvest-agent  │
              │  (Vite / JS) │    │ (Rust daemon)  │
              └──────────────┘    └────────────────┘
```

All three Rust components communicate via Neo4j and HTTP. The graph store is **Neo4j**. The agentic workflow and documentation pipeline can use either **Claude** (Anthropic API) or any **OpenAI-compatible provider** (Groq, local Ollama, etc.).

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

An HTTP API server that answers natural-language questions about the harvested code, manages users and projects, and coordinates remote agents. It:

1. Accepts queries via `POST /query` (batch) or `POST /query/stream` (SSE).
2. Runs an **agentic loop**: the LLM is given Neo4j-backed tools and iterates until it has gathered enough context.
3. Returns a structured response with inline source citations in `[repo:version:file:line]` format.
4. Serves the full symbol graph for any `(repo, version)` pair via `GET /graph/:repo/:version`, backed by an in-memory cache pre-warmed at startup.
5. Serves Diataxis documentation pages produced by the harvester.
6. Manages users, groups, and projects. Each project can have multiple conversations, a secret store, and connected agent machines.
7. Maintains an in-memory registry of connected `harvest-agent` daemons via SSE. The LLM can `run_command` on any connected agent.
8. Generates AI-powered environment status dashboards (the "overview pipeline") by analysing project conversation history and querying agents.

See [server.md](server.md) for the full API reference, tool definitions, and LLM provider configuration.

### web-ui

A single-page application providing multiple views:

- **Chat** — streaming query interface with tool-call step timeline, inline symbol graphs, source citations, and file attachments.
- **Explore** — interactive symbol graph for any `(repo, version)` pair rendered with Cytoscape.js and an off-thread fcose layout; supports full-text and AI-powered symbol search with a source panel.
- **Document** — Diataxis documentation browser for AI-generated docs organised into Tutorials, How-to Guides, Explanations, and Reference.
- **Projects** — workspace management: create/edit projects, manage conversations, view collaboration presence.
- **Agents** — manage connected `harvest-agent` daemons; view online status, run commands, rotate install tokens.
- **Overview** — per-project environment status dashboard generated by the LLM pipeline.
- **Admin** — user role management, group and membership administration.

See [web-ui/README.md](../../web-ui/README.md) for architecture, scripts, and test coverage.

### harvest-agent

A lightweight Rust daemon that runs on any machine and connects back to the knowledge-server via a long-lived SSE stream. The server pushes `Execute` commands; the agent runs them as bash one-liners and posts results back. This lets the project agent (and through it, the LLM) inspect and control connected machines in real time.

The agent authenticates with a short-lived install token on first connection and is issued a permanent hashed token (`agent_token_hash` stored in Neo4j). The config file never stores the project ID — the server derives project membership from the token hash.

---

## Authentication and Authorisation

The server uses **JWT cookies** for session management. All protected routes (everything except `/health`, `/auth/*`, and agent-facing endpoints) require a valid JWT.

Roles:
- **admin** — full access to all projects, groups, users, and admin routes. The first registered user is automatically made admin.
- **regular** — can access projects belonging to groups they are a member of.

Google OAuth 2.0 is optionally supported alongside local password authentication.

---

## LLM Retry Strategy

All LLM providers (Anthropic and OpenAI-compatible) share a common retry strategy implemented in `llm/retry.rs`:

- **Timeout errors** — exponential backoff (2, 4, 8, 16, 32 seconds, capped at 32).
- **429 rate limit** — honours the `retry-after` header if present; falls back to exponential backoff.
- **Overload (529/503 for Anthropic, 502/503 for OpenAI)** — fixed 5-second delay.
- **Other errors** — returned immediately without retrying.

---

## Agentic Loop

The `Agent` type in `agent/mod.rs` implements the agentic loop. `query_streaming` is the primary method; `query` is a thin wrapper that collects all events from an internal channel and returns the final `QueryResponse`.

The loop:
1. Optionally compacts the conversation history if it exceeds `compaction_threshold_chars` (summarises old turns with one LLM call).
2. Appends the system prompt, compacted history, and the current user message.
3. Calls the LLM with the current message list and all tool definitions.
4. If the response is a tool-use batch, executes all tool calls concurrently and appends results.
5. Repeats until the LLM returns a plain text message or `max_iterations` is reached.
6. Extracts `[repo:version:file:line]` citations from the final answer.

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
| LLM providers        | Claude (Anthropic API) or OpenAI-compatible      |
| Authentication       | JWT cookies + optional Google OAuth 2.0          |
| Async runtime        | tokio                                            |
| Configuration        | TOML                                             |
| Web UI build         | Vite                                             |
| Web UI tests         | Vitest (jsdom)                                   |
| Graph rendering      | Cytoscape.js + fcose layout (off-thread worker)  |
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
│   │   │   ├── llm.rs          # Anthropic + OpenAI-compat clients
│   │   │   ├── retry.rs        # shared exponential-backoff retry
│   │   │   └── workflow.rs     # 4-phase doc generation workflow
│   │   └── pipeline.rs         # orchestrates ingestion
│   └── Cargo.toml
├── knowledge-server/       # server crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs
│   │   ├── neo4j.rs            # Cypher query helpers
│   │   ├── api/                # axum router and shared state types
│   │   ├── agent/              # agentic loop + all tool definitions
│   │   │   ├── mod.rs          # Agent struct, query/query_streaming
│   │   │   ├── tool.rs         # Tool trait + DEFAULT_PREVIEW_CHARS
│   │   │   ├── graph_tools.rs  # Neo4j graph tools
│   │   │   ├── machine_tools.rs # list_agents, run_command
│   │   │   ├── secret_tools.rs # list/get/save secret
│   │   │   └── prompt.rs       # system prompt
│   │   ├── llm/                # LLM provider abstraction
│   │   │   ├── mod.rs          # LlmProvider trait + factory
│   │   │   ├── types.rs        # Message, ToolCall, LlmResponse, …
│   │   │   ├── anthropic.rs    # Anthropic Messages API
│   │   │   ├── openai_compat.rs # OpenAI Chat Completions API
│   │   │   └── retry.rs        # shared retry helper
│   │   ├── auth/               # JWT, Google OAuth, password hashing
│   │   ├── conversations/      # user conversation history
│   │   ├── machines/           # agent daemon registry + SSE handlers
│   │   ├── overview/           # environment status pipeline
│   │   └── projects/           # project/group CRUD + per-project query
│   └── Cargo.toml
├── agent/                  # harvest-agent daemon crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs       # server_url + agent_token config
│   │   ├── executor.rs     # bash command runner with timeout
│   │   └── sse_client.rs   # SSE reconnect loop + ping task
│   └── Cargo.toml
├── web-ui/                 # Vanilla JS SPA (Vite + Vitest)
│   ├── src/
│   └── tests/
├── documentation/
│   └── developer/          # this directory
├── docker-compose.yml      # Neo4j (Community 5) with APOC
└── Cargo.toml              # workspace root
```
