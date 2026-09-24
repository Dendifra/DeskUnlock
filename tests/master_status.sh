#!/usr/bin/env bash
# Regression test for `syauth-control status`.
#
# Turning Proximity Lock off in the GUI stops its watcher on purpose. The
# master status used to treat that stopped unit as a failure and reported
# DEGRADED, which made the GUI show "Stato degradato" and flag the phone
# association as broken. A disabled Proximity Lock must not degrade the
# session; a genuinely stopped required unit still must.
set -euo pipefail

root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT

export HOME="$root/home"
export XDG_CONFIG_HOME="$root/home/.config"
export PATH="$root/bin:$PATH"
mkdir -p "$HOME/.config/syauth" "$root/bin"

cat > "$root/bin/systemctl" <<'EOF'
#!/usr/bin/env bash
# Only the queries `syauth-control status` makes are implemented.
args=("$@")
unit="${args[$((${#args[@]} - 1))]}"
case " ${FAKE_INACTIVE_UNITS:-} " in
    *" $unit "*) active=1 ;;
    *) active=0 ;;
esac
for a in "${args[@]}"; do
    case "$a" in
        is-active)
            [[ "$active" == 0 ]] && exit 0 || exit 3
            ;;
        is-enabled)
            if [[ " ${args[*]} " == *" --quiet "* ]]; then
                exit 0
            fi
            printf 'enabled\n'
            exit 0
            ;;
    esac
done
exit 0
EOF
chmod +x "$root/bin/systemctl"

control="$(dirname "$0")/../desktop/bin/syauth-control"

run_status() {
    set +e
    out="$("$control" status 2>/dev/null)"
    code=$?
    set -e
}

# TC-01: Proximity Lock disabled and its watcher stopped -> session stays ON.
printf 'enabled=0\n' > "$XDG_CONFIG_HOME/syauth/proximity.conf"
FAKE_INACTIVE_UNITS="syauth-proximity.service" run_status
[[ "$out" == "Syauth: ON" && "$code" -eq 0 ]] || {
    echo "TC-01 failed: out='$out' code=$code"; exit 1
}

# TC-02: Proximity Lock enabled but its watcher stopped -> DEGRADED.
printf 'enabled=1\n' > "$XDG_CONFIG_HOME/syauth/proximity.conf"
FAKE_INACTIVE_UNITS="syauth-proximity.service" run_status
[[ "$out" == "Syauth: DEGRADED" && "$code" -eq 2 ]] || {
    echo "TC-02 failed: out='$out' code=$code"; exit 1
}

# TC-03: A required unit stopped still degrades, even with proximity off.
printf 'enabled=0\n' > "$XDG_CONFIG_HOME/syauth/proximity.conf"
FAKE_INACTIVE_UNITS="syauth-presenced.service" run_status
[[ "$out" == "Syauth: DEGRADED" && "$code" -eq 2 ]] || {
    echo "TC-03 failed: out='$out' code=$code"; exit 1
}

# TC-04: Master OFF marker wins over everything.
touch "$XDG_CONFIG_HOME/syauth/disabled"
FAKE_INACTIVE_UNITS="syauth-presenced.service" run_status
[[ "$out" == "Syauth: OFF" && "$code" -eq 0 ]] || {
    echo "TC-04 failed: out='$out' code=$code"; exit 1
}

echo "Master status tests: ok"
