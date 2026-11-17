# Configuration file (`server.toml`)

`knowledge-server --config <file>` reads this TOML file (default `server.toml`). Snap installs generate it at `/var/snap/harvest/common/server.toml` (see [Snap settings](snap-and-cli.md)). Validate with `knowledge-server --config <file> check-config`. Restart after changes.

Required: `[database] url` and `[auth] jwt_secret`. Every other key has a default.

## `[server]`

| Key | Default | Meaning |
|---|---|---|
| `host` | `0.0.0.0` | listen address |
| `port` | `8080` | listen port |

## `[database]`

| Key | Default | Meaning |
|---|---|---|
| `url` | required | `postgresql://user:password@host:5432/db` |
| `pool_size` | `16` | connections per server |
| `ca_file` | none | CA certificate for TLS to PostgreSQL |
| `migrate_on_start` | `true` | apply migrations on start |

## `[auth]`

| Key | Default | Meaning |
|---|---|---|
| `jwt_secret` | required | signs session tokens; changing it signs everyone out |
| `allow_local_login` | `true` | email and password accounts |
| `public_url` | none | Harvest's public address, for redirects and preview framing |
| `[auth.google]` | none | `client_id`, `client_secret`, `redirect_uri` |
| `[auth.oidc]` | none | `issuer_url`, `client_id`, `client_secret`, `redirect_uri`, `display_name` |

## `[security]`

| Key | Default | Meaning |
|---|---|---|
| `user_key_encryption_key` | none | 64 hex characters (32 bytes). Encrypts users' model keys, SSH keys and git tokens (AES-256-GCM). Required when any provider has `user_provided_key = true`; without it, **Settings → Git access** is unavailable |

## `[agent]`

| Key | Default | Meaning |
|---|---|---|
| `max_iterations` | `20` | model calls per answer before the agent must stop |
| `compaction_threshold_chars` | `40000` | conversation size that triggers summarising older messages |
| `compaction_keep_last` | `6` | recent messages kept verbatim when compacting |
| `max_active_turns` | `200` | answers in progress at once, server-wide |
| `max_active_turns_per_user` | `10` | answers in progress at once, per user |

## `[[llm]]`

One block per provider.

| Key | Default | Meaning |
|---|---|---|
| `provider` | required | `anthropic`, `gemini`, `openai-compatible` |
| `id` | provider name | identifier used by the UI, API and `[search.fast_model]` |
| `name` | provider name | label in the model picker |
| `model` | required | default model |
| `models` | `[model]` | models users may pick |
| `api_key` | none | server key |
| `user_provided_key` | `false` | each user supplies their own key |
| `base_url` | provider's | required for `openai-compatible` |
| `enable_prompt_caching` | on for Claude models | `openai-compatible` only: mark the prompt for caching. Unset: on when the requested model is a Claude model; `true`/`false` force it |
| `priority` | `0` | failover order, lower first |
| `timeout_secs` | `120` | per request |
| `max_retries` | `3` | on transient errors |
| `max_concurrency` | none | requests in flight to this provider |
| `context_window` | model's | tokens, for compaction |
| `expose_to_ui` | `true` | show in the model picker |
| `[llm.pricing]` | built-in table | micro-dollars per 1,000 tokens: `input_per_1k`, `output_per_1k`, `cache_read_per_1k`, `cache_creation_per_1k`, `reasoning_per_1k`. The built-in table covers current Claude models under any id form (`claude-sonnet-5-5`, `anthropic/claude-sonnet-5.5`, `anthropic.claude-sonnet-5-5`) |

## `[agents]`

| Key | Default | Meaning |
|---|---|---|
| `public_url` | `auth.public_url` | address machine agents and install commands use |
| `binary_path` | bundled | `harvest-agent` binary served to installers |

## `[lxd]`

Enables Code mode and LXD-managed agents.

| Key | Default | Meaning |
|---|---|---|
| `endpoint` | none | `https://host:8443` |
| `trust_token` | none | single-use token from `lxc config trust add`; used once to enrol |
| `client_cert`, `client_key` | generated | PEM; generated and stored in the database when absent |
| `ca_cert` | none | PEM of a self-signed LXD server |
| `insecure` | `false` | skip server certificate checks |
| `project` | `default` | LXD project |
| `profile` | `default` | profile for new containers |
| `image_server` | `https://cloud-images.ubuntu.com/releases` | base image server |
| `image_alias` | `24.04` | base image |

## `[code]`

