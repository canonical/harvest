# Read logs and change verbosity

```bash
sudo snap logs harvest.server -n 200     # API server
sudo snap logs harvest.ui -n 200         # web UI proxy
```

Change the level (`error`, `warn`, `info`, `debug`, `trace`; default `warn`):

```bash
sudo harvest config set log.level=info
```

Outside the snap, set `RUST_LOG`, for example `RUST_LOG=info,knowledge_server=debug`, and read the service's journal (`journalctl -u <service>`).

Useful messages:

| Message | Meaning |
|---|---|
| `applied database migration version=N` | schema upgraded on start |
| `LXD client identity successfully joined via trust token` | Code mode connected to LXD |
| `code image build failed project=...` | a project's container image failed; see the project page's **Container setup** |
| `could not refresh a code project's index` | the keeper could not fetch a project's branch |
