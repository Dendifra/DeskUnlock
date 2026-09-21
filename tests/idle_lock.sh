#!/usr/bin/env bash
set -euo pipefail

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
export XDG_CONFIG_HOME="$root/config"
control="$(dirname "$0")/../desktop/bin/syauth-idle-lock"

fail() { echo "FAIL: $*" >&2; exit 1; }
assert_eq() { [[ "$1" == "$2" ]] || fail "$3: expected '$1', got '$2'"; }

status=$(/bin/bash "$control" status)
grep -Fxq 'idle_lock_enabled=1' <<<"$status" || fail "default enabled"
grep -Fxq 'idle_lock_minutes=10' <<<"$status" || fail "default timeout"
grep -Fxq 'mechanism=ext-idle-notify-v1' <<<"$status" || fail "event-driven mechanism"

/bin/bash "$control" disable
/bin/bash "$control" minutes 25
assert_eq 0 "$(/bin/bash "$control" status | awk -F= '$1 == "idle_lock_enabled" { print $2 }')" "disable"
assert_eq 25 "$(/bin/bash "$control" status | awk -F= '$1 == "idle_lock_minutes" { print $2 }')" "timeout update"
/bin/bash "$control" enable
assert_eq 1 "$(/bin/bash "$control" status | awk -F= '$1 == "idle_lock_enabled" { print $2 }')" "enable"
! /bin/bash "$control" minutes 121 >/dev/null 2>&1 || fail "range validation"

while IFS='=' read -r key _; do
    case "$key" in
        idle_lock_enabled|idle_lock_minutes) ;;
        *) fail "unexpected config key: $key" ;;
    esac
done < "$XDG_CONFIG_HOME/syauth/idle.conf"

grep -Fq 'ext_idle_notifier_v1' desktop/wayland/syauth-idle-lock.c || fail "Wayland idle notifier missing"
grep -Fq 'wl_display_dispatch' desktop/wayland/syauth-idle-lock.c || fail "event dispatch missing"
! grep -Eiq '(/dev/input|key(code|text)|mouse(x|y|position)|sleep[[:space:]])' desktop/wayland/syauth-idle-lock.c desktop/bin/syauth-idle-lock || fail "raw input or polling found"
grep -Fq 'ExecStart=/usr/bin/syauth-idle-lock run' desktop/systemd/syauth-idle-lock.service || fail "idle service command missing"
grep -Fq 'syauth-idle-lock.service' desktop/bin/syauth-control || fail "master lifecycle missing"
grep -Fq 'Blocco per inattività' desktop/bin/syauth-settings || fail "GUI title missing"
grep -Fq 'Blocca il PC dopo un periodo senza attività' desktop/bin/syauth-settings || fail "GUI description missing"
grep -Fq 'idle_lock_minutes' desktop/bin/syauth-settings || fail "GUI setting binding missing"

echo 'Idle lock tests: ok'