| Key | Default | Meaning |
|---|---|---|
| `pool_size` | `2` | warm containers per project |
| `warm_days` | `7` | keep a project warm this long after last use |
| `idle_hours` | `24` | end idle sessions |
| `max_preview_copies` | `5` | viewer copies per preview |
| `reindex_hours` | `6` | branch freshness check interval |
| `preview_domain` | none | web previews at `p-<id>.<preview_domain>`; web previews are off without it |
| `preview_scheme` | `https` | `https` or `http` |
| `preview_ttl_hours` | `24` | delete previews unused this long |
| `review_changes` | `true` | after a turn that adds commits, a reviewer agent reads the change and posts its findings in the chat |
| `working_set_chars` | `24000` | characters of the files the previous turn read or edited, given to the next turn so it does not read them again; `0` turns it off |

## `[search]`

| Key | Default | Meaning |
|---|---|---|
| `engine` | `loop` | `loop`, `tiered` or `shadow` |
| `deadline_ms` | `7000` | tiered: time budget per search |
| `token_budget` | `8000` | tiered: tokens returned per search |
| `fast_model` | none | `{ provider_id, model }` for tiered rounds and enrichment |
| `tier3_turns` | `6` | tiered: fast-model turns in the last round |
| `enrich` | `false` | summarise symbols in the background |
| `max_followups` | `3` | tiered: follow-up searches |
| `prefetch` | `true` | warm likely searches |
| `calibrate` | `true` | adjust tier thresholds from results |
| `warm` | `[]` | `"repo@version"` kept in memory |
| `refresh_secs` | `30` | cache refresh interval |
| `cache_dir` | none | on-disk cache |
| `judge`, `reranker`, `gate` | none | optional model stages |

## `[semantic]`

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | embeddings for semantic search (needs pgvector and a Gemini key) |
| `model` | `text-embedding-004` | embedding model |
| `dimensions` | `768` | vector size |
| `lexical_weight` / `semantic_weight` | `0.6` / `0.4` | score blend |
| `backfill_on_start` | `false` | embed existing symbols on start |
| `batch_size` | `64` | symbols per request |

## `[cluster]`

| Key | Default | Meaning |
|---|---|---|
| `node_name` | generated | this node's name |
| `internal_listen` | none | address for node-to-node traffic, e.g. `0.0.0.0:8081` |
| `internal_url` | none | how other nodes reach this one |
| `shared_secret` | none | authenticates nodes; `accepted_secrets` lists extra ones during rotation |
| `backend` | `postgres` | `postgres` or `quorium` |
| `bus` | `notify` (`mesh` with quorium) | event bus |
| `heartbeat_interval_ms` | `5000` | |
| `node_timeout_ms` | `20000` | at least twice the heartbeat |
| `bus_batch_window_ms` | `50` | |
| `drain_grace_secs` / `drain_timeout_secs` | `10` / `120` | on shutdown |
| `data_dir` | none | quorium state |
| `bootstrap` | none | quorium: `init` or `join` |
| `join_token`, `join_addresses` | none | quorium joining |
| `election_timeout_ms` | `1000` | quorium |
| `join_timeout_secs` | `60` | quorium |
| `snapshot_logs` | `1000` | quorium |
| `replace_absent_voter_after_secs` | `0` (never) | quorium |
| `[cluster.tls]` | none | mutual TLS between nodes: `ca_cert`, `cert`, `key` (file paths) |

## Other sections

| Section | Keys |
|---|---|
| `[ui]` | `enable_docs`: serve the documentation in the web UI |
| `[documentation]` | `docs_dir`: where generated documentation is kept |
| `[collocate]` | sandbox containers for Deploy chat: `endpoint`, `trust_token`, `project` (`harvest`), `default_image` (`ubuntu:24.04`), `default_idle_timeout_secs` (`600`), `max_containers` (`20`), TLS keys |

## Example

```toml
[server]
port = 8080

[database]
url = "postgresql://harvest:secret@127.0.0.1:5432/harvest"

[auth]
jwt_secret = "change-me"
public_url = "https://harvest.example.com"

[security]
user_key_encryption_key = "0123…(64 hex)"

[[llm]]
provider = "anthropic"
id = "anthropic"
model = "claude-sonnet-5-5"
user_provided_key = true

[lxd]
endpoint = "https://127.0.0.1:8443"
trust_token = "…"
insecure = true

[code]
preview_domain = "preview.harvest.example.com"
```
