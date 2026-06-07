---
name: landscape
description: Deploy and operate Canonical Landscape for Ubuntu fleet management — quickstart single-server install and charmed HA deployment with Juju, plus client registration and day-2 operations
---

# Canonical Landscape

Landscape is Canonical's systems management tool for Ubuntu machines. It provides package management, security patching, script execution, role-based access, and compliance monitoring across a fleet. It is available as SaaS (landscape.canonical.com) or self-hosted.

Self-hosted has two deployment paths:
- **Quickstart**: single-server, automated, no Juju required — suitable for ≤ 1 000 clients
- **Charmed**: Juju-driven, horizontally scalable, HA-capable — recommended for production

---

## Quickstart deployment

The quickstart installer deploys all Landscape components (application server, PostgreSQL, RabbitMQ, NGINX) on one machine.

### Requirements

- Ubuntu 22.04 LTS (or 24.04 LTS)
- Minimum 4 CPU, 8 GB RAM, 100 GB disk
- A resolvable hostname or IP that client machines can reach

### Install

```bash
sudo apt update
sudo apt install -y landscape-server-quickstart
```

The installer prompts for:
- Hostname / FQDN (used in client registration URLs)
- TLS certificate (self-signed or provide paths to key + cert)

After install, the web UI is at `https://<hostname>/`.  
Create the first admin account through the web UI on first visit.

### Post-install configuration

```bash
# Reconfigure (re-runs setup wizard)
sudo dpkg-reconfigure landscape-server

# Services managed by systemd
sudo systemctl status landscape-server
sudo systemctl restart landscape-server

# Application config file
cat /etc/landscape/service.conf

# Logs
journalctl -u landscape-server -f
tail -f /var/log/landscape/server.log
```

### Upgrades (quickstart)

```bash
sudo apt update && sudo apt upgrade -y
sudo systemctl restart landscape-server
```

---

## Charmed deployment

The charmed deployment uses Juju to orchestrate Landscape components as separate applications, enabling horizontal scaling and high availability.

### Architecture

| Charm | Role |
|---|---|
| `landscape-server` | Application tier (can scale to multiple units) |
| `postgresql` | Database backend (use `patroni` bundle for HA) |
| `rabbitmq-server` | Async job queue |
| `haproxy` | Load balancer in front of multiple `landscape-server` units |

### Bootstrap and deploy

```bash
# Bootstrap a controller if needed
juju bootstrap lxd lxd-controller
juju add-model landscape

# Deploy components
juju deploy landscape-server
juju deploy postgresql
juju deploy rabbitmq-server
juju deploy haproxy

# Wire relations
juju integrate landscape-server:db          postgresql:db
juju integrate landscape-server:amqp        rabbitmq-server:amqp
juju integrate landscape-server:reverseproxy haproxy:reverseproxy

# Wait for everything to settle
juju status --watch 5s
```

### Scale the application tier

```bash
# Add units for HA (minimum 2 for active-active)
juju add-unit landscape-server -n 2

# Confirm all units are active
juju status landscape-server
```

### HA database with Patroni

```bash
juju deploy postgresql --channel 14/stable -n 3

juju integrate landscape-server:db postgresql:db
```

### TLS termination

HAProxy handles TLS in the charmed model. Configure the certificate via charm config:

```bash
# Self-signed (development only)
juju config haproxy ssl_cert=SELFSIGNED

# Provide a certificate bundle
juju config haproxy ssl_cert="$(base64 < cert.pem)"
juju config haproxy ssl_key="$(base64 < key.pem)"
```

### Configuration knobs

```bash
# Landscape application config
juju config landscape-server

# Set FQDN (used in client registration URLs)
juju config landscape-server fqdn=landscape.example.com

# SMTP for outbound email
juju config landscape-server smtp_host=smtp.example.com
juju config landscape-server smtp_from=landscape@example.com
```

### Upgrades (charmed)

```bash
# Refresh to a new charm revision or channel
juju refresh landscape-server --channel latest/stable
juju refresh postgresql --channel 14/stable

# Monitor hook progress
juju debug-log --include unit-landscape-server-0
```

### Useful status checks

```bash
juju status landscape-server --format=yaml | grep -A5 "application-status"
juju exec --unit landscape-server/0 -- sudo systemctl status landscape-*
juju exec --unit landscape-server/0 -- sudo landscape-server-admin --help
```

---

## Client registration

Install and register the Landscape client on each managed machine.

```bash
# Install the client
sudo apt install -y landscape-client

# Register (non-interactive)
sudo landscape-config \
  --computer-title "$(hostname)" \
  --account-name standalone \
  --url https://landscape.example.com/message-system \
  --ping-url https://landscape.example.com/ping \
  --silent

# For self-signed certificates, add:
#   --ssl-public-key /path/to/landscape_server.pem

sudo systemctl enable --now landscape-client
sudo systemctl status landscape-client
```

### Silent registration in cloud-init

```yaml
#cloud-config
packages:
  - landscape-client
runcmd:
  - landscape-config --silent
      --computer-title "$(hostname)"
      --account-name standalone
      --url https://landscape.example.com/message-system
      --ping-url https://landscape.example.com/ping
```

### Client config file

```bash
cat /etc/landscape/client.conf
sudo landscape-config --show       # print current config
sudo landscape-config --disable    # unregister
```

---

## Day-2 operations

### Package management (via API or UI)

Landscape tracks available upgrades per machine. Upgrades can be approved and scheduled from the web UI or via the REST API.

```bash
# Using the landscape-api CLI (install separately)
pip install landscape-api
landscape-api --uri https://landscape.example.com/api \
              --key <access-key> --secret <secret-key> \
              get-computers
```

### Script execution

Scripts can be run against computer selections through the Landscape UI or API:

```bash
landscape-api run-script \
  --computers "tag:webservers" \
  --interpreter /bin/bash \
  --code "apt list --upgradable 2>/dev/null"
```

### Server admin CLI

```bash
# Available on the landscape-server unit
sudo landscape-server-admin create-account \
  --name myorg --admin-email admin@example.com --admin-name "Admin"

sudo landscape-server-admin list-accounts
```

---

## Common patterns

- Use the charmed deployment for any production environment expected to grow beyond a single server or requiring zero-downtime upgrades
- The `fqdn` config must be reachable from all client machines — use a stable DNS name, not an IP
- After scaling `landscape-server`, run `juju status` and confirm all units show `active/idle` before directing traffic through HAProxy
- Client connectivity problems: check `/var/log/landscape/client.log` and verify the ping URL is reachable (`curl https://landscape.example.com/ping`)
- For air-gapped environments, configure a local APT mirror in Landscape's repository management and point clients at it
