#!/usr/bin/env bash
# Start Harvest: the knowledge-server backend and the web UI (`vite preview`
# over the built web-ui/dist, proxying API routes to the backend).
#
# Expects prebuilt artifacts — run scripts/install-systemd-service.sh, or:
#   cargo build --release -p knowledge-server -p agent
#   (cd web-ui && npm ci && npm run build)
#
# Usage: scripts/start-harvest.sh --config /path/to/server.toml
#                                 [--ui-host 0.0.0.0] [--ui-port 3000]
#                                 [--allowed-hosts host1,host2]
#
# Each option can also come from the environment: HARVEST_CONFIG,
# HARVEST_UI_HOST, HARVEST_UI_PORT, HARVEST_ALLOWED_HOSTS.
#
# If either process exits, the other is stopped and the script exits non-zero
# so a supervisor (systemd) restarts the pair.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

CONFIG="${HARVEST_CONFIG:-}"
UI_HOST="${HARVEST_UI_HOST:-0.0.0.0}"
UI_PORT="${HARVEST_UI_PORT:-3000}"
ALLOWED_HOSTS="${HARVEST_ALLOWED_HOSTS:-}"

usage() { sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
  case "$1" in
    -c|--config)     CONFIG="$2"; shift 2 ;;
    --ui-host)       UI_HOST="$2"; shift 2 ;;
    --ui-port)       UI_PORT="$2"; shift 2 ;;
    --allowed-hosts) ALLOWED_HOSTS="$2"; shift 2 ;;
    -h|--help)       usage; exit 0 ;;
    *) echo "error: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }

[ -n "$CONFIG" ] || die "--config <server.toml> is required"
[ -r "$CONFIG" ] || die "config file not readable: $CONFIG"
CONFIG="$(realpath "$CONFIG")"

SERVER_BIN="$REPO_ROOT/target/release/knowledge-server"
DIST_DIR="$REPO_ROOT/web-ui/dist"
VITE_BIN="$REPO_ROOT/web-ui/node_modules/.bin/vite"

[ -x "$SERVER_BIN" ]           || die "$SERVER_BIN missing — run: cargo build --release -p knowledge-server"
[ -f "$DIST_DIR/index.html" ]  || die "$DIST_DIR missing — run: (cd web-ui && npm ci && npm run build)"
[ -x "$VITE_BIN" ]             || die "$VITE_BIN missing — run: (cd web-ui && npm ci)"
command -v node >/dev/null     || die "node not found on PATH"
command -v python3 >/dev/null  || die "python3 not found on PATH (used to read server.toml)"

# Pull what the UI needs out of server.toml: where the backend listens, and the
# public URL hostnames (vite rejects requests whose Host isn't allow-listed).
read -r BACKEND_HOST BACKEND_PORT CONFIG_HOSTS < <(python3 - "$CONFIG" <<'EOF'
import sys, tomllib
from urllib.parse import urlparse
with open(sys.argv[1], "rb") as f:
    cfg = tomllib.load(f)
server = cfg.get("server", {})
host = server.get("host", "0.0.0.0")
if host in ("0.0.0.0", "::", ""):
    host = "127.0.0.1"
port = server.get("port", 8080)
urls = [cfg.get("auth", {}).get("public_url"), cfg.get("agents", {}).get("public_url")]
hosts = sorted({urlparse(u).hostname for u in urls if u and urlparse(u).hostname})
print(host, port, ",".join(hosts) or "-")
EOF
)
[ "$CONFIG_HOSTS" = "-" ] && CONFIG_HOSTS=""
case "$BACKEND_HOST" in *:*) BACKEND_HOST="[$BACKEND_HOST]" ;; esac

export HARVEST_BACKEND_URL="http://${BACKEND_HOST}:${BACKEND_PORT}"
export HARVEST_ALLOWED_HOSTS="${CONFIG_HOSTS}${CONFIG_HOSTS:+${ALLOWED_HOSTS:+,}}${ALLOWED_HOSTS}"
export RUST_LOG="${RUST_LOG:-info}"

SERVER_PID=""
UI_PID=""

shutdown() {
  trap - TERM INT EXIT
  for pid in "$UI_PID" "$SERVER_PID"; do
    [ -n "$pid" ] && kill -TERM "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
trap 'shutdown; exit 0' TERM INT
trap shutdown EXIT

echo "Starting knowledge-server (config: $CONFIG, listening on $HARVEST_BACKEND_URL)"
# Run from the config's directory so relative paths inside server.toml resolve
# the same way as when launching the server by hand next to it.
(cd "$(dirname "$CONFIG")" && exec "$SERVER_BIN" --config "$CONFIG") &
SERVER_PID=$!

# Hold the UI back until the backend answers, so the first page load doesn't
# hit proxy errors. Migrations on a fresh database can take a few seconds.
for _ in $(seq 1 120); do
  kill -0 "$SERVER_PID" 2>/dev/null || { wait "$SERVER_PID" || true; die "knowledge-server exited during startup"; }
  curl -fsS -o /dev/null "$HARVEST_BACKEND_URL/health" 2>/dev/null && break
  sleep 1
done

echo "Starting web UI on http://${UI_HOST}:${UI_PORT} (allowed hosts: ${HARVEST_ALLOWED_HOSTS:-localhost/IPs only})"
(cd "$REPO_ROOT/web-ui" && exec "$VITE_BIN" preview --host "$UI_HOST" --port "$UI_PORT" --strictPort) &
UI_PID=$!

set +e
wait -n "$SERVER_PID" "$UI_PID"
STATUS=$?
set -e
if kill -0 "$SERVER_PID" 2>/dev/null; then
  echo "web UI exited (status $STATUS); stopping knowledge-server" >&2
else
  echo "knowledge-server exited (status $STATUS); stopping web UI" >&2
fi
exit $(( STATUS == 0 ? 1 : STATUS ))
