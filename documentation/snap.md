# Installing and Configuring Harvest via Snap

This guide covers installing the Harvest snap and configuring it for production use.

---

## Prerequisites

Harvest requires a running Neo4j instance. The snap does **not** bundle Neo4j. Install it separately before proceeding:

```bash
# Option A — Docker (quickest)
docker compose up -d
# Neo4j browser: http://localhost:7474  (neo4j / devpassword - change password)

# Option B — apt (official Neo4j repo)
curl -fsSL https://debian.neo4j.com/neotechnology.gpg.key | sudo gpg --dearmor -o /etc/apt/keyrings/neo4j.gpg
echo "deb [signed-by=/etc/apt/keyrings/neo4j.gpg] https://debian.neo4j.com stable latest" | sudo tee /etc/apt/sources.list.d/neo4j.list
sudo apt update && sudo apt install neo4j
sudo systemctl enable --now neo4j
# Neo4j browser: http://localhost:7474  (neo4j / neo4j - change password)
```

---

## Installation

```bash
sudo snap install harvest --edge
```

After installation, two daemons (serve and ui) are registered but left **disabled** until you configure them.
A one shot command is also available to ingest code repositories into harvest (ingest-code).

| Service / command | Purpose |
|-------------------|---------|
| `harvest.server` | The `knowledge-server` API (port 8080 by default) |
| `harvest.ui` | nginx serving the web UI and proxying to the API (port 3000 by default) |
| `harvest.ingest-code` | One-shot repository ingestion into the knowledge graph |

---

## Configuration

All configuration is managed through `snap set`. No config files need to be edited manually.

### Required keys

```bash
sudo snap set harvest \
  neo4j.password=<neo4j-password> \
  llm.<name>.provider=anthropic \
  llm.<name>.api-key=<llm-api-key> \
  auth.jwt-secret=$(openssl rand -base64 32)
```

| Key | Description |
|-----|-------------|
| `neo4j.password` | Password for the Neo4j database |
| `llm.<name>.provider` | LLM backend: `anthropic`, `gemini`, or `openai-compatible` |
| `llm.<name>.api-key` | API key for the LLM provider |
| `auth.jwt-secret` | Secret used to sign JWT session cookies — generate with `openssl rand -base64 32` |

### Neo4j connection

```bash
sudo snap set harvest \
  neo4j.host=localhost \
  neo4j.port=7687 \
  neo4j.user=neo4j
```

| Key | Default | Description |
|-----|---------|-------------|
| `neo4j.host` | `localhost` | Neo4j hostname |
| `neo4j.port` | `7687` | Neo4j Bolt port |
| `neo4j.user` | `neo4j` | Neo4j username |

### Server

```bash
sudo snap set harvest \
  server.host=127.0.0.1 \
  server.port=8080
```

| Key | Default | Description |
|-----|---------|-------------|
| `server.host` | `127.0.0.1` | API bind address — use `0.0.0.0` to listen on all interfaces |
| `server.port` | `8080` | API port |

### Web UI

```bash
sudo snap set harvest \
  ui.host=0.0.0.0 \
  ui.port=3000
```

| Key | Default | Description |
|-----|---------|-------------|
| `ui.host` | `127.0.0.1` | Web UI bind address |
| `ui.port` | `3000` | Web UI port |

### LLM providers

Harvest supports multiple LLM providers simultaneously. Each provider is identified by a user-chosen name (e.g., `primary`, `fallback`). On rate-limit errors, providers are tried in priority order (lower value = higher priority).

```bash
# Single provider — Anthropic Claude
sudo snap set harvest \
  llm.primary.provider=anthropic \
  llm.primary.api-key=sk-ant-...

# Two providers — Anthropic primary, Gemini fallback
sudo snap set harvest \
  llm.primary.provider=anthropic \
  llm.primary.api-key=sk-ant-... \
  llm.primary.priority=0 \
  llm.fallback.provider=gemini \
  llm.fallback.api-key=AIza... \
  llm.fallback.priority=1

# OpenAI-compatible endpoint (Groq, Ollama, etc.)
sudo snap set harvest \
  llm.primary.provider=openai-compatible \
  llm.primary.api-key=gsk_... \
  llm.primary.model=llama-3.3-70b-versatile \
  llm.primary.base-url=https://api.groq.com/openai/v1
```

| Key | Default | Description |
|-----|---------|-------------|
| `llm.<name>.provider` | — | `anthropic`, `gemini`, or `openai-compatible` |
| `llm.<name>.api-key` | — | API key (setting this enables the provider) |
| `llm.<name>.model` | — | Model name (e.g. `claude-sonnet-4-6`) |
| `llm.<name>.base-url` | — | Base URL — required for `openai-compatible` |
| `llm.<name>.timeout-secs` | `120` | Per-request timeout |
| `llm.<name>.max-retries` | `3` | Retry limit per request |
| `llm.<name>.priority` | `0` | Lower = tried first on rate limits |

### Agent settings

```bash
sudo snap set harvest \
  agent.max-iterations=20 \
  agent.compaction-threshold-chars=40000 \
  agent.compaction-keep-last=6 \
  agents.public-url=https://harvest.example.com
```

| Key | Default | Description |
|-----|---------|-------------|
| `agent.max-iterations` | `20` | Maximum LLM iterations per query |
| `agent.compaction-threshold-chars` | `40000` | Context length that triggers history compaction |
| `agent.compaction-keep-last` | `6` | Messages retained after compaction |
| `agents.public-url` | — | Public URL used for remote agent install scripts |

