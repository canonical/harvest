# Back up and restore Harvest

**Goal:** be able to rebuild Harvest after losing the host.

Back up these; everything else is rebuilt automatically:

| What | Where | How |
|---|---|---|
| Database: users, chats, projects, the code graph, encrypted credentials | PostgreSQL | `pg_dump -Fc harvest > harvest.dump` |
| Configuration | snap: `/var/snap/harvest/common/server.toml` and snap settings | `harvest config export > harvest-config.json` and copy the file |
| `security.user_key_encryption_key` | in the configuration | store it separately and safely |
| LXD configuration (Code mode) | `/var/snap/lxd/common/lxd/database` | copy while LXD is stopped, or `lxd recover` later |

Coding containers, images and preview copies are disposable: Harvest rebuilds images and warm containers on its own. Uncommitted work inside a running session is lost if its container is.

## Restore

```bash
createdb -O harvest harvest
psql harvest -c 'CREATE EXTENSION vector; CREATE EXTENSION pg_trgm;'   # as a superuser
pg_restore -d harvest harvest.dump
sudo harvest config import harvest-config.json      # snap settings, if exported
sudo harvest config apply
```

Restore the same `user_key_encryption_key`; with a different key, stored model keys and git credentials cannot be read and users must enter them again.
