# Development Setup

## Prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| Rust (stable) | Build both crates | `curl https://sh.rustup.rs -sSf \| sh` |
| Docker + Compose | Run Neo4j locally | docker.com/get-docker |
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

## Step 2 — Configure the harvester

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
```

Add more `[[repositories]]` blocks for each repo you want to index.

---

## Step 3 — Run the harvester

```bash
cd knowledge-harvester

# Single harvest pass (recommended for first run)
RUST_LOG=info cargo run -- --config harvester.toml run

# Watch mode: re-check for new tags every 5 min
RUST_LOG=info cargo run -- --config harvester.toml watch --interval-secs 300

# Check ingestion status
cargo run -- --config harvester.toml status
```

The harvester will:
1. Clone each repo under `clone_root`
2. Enumerate git tags
3. Skip versions already marked `ingested: true` in Neo4j
4. Parse source files with tree-sitter
5. Write nodes + relationships to Neo4j
6. Set `ingested: true` on the Version node when done

---

## Step 4 — Configure the server

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
provider  = "openai-compat"
base_url  = "https://api.groq.com/openai/v1"   # or http://localhost:11434/v1
api_key   = "gsk_..."
model     = "llama-3.3-70b-versatile"
```

---

## Step 5 — Run the server

```bash
cd knowledge-server

RUST_LOG=info cargo run -- --config server.toml
# Listening on 127.0.0.1:8080
```

With auto-rebuild on save:

```bash
cargo watch -x 'run -- --config server.toml'
```

---

## Step 6 — Test the API

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
  -d '{"query": "How does the regex engine handle Unicode character classes?"}' \
  | jq '{answer: .answer, sources: .sources, tool_calls_made: .tool_calls_made}'
```

Optional filters (not yet wired to the agent, reserved for future use):
```json
{
  "query": "...",
  "repositories": ["my-repo"],
  "versions": ["v1.2.0"]
}
```

---

## Running both together (split terminals)

```
Terminal 1:  docker compose up          # Neo4j logs
Terminal 2:  cd knowledge-harvester && RUST_LOG=info cargo run -- run
Terminal 3:  cd knowledge-server    && RUST_LOG=info cargo run
Terminal 4:  curl / httpie / Postman
```

Or with `tmux`:

```bash
tmux new-session -d -s harvest
tmux send-keys -t harvest 'docker compose up' Enter
tmux split-window -h -t harvest
tmux send-keys -t harvest 'cd knowledge-harvester && RUST_LOG=info cargo run -- --config harvester.toml run' Enter
tmux split-window -v -t harvest
tmux send-keys -t harvest 'cd knowledge-server && RUST_LOG=info cargo run -- --config server.toml' Enter
tmux attach -t harvest
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

---

## Known issues (pre-release)

- **P1 compile blocker** in `knowledge-server/src/agent/mod.rs:57` — `tool_map` key borrows from a temporary. Fix: change `HashMap<&str, &dyn Tool>` to `HashMap<String, &dyn Tool>`. Neither binary will compile until this is resolved.
- **Call edge writes** are not yet implemented in the harvester (`graph/writer.rs`). `CALLS` relationships won't appear in the graph until P2 is fixed.
- Language parsers other than Rust return empty results (P4-P9).
