# Architecture Overview

Harvest is a two-component system for extracting, storing, and querying structural knowledge from versioned source code repositories.

```
┌─────────────────────────────────────────────────────────────┐
│                     knowledge-harvester                     │
│                                                             │
│  config.toml ──► repo list ──► git clone / fetch            │
│                                    │                        │
│                               list git tags                 │
│                                    │                        │
│                          for each tag (version):            │
│                            checkout ──► tree-sitter parse   │
│                                              │              │
│                                       build graph ──► Neo4j │
└─────────────────────────────────────────────────────────────┘
                                │
                           Neo4j DB
                                │
┌─────────────────────────────────────────────────────────────┐
│                      knowledge-server                       │
│                                                             │
│  HTTP POST /query                                           │
│       │                                                     │
│       ▼                                                     │
│  agentic loop:                                              │
│    LLM ◄──► graph query tools (Cypher over Neo4j)           │
│       │                                                     │
│       ▼                                                     │
│  structured answer + sources [repo:version:file:line]       │
└─────────────────────────────────────────────────────────────┘
```

Both components are written in **Rust**. The graph store is **Neo4j**. The agentic workflow can use either **Claude** (Anthropic API) or any **OpenAI-compatible provider** (Groq, local Ollama, etc.).

---

## Components

### knowledge-harvester

A CLI tool and long-running daemon responsible for ingesting repositories. It:

1. Reads a configuration file listing target repositories.
2. Clones each repository locally (or fetches updates if already cached).
3. Lists all **git tags** — each tag is treated as one version.
4. For each (repo, tag) pair not yet in the graph, checks out the tag and parses the source code with **tree-sitter**.
5. Writes the resulting knowledge graph into Neo4j.

See [harvester.md](harvester.md) for the detailed pipeline.

### knowledge-server

An HTTP API server that answers natural-language questions about the harvested code. It:

1. Accepts a query via `POST /query`.
2. Runs an **agentic loop**: the LLM is given a set of Neo4j-backed tools and iterates until it has gathered enough context.
3. Returns a structured response including inline source citations in `[repo:version:file:line]` format.

See [server.md](server.md) for the API spec, tool definitions, and LLM provider configuration.

---

## Technology Stack

| Concern              | Choice                                      |
|----------------------|---------------------------------------------|
| Language             | Rust (both components)                      |
| HTTP framework       | axum                                        |
| Code parsing         | tree-sitter                                 |
| Graph database       | Neo4j (Community Edition)                   |
| Neo4j Rust driver    | neo4rs                                      |
| LLM providers        | Claude (Anthropic API) or OpenAI-compatible |
| Async runtime        | tokio                                       |
| Configuration        | TOML                                        |

---

## Monorepo Layout

```
harvest/
├── knowledge-harvester/    # harvester crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs       # config loading
│   │   ├── git.rs          # clone/fetch/tag listing
│   │   ├── parser/         # tree-sitter per-language parsers
│   │   ├── graph/          # graph builder and Neo4j writer
│   │   └── pipeline.rs     # orchestrates the above
│   └── Cargo.toml
├── knowledge-server/       # server crate
│   ├── src/
│   │   ├── main.rs
│   │   ├── config.rs
│   │   ├── api/            # axum routes
│   │   ├── agent/          # agentic loop + tool definitions
│   │   ├── llm/            # LLM provider abstraction
│   │   └── neo4j.rs        # Cypher query helpers
│   └── Cargo.toml
├── documentation/
│   ├── developer/
│   └── user/
└── Cargo.toml              # workspace root
```
