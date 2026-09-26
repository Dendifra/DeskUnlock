#!/usr/bin/env bash
# DeskUnlock — activation scripts must survive a masked unit.
#
# Why this test exists (2026-09-23): `syauth-dms-lock-patch.timer` was masked
# on purpose (the lock-screen adaptation is disabled while the out-of-band
# unlock path is being rebuilt). `syauth-control on` ran `systemctl --user
# enable` on it under `set -euo pipefail`; `enable` fails hard on a masked
# unit, so the script aborted half-way: the daemon never came back and the
# GUI — which calls `syauth-control` — looked like it would not start at all.
#
# The fake `systemctl` below reproduces the real failure mode: `enable` on a
# masked unit exits non-zero with a message. Any regression back to a bulk
# `enable` list makes these cases fail.
#
# | TC | Scenario                                                        |
# |----|-----------------------------------------------------------------|
# | 01 | `syauth-control on` with a masked unit: exit 0, daemon started   |
# | 02 | the masked unit is never enabled and is reported loudly          |
# | 03 | `syauth-control status` reports the mask instead of DEGRADED     |
# | 04 | `syauth-user-setup --bootstrap` also survives the mask           |

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONTROL="$REPO_ROOT/desktop/bin/syauth-control"
SETUP="$REPO_ROOT/desktop/bin/syauth-user-setup"
MASKED_UNIT="syauth-dms-lock-patch.timer"

pass=0
fail=0

ok() {
    printf 'ok   %s\n' "$1"
    pass=$((pass + 1))
}

bad() {
    printf 'FAIL %s — %s\n' "$1" "$2"
    fail=$((fail + 1))
}

# ---------------------------------------------------------------------------
# Fixture: a fake systemctl that behaves like the real one for our purposes.
# ---------------------------------------------------------------------------
make_fixture() {
    local root="$1"
    mkdir -p "$root/home/.config" "$root/bin"
    cat >"$root/bin/loginctl" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
    show-user) echo Linger=yes ;;
    enable-linger) exit 0 ;;
esac
EOF
    chmod +x "$root/bin/loginctl"
    cat >"$root/bin/systemctl" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"$root/calls"
joined="\$*"
case "\$joined" in
    *is-enabled*"$MASKED_UNIT"*) echo masked; exit 0 ;;
    *is-enabled*) [[ "\$joined" == *--quiet* ]] || echo enabled; exit 0 ;;
    *is-active*) exit 0 ;;
esac
if [[ "\$joined" == *enable* && "\$joined" == *"$MASKED_UNIT"* ]]; then
    echo "Failed to enable unit: Unit file $MASKED_UNIT is masked." >&2
    exit 1
fi
exit 0
EOF
    chmod +x "$root/bin/systemctl"
    : >"$root/calls"
}

# Run one script with the fake systemctl ahead of the real one.
run_script() {
    local root="$1" script="$2"
    shift 2
    HOME="$root/home" PATH="$root/bin:$PATH" bash "$script" "$@"
}

# ---------------------------------------------------------------------------
# TC 01/02 — `syauth-control on`
# ---------------------------------------------------------------------------
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
make_fixture "$T"

out="$(run_script "$T" "$CONTROL" on 2>&1)"
rc=$?

if [[ $rc -eq 0 ]]; then
    ok "01 syauth-control on esce 0 con una unità mascherata"
else
    bad "01 syauth-control on esce 0 con una unità mascherata" "exit=$rc output=$out"
fi

if grep -q "enable syauth-presenced.service" "$T/calls"; then
    ok "01b il daemon viene comunque abilitato"
else
    bad "01b il daemon viene comunque abilitato" "calls=$(tr '\n' '|' <"$T/calls")"
fi

if grep -q "start syauth-presenced.service" "$T/calls"; then
    ok "01c il daemon viene comunque avviato"
else
    bad "01c il daemon viene comunque avviato" "calls=$(tr '\n' '|' <"$T/calls")"
fi

if grep -q "enable $MASKED_UNIT" "$T/calls"; then
    bad "02 la unità mascherata non viene mai abilitata" "calls=$(tr '\n' '|' <"$T/calls")"
else
    ok "02 la unità mascherata non viene mai abilitata"
fi

if [[ "$out" == *"mascherata"* ]]; then
    ok "02b la unità saltata viene detto a voce alta"
else
    bad "02b la unità saltata viene detto a voce alta" "output=$out"
fi

# ---------------------------------------------------------------------------
# TC 03 — `syauth-control status`
# ---------------------------------------------------------------------------
out="$(run_script "$T" "$CONTROL" status 2>&1)"
rc=$?

if [[ $rc -eq 0 && "$out" == *"Syauth: ON"* && "$out" == *"mascherate"* ]]; then
    ok "03 status: ON con la mascherata elencata (non DEGRADED)"
else
    bad "03 status: ON con la mascherata elencata (non DEGRADED)" "exit=$rc output=$out"
fi

# ---------------------------------------------------------------------------
# TC 04 — `syauth-user-setup --bootstrap`
# ---------------------------------------------------------------------------
: >"$T/calls"
out="$(run_script "$T" "$SETUP" --bootstrap 2>&1)"
rc=$?

if [[ $rc -eq 0 ]]; then
    ok "04 syauth-user-setup --bootstrap esce 0 con una unità mascherata"
else
    bad "04 syauth-user-setup --bootstrap esce 0 con una unità mascherata" "exit=$rc output=$out"
fi

if grep -q "enable $MASKED_UNIT" "$T/calls"; then
    bad "04b user-setup non abilita la unità mascherata" "calls=$(tr '\n' '|' <"$T/calls")"
else
    ok "04b user-setup non abilita la unità mascherata"
fi

# ---------------------------------------------------------------------------
printf '\n%d passed, %d failed\n' "$pass" "$fail"
[[ $fail -eq 0 ]]
