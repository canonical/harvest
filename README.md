# Harvest

Harvest is an operator-focused platform for querying and operating software systems with AI.

It includes:

- `knowledge-server` (API + orchestration)
- `web-ui` (chat and operations UI)
- `harvest-agent` (remote command execution on managed machines)
- `knowledge-harvester` (repository ingestion)
- `harvest` CLI (installation-time and post-init configuration)

Harvest uses **PostgreSQL** as the system database.

---

## What Harvest is for

Use Harvest when you want to:

- ask system and code questions with grounded context,
- run controlled operations through connected agents,
- manage providers, auth, and runtime config without re-running installation.

---

## Quick start (recommended: snap)

### 1) Prerequisites

- Ubuntu host with `snapd`
- PostgreSQL reachable from the host
- Database URL (example):

```text
postgresql://harvest:<password>@<host>:5432/harvest
```

For a quick single-node PostgreSQL setup suitable for testing, you can use:

```bash
harvest prepare-db-script | sudo bash -x
```

The script prints a ready-to-use `database.url` value.

### 2) Install

From store channel:

```bash
sudo snap install harvest --edge
```

### 3) Initialize

```bash
sudo harvest init
```

### 4) Verify

```bash
harvest status
```

---

## Post-init configuration

You can fully manage runtime config after init.

Top-level groups:

```bash
harvest llm ...
harvest identity ...
harvest auth ...
harvest security ...
harvest database ...
harvest server ...
harvest ui ...
harvest agents ...
harvest config ...
harvest cluster ...
```

### Common operations

List LLM providers:

```bash
harvest llm provider list
```

Add a provider with user-provided key mode:

```bash
sudo harvest llm provider add --id primary --provider anthropic --user-provided-key
```

Set or rotate JWT secret:

```bash
sudo harvest auth jwt rotate --auto
```

Set database URL:

```bash
sudo harvest database set-url 'postgresql://harvest:<password>@<host>:5432/harvest'
```

Validate current config:

```bash
harvest config validate
```

Apply pending config and restart services:

```bash
sudo harvest config apply
```

---

## User-provided key mode

Harvest supports provider entries with:

```text
user-provided-key=true
```

In this mode:

- provider API key storage is intentionally empty,
- server/provider blocks are still generated and valid,
- live model detection is skipped when no static key is available,
- cached models are used when available.

---

## Repository layout

```text
harvest/
├── harvest-cli/          # Operator CLI (init + lifecycle config commands)
├── knowledge-server/     # API server and orchestration
├── knowledge-harvester/  # Repository ingestion
├── agent/                # Remote execution agent
├── web-ui/               # Frontend
├── snap/                 # Snap packaging hooks/tests
└── documentation/        # Product and developer docs
```

---

## Build and test

Rust workspace tests:

```bash
cargo test
```

Build snap (on supported host setup):

```bash
snapcraft pack --destructive-mode
```

---

## Troubleshooting

- Run init as root inside snap context:

```bash
sudo harvest init
```

- If any LLM provider uses:

```text
user-provided-key=true
```

set:

```text
security.user-key-encryption-key
```

- Validate before apply:

```bash
harvest config validate
sudo harvest config apply
```
- For snap install/runtime issues, check:

```bash
sudo journalctl -u snap.harvest.server -f
sudo journalctl -u snap.harvest.ui -f
```

---

## Documentation

- Snap installation and operations: [`documentation/snap.md`](documentation/snap.md)
- Architecture: [`documentation/developer/architecture.md`](documentation/developer/architecture.md)
- Server internals: [`documentation/developer/server.md`](documentation/developer/server.md)
- Harvester internals: [`documentation/developer/harvester.md`](documentation/developer/harvester.md)
- Dev setup: [`documentation/developer/dev-setup.md`](documentation/developer/dev-setup.md)

---

## License

[MIT](LICENSE)
