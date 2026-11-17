# Enable Code mode with LXD

**Goal:** coding sessions, each in its own container.

Code mode needs an LXD server reachable from Harvest over HTTPS, and `git` on the Harvest host. Without `[lxd]`, the Code pages say Code mode is not configured.

## 1. Prepare LXD

On the LXD host, with a storage pool, a bridge and a default profile (or run `lxd init`):

```bash
lxc config set core.https_address 127.0.0.1:8443   # or a reachable address
lxc config trust add --name harvest                 # prints a single-use trust token
```

## 2. Point Harvest at it

In `server.toml` (snap: `harvest config set lxd.endpoint=... lxd.trust-token=...`):

```toml
[lxd]
endpoint    = "https://127.0.0.1:8443"
trust_token = "<token from step 1>"
insecure    = true          # only for a self-signed certificate on a trusted network
# ca_cert   = "<PEM>"       # instead of insecure, for a self-signed server
# project   = "default"
# profile   = "default"
```

Restart the server. On start, Harvest creates its own client certificate, joins LXD with the token and logs `LXD client identity successfully joined via trust token`. The token is then used up; the certificate is kept in the database.

## 3. Tune Code mode (optional)

```toml
[code]
pool_size          = 2    # warm containers kept per project
warm_days          = 7    # keep a project warm this many days after last use
idle_hours         = 24   # end sessions idle this long
max_preview_copies = 5    # viewer copies of previews at once
reindex_hours      = 6    # how often to check a project's branch for new commits
preview_ttl_hours  = 24   # remove previews unused this long
```

Each warm container is a full copy of the project's image: lower `pool_size` on small disks.

## 4. Check

Open **Code**, paste a repository URL and select **Open**. The project page shows **Container setup: Ready** after the first image build.

Next: [Serve web previews on their own domain](serve-previews.md).
