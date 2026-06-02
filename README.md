# Harvest
Ask code, not documentation - Harvesting knowledge from project source using Agents.

| Chat | Explore | Document |
|-|-|-|
| <img width="1920" height="952" alt="image" src="https://github.com/user-attachments/assets/cb1311b8-df2c-4644-b68f-df667446082a" /> | <img width="1920" height="952" alt="image" src="https://github.com/user-attachments/assets/a5df1658-a119-4d97-a817-95bcb050d1ac" /> | <img width="1920" height="952" alt="image" src="https://github.com/user-attachments/assets/6f71950f-1769-42c4-9466-7050e1d28346" /> |

Harvest turns versioned source code repositories into a queryable knowledge graph. Point it at a list of Git repositories, let it ingest every tagged version, then ask natural-language questions through the chat interface or the HTTP API.

```
┌─────────────────────┐     ┌──────────────────────┐     ┌──────────────┐
│ knowledge-harvester │────▶│      Neo4j graph     │────▶│ knowledge-   │
│  (Rust CLI / daemon)│     │  functions, classes, │     │ server       │
│  git + tree-sitter  │     │  calls, imports, …   │     │ (HTTP + SSE) │
└─────────────────────┘     └──────────────────────┘     └──────┬───────┘
                                                                │
                                                       ┌────────▼───────┐
                                                       │    web-ui      │
                                                       │  (Vite / JS)   │
                                                       └────────────────┘
```

---

## What it does