### Authentication

```bash
sudo snap set harvest \
  auth.allow-local-login=true \
  auth.google.client-id=<id> \
  auth.google.client-secret=<secret> \
  auth.google.redirect-uri=https://harvest.example.com/auth/google/callback
```

| Key | Default | Description |
|-----|---------|-------------|
| `auth.allow-local-login` | `true` | Enable username/password login |
| `auth.jwt-secret` | — | JWT signing secret (required) |
| `auth.google.client-id` | — | Google OAuth client ID |
| `auth.google.client-secret` | — | Google OAuth client secret |
| `auth.google.redirect-uri` | — | Google OAuth redirect URI |
| `auth.oidc.issuer-url` | — | OIDC provider URL |
| `auth.oidc.client-id` | — | OIDC client ID |
| `auth.oidc.client-secret` | — | OIDC client secret |
| `auth.oidc.redirect-uri` | — | OIDC redirect URI |
| `auth.oidc.display-name` | — | OIDC provider display name (shown on login page) |

### Harvester (ingest-code)

The harvester reuses the same `neo4j.*` and `llm.*` keys as the server — no separate credentials are needed. Repositories are cloned to a temporary directory and deleted automatically after ingestion.

### Logging

```bash
sudo snap set harvest log.level=info
```

| Key | Default | Description |
|-----|---------|-------------|
| `log.level` | `warn` | Log verbosity: `error`, `warn`, `info`, `debug`, or `trace` |

### Documentation (optional)

```bash
sudo snap set harvest \
  docs.dir=/var/snap/harvest/common/docs \
  ui.enable-docs=true
```

| Key | Default | Description |
|-----|---------|-------------|
| `docs.dir` | `$SNAP_COMMON/docs` | Directory containing generated Diataxis docs |
| `ui.enable-docs` | `false` | Show the documentation page in the web UI |

---

## Starting the services

```bash
sudo snap start --enable harvest.server
sudo snap start --enable harvest.ui
```

Open `http://localhost:3000` in your browser. The first user to register becomes an admin.

---

## Verifying the installation

```bash
# Check service status
sudo snap services harvest

# Test the API health endpoint
curl http://localhost:8080/health
# → {"status":"ok"}

# Check server logs
sudo snap logs harvest.server

# Check UI / nginx logs
sudo snap logs harvest.ui
```

Log files are also written to:

| File | Contents |
|------|----------|
| `/var/snap/harvest/common/nginx-access.log` | nginx access log |
| `/var/snap/harvest/common/nginx-error.log` | nginx error log |

---

## Log verbosity

By default, only warnings and errors are logged (`warn`). To see informational or debug output, set the `log.level` key:

```bash
sudo snap set harvest log.level=info
sudo snap restart harvest.server
```

| Value | When to use |
|-------|-------------|
| `error` | Errors only |
| `warn` | Warnings and errors (default) |
| `info` | Operational progress — connections, queries, ingestion steps |
| `debug` | Verbose internal state |
| `trace` | Maximum verbosity |

For the one-shot ingest command you can also override the level inline without restarting any service:

```bash
RUST_LOG=info sudo harvest.ingest-code
```

---

## Stopping and restarting

```bash
sudo snap stop harvest.server
sudo snap stop harvest.ui

sudo snap restart harvest.server
sudo snap restart harvest.ui
```

---

## Updating configuration at runtime

Changes made with `snap set` take effect on the next service restart:

```bash
sudo snap set harvest server.port=9090
sudo snap restart harvest.server
sudo snap restart harvest.ui   # the UI proxy config is regenerated on startup
```

---

## Uninstalling

```bash
sudo snap remove harvest
```

Snap data (logs, generated nginx config) stored under `/var/snap/harvest/` is removed automatically. Your Neo4j data is unaffected.

---

## Ingest repositories with the harvester

The snap includes `harvest.ingest-code`, a one-shot command that ingests configured repositories into the knowledge graph. It reuses the Neo4j and LLM credentials already set via `snap set`.

### 1. Write a repositories file

Create `/var/snap/harvest/common/repositories.toml` with one `[[repositories]]` block per repository:

```toml
[[repositories]]
name = "my-repo"
url  = "https://github.com/owner/my-repo.git"
refs = ["main"]

[[repositories]]
name = "another-repo"
url  = "https://github.com/owner/another-repo.git"
refs = ["main", "v2.0"]
```

### 2. Run ingestion

```bash
sudo harvest.ingest-code
```

To force re-ingestion of versions already processed:

```bash
sudo harvest.ingest-code --force
```

To use a repositories file in a different location:

```bash
sudo harvest.ingest-code -f /path/to/repositories.toml
```

Once ingestion completes, repositories appear in the web UI immediately.

---

## Exposing Harvest publicly

By default both services bind to `127.0.0.1`. To expose them:

```bash
# Expose the web UI on all interfaces
sudo snap set harvest ui.host=0.0.0.0

# Keep the API local (the UI proxy handles all browser traffic)
# — no change needed for server.host

sudo snap restart harvest.ui
```

For HTTPS, place a reverse proxy (nginx, Caddy, Traefik) in front of port 3000. Set `agents.public-url` to the public HTTPS address so that remote agent install scripts use the correct callback URL.
