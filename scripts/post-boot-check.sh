#!/usr/bin/env bash
# DeskUnlock — post-boot health check.
#
# Run after a reboot to confirm every DeskUnlock piece came back on its own:
# master switch, bond, presence daemon, proximity watcher, inactivity lock and
# the DMS lock-screen adaptation (fingerprint / phone unlock). Prints one
# PASS/FAIL line per invariant and exits non-zero if any failed.
#
# The DMS adaptation is the fragile one: DMS extracts its embedded UI into a
# fresh tmpfs dir on every boot, so the patch timer must re-apply it AND DMS
# must reload it (the patch restarts DMS once). Give the timer ~60 s after login
# before judging that line.
set -uo pipefail

pass=0
fail=0
ok() {
    printf 'PASS  %-18s %s\n' "$1" "${2:-}"
    pass=$((pass + 1))
}
bad() {
    printf 'FAIL  %-18s %s\n' "$1" "${2:-}"
    fail=$((fail + 1))
}

# Master switch
if [[ "$(syauth-control status 2>/dev/null)" == "Syauth: ON" ]]; then
    ok master "ON"
else
    bad master "$(syauth-control status 2>&1)"
fi

# Bonded phone
bonded="$(syauth list 2>/dev/null | grep -c bonded)"
if [[ "$bonded" -ge 1 ]]; then
    ok bond "$(syauth list 2>/dev/null | grep bonded | awk -F'\t' '{print $2}')"
else
    bad bond "nessun telefono associato"
fi

# Presence daemon
if systemctl --user -q is-active syauth-presenced.service; then
    ok presenced "active"
else
    bad presenced "non attivo"
fi

# Proximity watcher
if systemctl --user -q is-active syauth-proximity.service; then
    ok proximity-service "active"
else
    bad proximity-service "non attivo"
fi
enabled="$(syauth-proximity status 2>/dev/null | sed -n 's/^enabled=//p')"
if [[ "$enabled" == "1" ]]; then
    ok proximity-enabled "enabled=1"
else
    bad proximity-enabled "enabled=${enabled:-?}"
fi

# Inactivity lock
if systemctl --user -q is-active syauth-idle-lock.service; then
    ok idle-service "active"
else
    bad idle-service "non attivo"
fi

# DMS lock-screen adaptation actually loaded (fingerprint / phone unlock)
SYAUTH_MASTER_STATE="$(syauth-control status 2>/dev/null)"
SYAUTH_BONDED_COUNT="$bonded"
export SYAUTH_MASTER_STATE SYAUTH_BONDED_COUNT
fingerprint="$(python3 - <<'PY' 2>/dev/null
import importlib.machinery
import importlib.util
import os

loader = importlib.machinery.SourceFileLoader(
    "syauth_settings", "/usr/bin/syauth-settings"
)
spec = importlib.util.spec_from_loader(loader.name, loader)
settings = importlib.util.module_from_spec(spec)
loader.exec_module(settings)
master = "on" if os.environ.get("SYAUTH_MASTER_STATE") == "Syauth: ON" else "off"
bonded = int(os.environ.get("SYAUTH_BONDED_COUNT", "0"))
ok, reason = settings.fingerprint_unlock_health(master, bonded)
print(("ok" if ok else "ko") + " " + reason)
PY
)"
if [[ "$fingerprint" == ok* ]]; then
    ok fingerprint "${fingerprint#ok }"
else
    bad fingerprint "${fingerprint#ko }"
fi

# Greeter phone unlock (plasmalogin PAM). Persists across reboot (it is a file
# on disk), but checked here so one boot verifies the whole stack.
if grep -q "pam_syauth" /etc/pam.d/plasmalogin 2>/dev/null; then
    ok greeter-pam "pam_syauth in plasmalogin"
else
    bad greeter-pam "pam_syauth assente da /etc/pam.d/plasmalogin"
fi

# Live presence samples (only meaningful when the phone is in range)
runtime="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/syauth"
if [[ -e "$runtime/presence.last" ]] && (( $(date +%s) - $(stat -c %Y "$runtime/presence.last") < 60 )); then
    ok presence-samples "freschi"
else
    bad presence-samples "assenti o vecchi (telefono fuori portata?)"
fi

printf '\n%d PASS, %d FAIL\n' "$pass" "$fail"
[[ "$fail" -eq 0 ]]
