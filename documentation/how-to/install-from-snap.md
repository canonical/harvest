# Install Harvest from the snap

**Goal:** a running Harvest server and web UI on one Ubuntu host.

You need `snapd` and a PostgreSQL server (see [Prepare PostgreSQL](prepare-postgresql.md)).

```bash
sudo snap install harvest --edge
harvest prepare-db-script | sudo bash -x   # optional: creates a local database, prints database.url
sudo harvest init                          # guided setup: database, models, sign-in, ports
harvest status
```

`harvest init` writes the configuration, enables both services and starts them:

| Service | Purpose | Default address |
|---|---|---|
| `harvest.server` | API and agents (`knowledge-server`) | `127.0.0.1:8080` |
| `harvest.ui` | Web UI and proxy to the API | `127.0.0.1:3000` |

To expose the UI on the network:

```bash
sudo harvest ui set-host 0.0.0.0
```

`harvest init --from-db-url <url>` skips the database question.

Check health at any time with `harvest status` (or `harvest status --json`). It probes `/health` on the server and `/ui-health` on the UI.

Next: [Configure Harvest](configure-harvest.md).
