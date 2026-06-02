# Architecture Overview

Harvest is a three-component system for extracting, storing, and querying structural knowledge from versioned source code repositories.

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
│  GET /graph/:repo/:version/source  ──► single symbol source          │
│                                                                      │
│  GET /docs/:repo/:version[/:section/:file]  ──► Diataxis pages       │
└──────────────────────────────────────────────────────────────────────┘
                                   │
┌──────────────────────────────────────────────────────────────────────┐
│                            web-ui                                    │
│                                                                      │
│  Chat      — streaming answers, step timeline, inline symbol graphs  │
│  Explore   — interactive Cytoscape.js graph with AI symbol search    │
│  Document  — Diataxis documentation browser                          │
└──────────────────────────────────────────────────────────────────────┘
```

All three components are written in **Rust** (harvester and server) or **vanilla JavaScript** (web UI). The graph store is **Neo4j**. The agentic workflow and documentation pipeline can use either **Claude** (Anthropic API) or any **OpenAI-compatible provider** (Groq, local Ollama, etc.).

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

An HTTP API server that answers natural-language questions about the harvested code and serves graph data and documentation. It:

1. Accepts queries via `POST /query` (batch) or `POST /query/stream` (SSE).
2. Runs an **agentic loop**: the LLM is given Neo4j-backed tools and iterates until it has gathered enough context.
3. Returns a structured response with inline source citations in `[repo:version:file:line]` format.
4. Serves the full symbol graph for any `(repo, version)` pair via `GET /graph/:repo/:version`, backed by an in-memory cache pre-warmed at startup.
5. Serves Diataxis documentation pages produced by the harvester via `GET /docs/:repo/:version`.

See [server.md](server.md) for the full API reference, tool definitions, and LLM provider configuration.

### web-ui

A single-page application providing three views:

- **Chat** — streaming query interface; tool calls appear as a collapsible step timeline with AI-generated descriptions; answers include inline mini-graphs for referenced symbols.
- **Explore** — interactive symbol graph for any `(repo, version)` pair rendered with Cytoscape.js and an off-thread fcose layout; supports full-text and AI-powered symbol search with a source panel.
- **Document** — Diataxis documentation browser for AI-generated docs organised into Tutorials, How-to Guides, Explanations, and Reference.

See [web-ui/README.md](../../web-ui/README.md) for architecture, scripts, and test coverage.

---

## Technology Stack

| Concern              | Choice                                           |
|----------------------|--------------------------------------------------|
| Language (backend)   | Rust (both harvester and server)                 |
| HTTP framework       | axum                                             |
| Code parsing         | tree-sitter                                      |
| Graph database       | Neo4j 5 Community Edition                        |
| Neo4j Rust driver    | neo4rs                                           |
| LLM providers        | Claude (Anthropic API) or OpenAI-compatible      |
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
│   │   ├── config.rs           # config loading
│   │   ├── git.rs              # clone/fetch/ref listing
│   │   ├── parser/             # tree-sitter per-language parsers
│   │   ├── graph/              # graph model and Neo4j writer
│   │   ├── documentation/      # LLM-driven Diataxis doc pipeline
│   │   └── pipeline.rs         # orchestrates ingestion
│   └── Cargo.toml
├── knowledge-server/       # server crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs
│   │   ├── api/                # axum routes (/query, /graph, /docs, /repositories)
│   │   ├── agent/              # agentic loop + tool definitions
│   │   ├── llm/                # LLM provider abstraction
│   │   └── neo4j.rs            # Cypher query helpers
│   └── Cargo.toml
├── web-ui/                 # Vanilla JS SPA (Vite + Vitest)
│   ├── src/
│   └── tests/
├── documentation/
│   └── developer/          # this directory
├── docker-compose.yml      # Neo4j (Community 5) with APOC
└── Cargo.toml              # workspace root
```
