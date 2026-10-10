#!/bin/bash
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

expect() {
  if eval "$2"; then echo "ok   - $1"; else echo "FAIL - $1"; FAILURES=$((FAILURES + 1)); fi
}

mkdir -p "$WORK/bin" "$WORK/snap/share/harvest-ui/assets"
echo "new" > "$WORK/snap/share/harvest-ui/assets/app-new.js"
cat > "$WORK/bin/snapctl" <<'SH'
#!/bin/bash
case "$2" in
  ui.mode) echo "$UI_MODE_VALUE" ;;
  ui.host) echo "0.0.0.0" ;;
  ui.port) echo "8080" ;;
  *) echo "" ;;
esac
SH
chmod +x "$WORK/bin/snapctl"

render() {
  local mode="$1" common="$WORK/common-$1"
  mkdir -p "$common/ui-assets/assets"
  echo "old" > "$common/ui-assets/assets/app-old.js"
  echo "new" > "$common/ui-assets/assets/app-new.js"
  touch -d "60 days ago" "$common/ui-assets/assets/app-new.js"
  PATH="$WORK/bin:$PATH" UI_MODE_VALUE="$mode" SNAP="$WORK/snap" SNAP_COMMON="$common" CADDY_BIN=/bin/true bash "$HERE/../local/wrapper-ui" > /dev/null
  echo "$common"
}

dir=$(render static)
expect "static mode does not proxy the API" '! grep -q reverse_proxy $dir/Caddyfile'
expect "static mode serves hashed assets as immutable" 'grep -q "immutable" $dir/Caddyfile'
expect "index.html is never cached" 'grep -q "no-cache" $dir/Caddyfile'
expect "assets of the previous release stay available during a rolling upgrade" '[ -f $dir/ui-assets/assets/app-old.js ] && [ -f $dir/ui-assets/assets/app-new.js ]'
expect "assets of the running release are never pruned" '[ -f $dir/ui-assets/assets/app-new.js ]'
expect "a health endpoint exists for the load balancer" 'grep -q "/ui-health" $dir/Caddyfile'

dir=$(render proxy)
expect "proxy mode forwards API paths to the server" 'grep -q "reverse_proxy 127.0.0.1:8080" $dir/Caddyfile'

if [ "$FAILURES" -gt 0 ]; then echo "$FAILURES failure(s)"; exit 1; fi
echo "all ui wrapper tests passed"
