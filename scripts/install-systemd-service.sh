#!/usr/bin/env bash
# Build Harvest and install a systemd service that runs scripts/start-harvest.sh
# (knowledge-server + web UI) persistently, restarting it on failure and at boot.
#
# Usage: sudo scripts/install-systemd-service.sh --config /path/to/server.toml
#             [--user ubuntu] [--name harvest]
#             [--ui-host 0.0.0.0] [--ui-port 3000]
#             [--allowed-hosts host1,host2] [--no-build]
#        sudo scripts/install-systemd-service.sh --uninstall [--name harvest]
#
#   --config         server.toml to run with (required). Must be readable by --user.
#   --user           account the service runs as (default: the user invoking sudo).
#                    It must own/read this checkout and have cargo and node installed.
#   --name           systemd unit name (default: harvest).
#   --ui-host/-port  where the web UI listens (default: 0.0.0.0:3000).
#   --allowed-hosts  extra hostnames the web UI accepts, besides those taken from
#                    auth.public_url / agents.public_url in server.toml.
#   --no-build       skip `cargo build --release` and `npm ci && npm run build`.
#
# Re-run it after pulling new code to rebuild and restart the service.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

CONFIG=""
RUN_USER="${SUDO_USER:-}"
NAME="harvest"
UI_HOST="0.0.0.0"
UI_PORT="3000"
ALLOWED_HOSTS=""
BUILD=1
UNINSTALL=0
ORIG_ARGS=("$@")

usage() { sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; }
die() { echo "error: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    -c|--config)     CONFIG="$2"; shift 2 ;;
    -u|--user)       RUN_USER="$2"; shift 2 ;;
    --name)          NAME="$2"; shift 2 ;;
    --ui-host)       UI_HOST="$2"; shift 2 ;;
    --ui-port)       UI_PORT="$2"; shift 2 ;;
    --allowed-hosts) ALLOWED_HOSTS="$2"; shift 2 ;;
    --no-build)      BUILD=0; shift ;;
    --uninstall)     UNINSTALL=1; shift ;;
    -h|--help)       usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[ "$EUID" -eq 0 ] || die "must run as root: sudo $0 ${ORIG_ARGS[*]}"

UNIT_PATH="/etc/systemd/system/${NAME}.service"

if [ "$UNINSTALL" -eq 1 ]; then
  systemctl disable --now "${NAME}.service" 2>/dev/null || true
  rm -f "$UNIT_PATH"
  systemctl daemon-reload
  echo "Removed ${NAME}.service"
  exit 0
fi

[ -n "$CONFIG" ] || die "--config <server.toml> is required"
[ -f "$CONFIG" ] || die "config file not found: $CONFIG"
CONFIG="$(realpath "$CONFIG")"

[ -n "$RUN_USER" ] || die "cannot tell which user to run as; pass --user <name>"
[ "$RUN_USER" != "root" ] || die "refusing to run Harvest as root; pass --user <name>"
getent passwd "$RUN_USER" >/dev/null || die "no such user: $RUN_USER"
RUN_HOME="$(getent passwd "$RUN_USER" | cut -d: -f6)"
RUN_GROUP="$(id -gn "$RUN_USER")"

as_user() { sudo -u "$RUN_USER" env HOME="$RUN_HOME" PATH="$SERVICE_PATH" "$@"; }

# sudo resets PATH, so locate the user's toolchains explicitly. Prefer rustup's
# cargo (honours rust-toolchain.toml) and the newest nvm node.
find_first() { for c in "$@"; do [ -x "$c" ] && { echo "$c"; return 0; }; done; return 1; }
CARGO_BIN="$(find_first "$RUN_HOME/.cargo/bin/cargo" /usr/local/bin/cargo /usr/bin/cargo)" \
  || die "cargo not found for $RUN_USER (install rustup: https://rustup.rs)"
NVM_NODE="$(ls -1d "$RUN_HOME"/.nvm/versions/node/*/bin/node 2>/dev/null | sort -V | tail -n1 || true)"
NODE_BIN="$(find_first "$NVM_NODE" /usr/local/bin/node /usr/bin/node /snap/bin/node)" \
  || die "node not found for $RUN_USER (install Node.js 20+)"
SERVICE_PATH="$(dirname "$NODE_BIN"):$(dirname "$CARGO_BIN"):/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/snap/bin"

command -v python3 >/dev/null || die "python3 is required (apt install python3)"
command -v curl >/dev/null    || die "curl is required (apt install curl)"
command -v dot >/dev/null     || echo "note: graphviz not installed — design-document diagrams won't render (apt install graphviz)" >&2

as_user test -r "$CONFIG"                     || die "$RUN_USER cannot read $CONFIG"
as_user test -w "$REPO_ROOT"                  || die "$RUN_USER cannot write to $REPO_ROOT (needed to build)"
python3 -c 'import sys, tomllib; c = tomllib.load(open(sys.argv[1], "rb")); c["database"]["url"]' "$CONFIG" 2>/dev/null \
  || die "$CONFIG is not valid TOML or has no [database] url"

if [ "$BUILD" -eq 1 ]; then
  echo "==> Building knowledge-server and harvest-agent (release) as $RUN_USER"
  as_user bash -c 'cd "$1" && cargo build --release -p knowledge-server -p agent' _ "$REPO_ROOT"
  echo "==> Building web UI as $RUN_USER"
  as_user bash -c 'cd "$1/web-ui" && npm ci && npm run build' _ "$REPO_ROOT"
fi

CAPS=""
if [ "$UI_PORT" -lt 1024 ]; then
  CAPS="AmbientCapabilities=CAP_NET_BIND_SERVICE"
fi

echo "==> Writing $UNIT_PATH"
cat > "$UNIT_PATH" <<EOF
[Unit]
Description=Harvest (knowledge-server + web UI)
Documentation=file://${REPO_ROOT}/README.md
After=network-online.target postgresql.service
Wants=network-online.target

[Service]
Type=simple
User=${RUN_USER}
Group=${RUN_GROUP}
WorkingDirectory=${REPO_ROOT}
Environment="HOME=${RUN_HOME}"
Environment="PATH=${SERVICE_PATH}"
Environment="RUST_LOG=info"
ExecStart="${REPO_ROOT}/scripts/start-harvest.sh" --config "${CONFIG}" --ui-host "${UI_HOST}" --ui-port "${UI_PORT}" --allowed-hosts "${ALLOWED_HOSTS}"
Restart=always
RestartSec=5
TimeoutStopSec=30
${CAPS}

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable "${NAME}.service" >/dev/null
systemctl restart "${NAME}.service"

echo "==> ${NAME}.service installed and (re)started"
echo "    Web UI:  http://<this-host>:${UI_PORT}   (first user to register becomes admin)"
echo "    Status:  systemctl status ${NAME}"
echo "    Logs:    journalctl -u ${NAME} -f"
