# Snap settings and the `harvest` CLI

## Snap applications

| Command | What |
|---|---|
| `harvest` | operator CLI (this page) |
| `harvest.server` | API server daemon (`knowledge-server serve`) |
| `harvest.ui` | web UI daemon; proxies `/api` to the server |
| `harvest.admin` | `knowledge-server` with the snap's config: `migrate`, `check-config`, `schema-version`, `recover-cluster`, `issue-cluster-certificate` |
| `harvest.ingest-code` | ingest `repositories.toml` with the harvester ([how-to](../how-to/ingest-from-the-command-line.md)) |

Files live in `/var/snap/harvest/common/`: `server.toml`, `harvester.toml`, `repositories.toml`.

## Snap settings

The configure hook writes `server.toml` from these settings on every change. `config.managed=true` stops it, so you can edit the file by hand. `[code]`, `[search]`, `[semantic]` have no settings: use `config.managed=true` for them.

| Group | Keys |
|---|---|
| `database.` | `url` (required), `pool-size`, `ca-file` |
| `server.` | `host`, `port` |
| `ui.` | `host` (`127.0.0.1`), `port` (`3000`), `mode` (`proxy`: UI and API; `static`: UI only), `enable-docs` |
| `auth.` | `jwt-secret` (required), `allow-local-login`, `public-url`, `google.client-id`, `google.client-secret`, `google.redirect-uri`, `oidc.issuer-url`, `oidc.client-id`, `oidc.client-secret`, `oidc.redirect-uri`, `oidc.display-name` |
| `security.` | `user-key-encryption-key` |
| `agent.` | `max-iterations`, `compaction-threshold-chars`, `compaction-keep-last` |
| `agents.` | `public-url` |
| `llm.<id>.` | `provider`, `model`, `models`, `api-key`, `base-url`, `display-name`, `priority`, `timeout-secs`, `max-retries`, `expose-to-ui`, `user-provided-key`, `enabled`, `enable-prompt-caching` |
| `lxd.` | `endpoint`, `trust-token`, `client-cert`, `client-key`, `ca-cert`, `insecure`, `project`, `profile`, `image-server`, `image-alias` |
| `cluster.` | `node-name`, `internal-listen`, `internal-url`, `shared-secret`, `accepted-secrets`, `backend`, `bus`, `bootstrap`, `data-dir`, `join-token`, `join-addresses`, `heartbeat-interval-ms`, `node-timeout-ms`, `election-timeout-ms`, `snapshot-logs`, `replace-absent-voter-after-secs`, `tls-ca`, `tls-cert`, `tls-key` |
| other | `docs.dir`, `log.level` (`warn`), `config.managed` |

Meaning of each key: [configuration file](configuration.md) (snap keys use `-` where the file uses `_`).

## `harvest` CLI

Commands that change settings take `--apply-now <true|false>` (default `true`: regenerate and restart), `--no-restart` and `--dry-run`.

### Setup

| Command | Does |
|---|---|
| `init [--from-db-url <url>]` | guided first setup |
| `prepare-db-script [--db-name] [--db-user] [--db-host] [--db-port] [--db-password] [--postgresql-snap]` | print a script that creates a local database |
| `status [--json] [--verbose]` | services and health |
| `show-public-url [--for auto\|auth\|agents\|ui]` | the public URL in use |

### Groups

| Command | Subcommands |
|---|---|
| `llm provider` | `list`, `show <id>`, `add`, `update <id>`, `remove <id>`, `enable <id>`, `disable <id>`, `set-primary <id>`, `reorder <ids>`, `models <id> set\|edit\|clear`, `key <id> set <key>\|clear`, `detect-models <id> [--set-primary] [--set-allowlist]` |
| `identity provider` | `list`, `show`, `enable`, `disable`, `update` — for `google` or `oidc`, with `--client-id`, `--client-secret`, `--redirect-uri`, `--issuer-url`, `--display-name` |
| `auth` | `local-login get\|set <bool>`, `jwt rotate [--auto\|--value <v>]`, `public-url get\|set <url>` |
| `security user-key-encryption-key` | `set <hex>`, `rotate`, `clear` |
| `database` | `get`, `set-url`, `set-pool-size`, `set-ca-file` |
| `server` | `get`, `set-host`, `set-port` |
| `ui` | `get`, `set-host`, `set-port`, `set-mode`, `enable-docs` |
| `agents` | `get`, `set-public-url` |
| `config` | `get <key>`, `set <key=value>…`, `unset <key>`, `list`, `export [--format json\|yaml]`, `import <file>`, `diff --file <file>`, `apply [--no-restart]`, `validate` |
| `cluster` | `init`, `join --token`, `preseed --file`, `status`, `list`, `remove`, `evacuate`, `restore`, `rotate-secrets stage\|commit\|finish`, `token create\|list\|revoke`, `recover` |

`harvest llm provider add` options: `--id`, `--provider anthropic|gemini|open-ai-compatible`, `--model`, `--models`, `--api-key`, `--user-provided-key`, `--base-url`, `--display-name`, `--priority`, `--timeout-secs`, `--max-retries`, `--expose-to-ui`, `--enabled`, `--enable-prompt-caching <true|false>`.

Run any command with `--help` for its options.

## `knowledge-server`

| Subcommand | Does |
|---|---|
| `serve` | run the server (default) |
| `migrate` | apply database migrations |
| `check-config` | validate the configuration |
| `schema-version` | print the database schema version |
| `recover-cluster` | rebuild cluster membership after losing quorum |
| `issue-cluster-certificate --dir --name --host` | issue a node certificate for cluster mutual TLS |

Global option: `--config <file>` (default `server.toml`).
