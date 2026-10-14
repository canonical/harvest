#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"
MODEL="${1:-$(juju models --format json | python3 -c 'import json,sys; print(json.load(sys.stdin)["current-model"])')}"
TIMEOUT="${TIMEOUT:-90m}"
FIRST="${FIRST:-postgresql}"

for file in harvest-server/harvest-server_amd64.charm harvest-web-ui/harvest-web-ui_amd64.charm harvest.snap; do
  [ -s "$file" ] || { echo "missing $file" >&2; exit 1; }
done

juju model-config -m "$MODEL" enable-os-upgrade=false cloudinit-userdata="$(cat cloudinit-userdata.yaml)"

first_bundle=$(mktemp --tmpdir="$PWD" .first-XXXXXX.yaml)
trap 'rm -f "$first_bundle"' EXIT
wanted=$(python3 - "$first_bundle" "$FIRST" <<'PY'
import sys, yaml
bundle = yaml.safe_load(open("bundle.yaml"))
names = sys.argv[2].split(",")
apps = {name: bundle["applications"][name] for name in names}
used = sorted({int(m) for app in apps.values() for m in app.get("to", [])})
first = {
    "default-base": bundle["default-base"],
    "machines": {str(m): bundle["machines"][str(m)] for m in used},
    "applications": apps,
}
yaml.safe_dump(first, open(sys.argv[1], "w"))
print(len(bundle["machines"]))
PY
)
existing=$(juju machines -m "$MODEL" --format json | python3 -c 'import json,sys; print(len(json.load(sys.stdin).get("machines", {})))')
if [ "$existing" -lt "$wanted" ]; then
  juju add-machine -m "$MODEL" -n $((wanted - existing)) --base ubuntu@24.04
fi
juju deploy -m "$MODEL" "$first_bundle" --map-machines=existing
juju deploy -m "$MODEL" ./bundle.yaml --map-machines=existing
juju wait-for model "$MODEL" --timeout "$TIMEOUT" \
  --query='forEach(units, unit => unit.workload-status == "active" && unit.agent-status == "idle")'
juju status -m "$MODEL"
