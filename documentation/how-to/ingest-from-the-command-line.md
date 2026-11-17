# Ingest repositories from the command line

**Goal:** index repositories in bulk or on a schedule, without the web UI.

Write a repositories file (snap: `/var/snap/harvest/common/repositories.toml`):

```toml
[[repositories]]
name = "my-service"
url  = "https://github.com/acme/my-service.git"
refs = ["main", "v2.0"]

[[repositories]]
name = "internal-lib"
url  = "git@git.acme.internal:platform/internal-lib.git"
```

Without `refs`, every tag is ingested.

```bash
sudo harvest.ingest-code                 # snap: ingest once
sudo harvest.ingest-code --force         # re-ingest versions already done
sudo harvest.ingest-code -f other.toml   # another repositories file
```

Outside the snap, use `knowledge-harvester` with a `harvester.toml` that has `[database] url` and the `[[repositories]]` blocks:

```bash
knowledge-harvester -c harvester.toml run
knowledge-harvester -c harvester.toml watch --interval-secs 300   # poll for new versions
knowledge-harvester -c harvester.toml status
```

For SSH repositories add `[git] ssh_key_path = "/path/to/key"`. All commands: [Harvester CLI](../reference/harvester-cli.md).

Repositories ingested this way are marked public, so every Harvest user can query them. Index private code from the web UI instead, where access follows each user's credentials.
