# Configure Harvest

**Goal:** change Harvest's settings after installation.

## Snap installs

The snap generates `/var/snap/harvest/common/server.toml` from snap settings. Change settings with the `harvest` CLI, then apply:

```bash
harvest config list                         # current settings
sudo harvest config set server.port=9090
harvest config validate
sudo harvest config apply                   # regenerates server.toml and restarts services
```

Most commands apply and restart at once; add `--apply-now false` to batch changes, then run `harvest config apply`.

Dedicated commands exist for common groups: `harvest llm`, `harvest identity`, `harvest auth`, `harvest security`, `harvest database`, `harvest server`, `harvest ui`, `harvest agents`, `harvest cluster`. See [Snap settings and the CLI](../reference/snap-and-cli.md).

Some sections have no snap settings: `[code]`, `[search]`, `[semantic]`, `[system_one]`. To use them, take over the file:

```bash
sudo snap set harvest config.managed=true   # the snap stops regenerating server.toml
sudoedit /var/snap/harvest/common/server.toml
sudo snap restart harvest.server
```

With `config.managed=true` you own the whole file; snap settings no longer change it.

## Other installs

Run the server with a config file:

```bash
knowledge-server --config /etc/harvest/server.toml check-config   # validate
knowledge-server --config /etc/harvest/server.toml serve
```

Every key is listed in the [configuration reference](../reference/configuration.md). Restart the server after editing.
