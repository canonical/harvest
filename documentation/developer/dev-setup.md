# Development Setup

## Prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| Rust (stable) | Build both crates | `curl https://sh.rustup.rs -sSf \| sh` |
| Docker + Compose | Run Neo4j locally | docker.com/get-docker |
| Node.js ≥ 20 | Run the web UI | [nodejs.org](https://nodejs.org) or `nvm install 22` |
| `cargo-watch` (optional) | Auto-rebuild on save | `cargo install cargo-watch` |

---

## Step 1 — Start Neo4j

```bash
# From the repo root
docker compose up -d

# Wait until healthy (~20 s)
docker compose ps
```

The Neo4j Browser UI is at http://localhost:7474 (login: `neo4j` / `devpassword`).  
Apps connect on the Bolt port `localhost:7687`.

---

## Step 2 — Configure and run the harvester

Edit [knowledge-harvester/harvester.toml](../../knowledge-harvester/harvester.toml):

```toml
[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "devpassword"

[storage]
clone_root = "/tmp/harvest-repos"   # created automatically

[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/repo.git"
# Optional: pin specific refs instead of all tags
# refs = ["v2.0.0", "v2.1.0", "main"]
```

Run the harvester:

```bash
cd knowledge-harvester

# Single harvest pass (recommended for first run)
RUST_LOG=info cargo run -- --config harvester.toml run

# Watch mode: re-check for new refs every 5 min
RUST_LOG=info cargo run -- --config harvester.toml watch --interval-secs 300

# Check ingestion status
cargo run -- --config harvester.toml status

# Mark all versions as pending (force full re-ingest on next run)
cargo run -- --config harvester.toml reingest
```

The harvester will:
1. Clone each repo under `clone_root`
2. Enumerate git refs (all tags, or the explicit `refs` list)
3. Skip versions already marked `ingested: true` in Neo4j
4. Parse source files with tree-sitter
5. Write nodes and relationships (calls, inherits, implements, embeds, uses) to Neo4j
6. Set `ingested: true` on the `Version` node when done

---

## Step 3 — Configure and start the server

Edit [knowledge-server/server.toml](../../knowledge-server/server.toml):

```toml
[server]
host = "127.0.0.1"
port = 8080

[neo4j]
uri      = "bolt://localhost:7687"
user     = "neo4j"
password = "devpassword"

[llm]
provider       = "anthropic"
model          = "claude-sonnet-4-6"
api_key        = "sk-ant-..."
max_iterations = 20
```

For Groq / Ollama instead:

```toml
[llm]
provider  = "openai-compatible"
base_url  = "https://api.groq.com/openai/v1"   # or http://localhost:11434/v1
api_key   = "gsk_..."
model     = "llama-3.3-70b-versatile"
```

```bash
cd knowledge-server

RUST_LOG=info cargo run -- --config server.toml
# Listening on 127.0.0.1:8080

# With auto-rebuild on save:
cargo watch -x 'run -- --config server.toml'
```

---

## Step 4 — Generate documentation (optional)

The `document` command generates [Diataxis](https://diataxis.fr/)-structured documentation for an ingested version. It requires `[llm]` and `[documentation]` sections in `harvester.toml`:

```toml
[llm]
provider = "anthropic"
model    = "claude-sonnet-4-6"
api_key  = "sk-ant-..."

[documentation]
docs_dir = "/tmp/harvest-docs"
```

```bash
cd knowledge-harvester
RUST_LOG=info cargo run -- --config harvester.toml document my-repo:v1.2.0
```

To serve these pages through the web UI, add the same `docs_dir` to `server.toml`:

```toml
[documentation]
docs_dir = "/tmp/harvest-docs"
```

Restart the server. The **Document** tab in the web UI will show the generated documentation.

---

## Step 5 — Start the web UI

```bash
cd web-ui
npm install
npm run dev
# Open http://localhost:5173
```

The Vite dev server proxies API calls to `localhost:8080` automatically — all `/query`, `/query/stream`, `/repositories`, `/graph`, `/docs`, and `/health` requests are forwarded. The knowledge-server (Step 3) must be running first.

---

## Step 6 — Verify everything works

### Health check
```bash
curl http://localhost:8080/health
# {"status":"ok"}
```

### List ingested repositories
```bash
curl http://localhost:8080/repositories | jq
```

### Ask a question
```bash
curl -s http://localhost:8080/query \
  -H 'Content-Type: application/json' \
  -d '{"query": "How does the retry logic work?"}' \
  | jq '{answer: .answer, sources: .sources, tool_calls_made: .tool_calls_made}'
```

### Fetch the symbol graph
```bash
curl "http://localhost:8080/graph/my-repo/v1.2.0" | jq '{nodes: (.nodes | length), edges: (.edges | length), truncated}'
```

### Browse the web UI pages

Open http://localhost:5173 and use the sidebar to switch between:

- **Chat** — submit a natural-language question and watch tool calls stream in real time
- **Explore** — select a repo and version to see the interactive symbol graph; click a node to open the source panel; use the search box to find symbols
- **Document** — select a repo and version to read the AI-generated documentation (requires Step 4)

---

## Running everything together (split terminals)

```
Terminal 1:  docker compose up
Terminal 2:  cd knowledge-harvester && RUST_LOG=info cargo run -- --config harvester.toml run
Terminal 3:  cd knowledge-server    && RUST_LOG=info cargo run -- --config server.toml
Terminal 4:  cd web-ui && npm run dev   # http://localhost:5173
```

Or with `tmux`:

```bash
tmux new-session -d -s harvest
tmux send-keys -t harvest 'docker compose up' Enter
tmux split-window -h -t harvest
tmux send-keys -t harvest 'cd knowledge-harvester && RUST_LOG=info cargo run -- --config harvester.toml run' Enter
tmux split-window -v -t harvest
tmux send-keys -t harvest 'cd knowledge-server && RUST_LOG=info cargo run -- --config server.toml' Enter
tmux split-window -h -t harvest
tmux send-keys -t harvest 'cd web-ui && npm run dev' Enter
tmux attach -t harvest
```

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

## Logging

Both binaries read `RUST_LOG`. Useful values:

| Value | What you see |
|-------|-------------|
| `error` | Failures only |
| `info` | Progress milestones (default) |
| `debug` | Per-file parsing, every Cypher query |
| `trace` | Full LLM request/response bodies |

Example: `RUST_LOG=knowledge_server=debug,tower_http=info cargo run`

---

## Resetting Neo4j

To wipe the graph and start fresh:

```bash
docker compose down -v   # destroys the neo4j_data volume
docker compose up -d
```
