# Harvester CLI

`knowledge-harvester` indexes git repositories into the database. Snap: `harvest.ingest-code` runs `run` with `/var/snap/harvest/common/harvester.toml` and `repositories.toml`.

```text
knowledge-harvester [-c <config>] <command>
```

| Command | Does |
|---|---|
| `run [--force]` | ingest every configured repository once; `--force` redoes versions already ingested |
| `watch [--interval-secs 300]` | poll repositories and ingest new versions continuously |
| `reingest` | mark every ingested version as pending, so the next `run` processes it again |
| `status` | versions and their ingestion state |
| `document <repository:version>` | generate Diátaxis documentation for a version into `docs_dir` |

`-c, --config` defaults to `harvester.toml`.

## `harvester.toml`

```toml
[database]
url = "postgresql://harvest:secret@127.0.0.1:5432/harvest"

[git]                                  # optional, for SSH URLs
ssh_key_path   = "/etc/harvest/deploy_key"
ssh_passphrase = "…"                   # optional

[[repositories]]
name       = "my-service"              # unique name shown in Harvest
url        = "https://github.com/acme/my-service.git"
browse_url = "https://github.com/acme/my-service"   # optional; links in citations
refs       = ["main", "v2.0"]          # optional; all tags when omitted

[documentation]                        # optional, for `document`
docs_dir = "/var/lib/harvest/docs"
```

An optional `[llm]` section (same fields as the server's `[[llm]]`) is used by `document`.
