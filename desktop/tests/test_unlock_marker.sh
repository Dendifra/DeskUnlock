#!/usr/bin/env bash
# DeskUnlock — the "phone unlock is usable right now" marker.
#
# The DMS lock screen shows the fingerprint indicator (and the unlock gesture
# starts) only while this marker exists, so the marker must track the phone's
# real reachability, not merely "the daemon is running": at boot the daemon is
# up in seconds while the phone needs a little longer, and an indicator that
# does nothing when pressed reads as a broken feature (2026-09-26).
#
# | TC | Scenario                                                    |
# |----|-------------------------------------------------------------|
# | 01 | fresh heartbeat → marker present                            |
# | 02 | stale heartbeat and stale sample → marker removed           |
# | 03 | master switch off → marker removed                          |
# | 04 | daemon not active → marker removed                          |
# | 05 | no bond → marker removed                                    |
# | 06 | fresh RSSI sample alone (no heartbeat file) → marker present |

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MARKER_SCRIPT="$REPO_ROOT/desktop/libexec/syauth-unlock-marker"

pass=0
fail=0
ok() { printf 'ok   %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf 'FAIL %s — %s\n' "$1" "$2"; fail=$((fail + 1)); }

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT

RUNTIME="$T/runtime/syauth"
mkdir -p "$RUNTIME"
printf '[[bond]]\nkind = "Bonded"\npeer_id = "aaaa"\n' >"$T/bonds.toml"

run_marker() {
    SYAUTH_RUNTIME_DIR="$RUNTIME" \
        SYAUTH_UNLOCK_READY="$T/marker" \
        SYAUTH_BONDS_FILE="$T/bonds.toml" \
        SYAUTH_DISABLED_FILE="$T/nodisabled" \
        SYAUTH_MARKER_PRESENCED="${1:-1}" \
        bash "$MARKER_SCRIPT"
}

# TC 01 — fresh heartbeat
: >"$RUNTIME/presence.last"
run_marker 1
if [[ -f "$T/marker" ]]; then
    ok "01 battito fresco → marker presente"
else
    bad "01 battito fresco → marker presente" "marker assente"
fi

# TC 02 — stale heartbeat and stale sample
rm -f "$T/marker"
printf 'raw=-60\nfiltered=-60.00\nsample_epoch_ms=1000\n' >"$RUNTIME/rssi.last"
touch -d '5 minutes ago' "$RUNTIME/presence.last"
run_marker 1
if [[ ! -e "$T/marker" ]]; then
    ok "02 telefono muto → marker rimosso"
else
    bad "02 telefono muto → marker rimosso" "marker ancora presente"
fi

# TC 03 — master off
: >"$RUNTIME/presence.last"
: >"$T/disabled"
SYAUTH_RUNTIME_DIR="$RUNTIME" SYAUTH_UNLOCK_READY="$T/marker" \
    SYAUTH_BONDS_FILE="$T/bonds.toml" SYAUTH_DISABLED_FILE="$T/disabled" \
    SYAUTH_MARKER_PRESENCED=1 bash "$MARKER_SCRIPT"
if [[ ! -e "$T/marker" ]]; then
    ok "03 master off → marker rimosso"
else
    bad "03 master off → marker rimosso" "marker ancora presente"
fi
rm -f "$T/disabled"

# TC 04 — daemon not active
run_marker 0
if [[ ! -e "$T/marker" ]]; then
    ok "04 demone non attivo → marker rimosso"
else
    bad "04 demone non attivo → marker rimosso" "marker ancora presente"
fi

# TC 05 — no bond
printf '[[bond]]\nkind = "Revoked"\npeer_id = "bbbb"\n' >"$T/nobond.toml"
SYAUTH_RUNTIME_DIR="$RUNTIME" SYAUTH_UNLOCK_READY="$T/marker" \
    SYAUTH_BONDS_FILE="$T/nobond.toml" SYAUTH_DISABLED_FILE="$T/nodisabled" \
    SYAUTH_MARKER_PRESENCED=1 bash "$MARKER_SCRIPT"
if [[ ! -e "$T/marker" ]]; then
    ok "05 nessun bond → marker rimosso"
else
    bad "05 nessun bond → marker rimosso" "marker ancora presente"
fi

# TC 06 — fresh RSSI sample alone
rm -f "$RUNTIME/presence.last"
NOW_MS="$(date +%s%3N)"
printf 'raw=-60\nfiltered=-60.00\nsample_epoch_ms=%s\n' "$NOW_MS" >"$RUNTIME/rssi.last"
run_marker 1
if [[ -f "$T/marker" ]]; then
    ok "06 campione fresco senza battito → marker presente"
else
    bad "06 campione fresco senza battito → marker presente" "marker assente"
fi

printf '\n%d passed, %d failed\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
