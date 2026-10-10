#!/bin/bash
set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
HOOK="$HERE/../hooks/configure"
SERVER_BIN="${SERVER_BIN:?set SERVER_BIN to a knowledge-server binary}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
FAILURES=0

fake_snapctl() {
  mkdir -p "$WORK/bin"
  cat > "$WORK/bin/snapctl" <<'SH'
#!/bin/bash
CONF="$SNAPCTL_STORE"
case "$1" in
  get)
    if [ "$2" = "-d" ]; then
      python3 - "$CONF" "$3" <<'PY'
import json, sys
store = json.load(open(sys.argv[1]))
prefix = sys.argv[2] + "."
tree = {}
for key, value in store.items():
    if key.startswith(prefix):
        parts = key[len(prefix):].split(".")
        node = tree
        for part in parts[:-1]:
            node = node.setdefault(part, {})
        node[parts[-1]] = value
print(json.dumps({sys.argv[2]: tree}))
PY
    else
      python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get(sys.argv[2], ""))' "$CONF" "$2"
    fi
    ;;
  restart) echo "restart $2" >> "$SNAP_COMMON/restarts" ;;
  *) ;;
esac
SH
  chmod +x "$WORK/bin/snapctl"
}

run_case() {
  local name="$1" store="$2"
  local common="$WORK/$name"
  mkdir -p "$common"
  echo "$store" > "$common/store.json"
  if PATH="$WORK/bin:$PATH" SNAPCTL_STORE="$common/store.json" SNAP_COMMON="$common" SNAP="/snap/harvest/current" bash "$HOOK" > "$common/out" 2>&1; then
    echo 0 > "$common/status"
  else
    echo $? > "$common/status"
  fi
  echo "$common"
}

expect() {
  if eval "$2"; then
    echo "ok   - $1"
  else
    echo "FAIL - $1"
    FAILURES=$((FAILURES + 1))
  fi
}

fake_snapctl

dir=$(run_case standalone '{"database.url":"postgres://h:p@db:5432/harvest","auth.jwt-secret":"s3cr3t","llm.main.provider":"gemini","llm.main.api-key":"k","server.host":"0.0.0.0","server.port":"8080"}')
expect "standalone config hook succeeds" '[ "$(cat $dir/status)" = 0 ]'
expect "standalone config uses postgres" 'grep -q "url = \"postgres://h:p@db:5432/harvest\"" $dir/server.toml'
expect "standalone config never mentions neo4j" '! grep -qi neo4j $dir/server.toml'
expect "standalone config is accepted by the server" '"$SERVER_BIN" --config $dir/server.toml check-config > /dev/null'
expect "harvester config points at the same database" 'grep -q "postgres://h:p@db:5432/harvest" $dir/harvester.toml'

dir=$(run_case cluster '{"database.url":"postgres://h:p@db1:5432,db2:5432/harvest?target_session_attrs=read-write","database.pool-size":"24","auth.jwt-secret":"s","llm.main.provider":"gemini","llm.main.api-key":"k","cluster.internal-listen":"0.0.0.0:8081","cluster.internal-url":"http://10.1.1.1:8081","cluster.shared-secret":"cs"}')
expect "cluster keys are rendered" 'grep -q "internal_url = \"http://10.1.1.1:8081\"" $dir/server.toml && grep -q "pool_size = 24" $dir/server.toml'
expect "cluster config is accepted by the server" '"$SERVER_BIN" --config $dir/server.toml check-config > /dev/null'

dir=$(run_case managed '{"config.managed":"true"}')
echo 'sentinel' > "$dir/server.toml"
PATH="$WORK/bin:$PATH" SNAPCTL_STORE="$dir/store.json" SNAP_COMMON="$dir" SNAP="/snap/harvest/current" bash "$HOOK" > /dev/null 2>&1
expect "managed mode leaves a charm-written server.toml untouched" '[ "$(cat $dir/server.toml)" = sentinel ]'
expect "managed mode leaves restarts to the charm" '[ ! -f $dir/restarts ]'

dir=$(run_case missing '{"database.url":"","auth.jwt-secret":""}')
touch "$dir/server.toml"
PATH="$WORK/bin:$PATH" SNAPCTL_STORE="$dir/store.json" SNAP_COMMON="$dir" SNAP="/snap/harvest/current" bash "$HOOK" > "$dir/out2" 2>&1 && status=0 || status=$?
expect "missing database url is rejected" '[ "$status" != 0 ] && grep -q "database.url" $dir/out2'

if [ "$FAILURES" -gt 0 ]; then
  echo "$FAILURES failure(s)"
  exit 1
fi
echo "all snap configure tests passed"