1. **Harvester** clones each repository, walks provided git refs, and parses the source with [tree-sitter](https://tree-sitter.github.io/tree-sitter/). Functions, classes, imports, and call edges are written to Neo4j. Each `(repo, version)` pair is an atomic unit — safe to interrupt and re-run.

2. **Server** exposes a REST + SSE API. A query triggers an agentic loop: the LLM calls graph tools (search, source retrieval, call-graph traversal, custom Cypher) until it has enough context, then returns a structured answer with inline `[repo:version:file:line]` citations.

3. **Web UI** provides a streaming chat interface. Tool calls appear as collapsible cards in real time. Final answers are rendered as Markdown with syntax-highlighted code and clickable source chips.

---

## Repository layout

```
harvest/
├── knowledge-harvester/    # Rust CLI — ingests repos into Neo4j
│   ├── src/
│   └── harvester.toml      # example config
├── knowledge-server/       # Rust HTTP server — answers questions
│   ├── src/
│   │   ├── agent/          # agentic loop, graph tools, prompts
│   │   ├── api/            # axum routes (/query, /query/stream, /repositories)
│   │   └── llm/            # Anthropic + OpenAI-compat provider adapters
│   └── server.toml         # example config
├── web-ui/                 # Vanilla JS chat interface (Vite + Vitest)
│   ├── src/
│   └── tests/
├── documentation/
│   └── developer/          # architecture, harvester, server, dev-setup docs
├── docker-compose.yml      # Neo4j (Community 5) with APOC
└── Cargo.toml              # Cargo workspace
```

---

## Quick start

### 1 — Start Neo4j

```bash
docker compose up -d
# Neo4j browser: http://localhost:7474  (neo4j / devpassword)
```

### 2 — Ingest some repositories

Edit `knowledge-harvester/harvester.toml`:

```toml
[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "devpassword"

[storage]
clone_root = "/tmp/harvest-repos"

[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/my-repo.git"
```

Run the harvester:

```bash
cd knowledge-harvester
RUST_LOG=info cargo run -- --config harvester.toml run
```

### 3 — Configure and start the server

Edit `knowledge-server/server.toml` — choose one LLM provider:

```toml
[server]
host = "127.0.0.1"
port = 8080

[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "devpassword"

# Anthropic Claude
[llm]
provider       = "anthropic"
model          = "claude-sonnet-4-6"
api_key        = "sk-ant-..."
max_iterations = 20

# — or — OpenAI-compatible (Groq, Ollama, etc.)
# [llm]
# provider = "openai-compat"
# base_url = "https://api.groq.com/openai/v1"
# api_key  = "gsk_..."
# model    = "llama-3.3-70b-versatile"
```

```bash
cd knowledge-server
RUST_LOG=info cargo run -- --config server.toml
# Listening on 127.0.0.1:8080
```

### 4 — Open the chat UI

```bash
cd web-ui
npm install
npm run dev
# Open http://localhost:5173
```

The Vite dev server proxies all API calls to `localhost:8080` automatically.

---

## HTTP API

### `POST /query`

Ask a question, get a complete JSON response.

```bash
curl -s http://localhost:8080/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "How does the retry logic work?"}' | jq .
```

```json
{
  "answer": "The retry logic lives in `llm/anthropic.rs` …",
  "sources": [
    { "repo": "my-repo", "version": "v1.2.0", "file": "src/llm/anthropic.rs", "line": 84 }
  ],
  "tool_calls_made": 4
}
```

### `POST /query/stream`

Same payload, streams [Server-Sent Events](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events) so you can display tool calls as they happen:

| Event | Payload |
|-------|---------|
| `tool_call` | `{type, name, input}` |
| `tool_result` | `{type, name, preview}` |
| `done` | `{type, answer, sources, tool_calls_made}` |
| `error` | `{type, message}` |

### `GET /repositories`

List all ingested repositories and their available versions.

### `GET /health`

Returns `{"status": "ok"}`.

---

## Graph query tools

The agent has access to these Neo4j-backed tools:

| Tool | Description |
|------|-------------|
| `list_repositories` | All repos and their ingested versions |
| `search_symbols` | Full-text search for functions/classes by name |
| `get_symbol_source` | Full source text of a specific function or class |
| `get_file_symbols` | All symbols defined in a file |
| `find_callers` | Functions that call a given function |
| `find_callees` | Functions called by a given function |
| `get_imports` | Import declarations for a file |
| `compare_symbol_across_versions` | Source diff for a symbol between two versions |
| `run_cypher` | Arbitrary read-only Cypher for custom traversals |

---

## Web UI features

- **Streaming tool calls** — each tool invocation appears as a collapsible card with inputs and a result preview
- **Markdown answers** — rendered with syntax-highlighted code blocks (Atom One Dark)
- **Source citations** — inline `[repo:version:file:line]` markers become amber chips; a sources panel lists them all
- **Dark / light / auto theme** — toggle in the bottom-right corner; persists across reloads; auto follows the OS setting with no flash on reload
- **Repository sidebar** — live list of ingested repos and versions from the server

---

## Technology stack

| Concern | Choice |
|---------|--------|
| Harvester language | Rust |
| Server language | Rust |
| HTTP framework | axum |
| Code parsing | tree-sitter |
| Graph database | Neo4j 5 Community |
| Neo4j Rust driver | neo4rs |
| LLM providers | Claude (Anthropic) · OpenAI-compatible |
| Streaming | Server-Sent Events (axum SSE) |
| Web UI build | Vite |
| Web UI tests | Vitest (jsdom) |
| CSS framework | Canonical Vanilla Framework |
| Async runtime | tokio |
| Configuration | TOML |

---

## Running tests

```bash
# Rust unit + integration tests (no Docker needed)
cargo test

# Rust Docker-gated tests (Neo4j testcontainers)
cargo test -- --include-ignored

# Web UI tests
cd web-ui && npm test
```

---

## Documentation

Detailed documentation lives under [`documentation/developer/`](documentation/developer/):

- [`architecture.md`](documentation/developer/architecture.md) — system design and component overview
- [`harvester.md`](documentation/developer/harvester.md) — pipeline, graph schema, tree-sitter integration
- [`server.md`](documentation/developer/server.md) — API reference, LLM provider config, agentic loop
- [`dev-setup.md`](documentation/developer/dev-setup.md) — step-by-step local development setup
- [`web-ui/README.md`](web-ui/README.md) — web UI architecture, scripts, and test coverage

---

## License

[MIT](LICENSE)
