#!/usr/bin/env bash
set -euo pipefail

runtime=$(mktemp -d)
trap 'rm -rf "$runtime"' EXIT
mkdir -p "$runtime/syauth"
export XDG_RUNTIME_DIR="$runtime"
now_ms=$(date +%s%3N)
cat > "$runtime/syauth/rssi.last" <<EOF
raw=-72
filtered=-70.50
sample_epoch_ms=$((now_ms - 5000))
EOF

output=$(desktop/bin/syauth-rssi-status)
grep -Fx 'RSSI raw: -72 dBm' <<<"$output"
grep -Fx 'RSSI filtered: -70.50 dBm' <<<"$output"
grep -Eq '^sample age: 5\.[0-9]s$' <<<"$output"
grep -Fx 'state: telemetry-only' <<<"$output"

rm "$runtime/syauth/rssi.last"
output=$(desktop/bin/syauth-rssi-status)
grep -Fx 'RSSI raw: n/a dBm' <<<"$output"
grep -Fx 'sample age: never' <<<"$output"

echo "RSSI status regression tests: ok"
