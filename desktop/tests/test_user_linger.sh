#!/usr/bin/env bash
# Journey: specs/bugs/BUG-20260926-172757-greeter-readiness.md
# Fresh-install contract: enabling DeskUnlock keeps the user manager alive at boot.
set -uo pipefail

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
mkdir -p "$ROOT/bin" "$ROOT/home/.config/syauth"
cat >"$ROOT/bin/loginctl" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${SYAUTH_CALLS:?}"
case "$1" in
    show-user) echo Linger=no ;;
    enable-linger) exit 0 ;;
esac
SH
cat >"$ROOT/bin/systemctl" <<'SH'
#!/usr/bin/env bash
exit 0
SH
chmod +x "$ROOT/bin/loginctl" "$ROOT/bin/systemctl"
: >"$ROOT/calls"

if ! PATH="$ROOT/bin:$PATH" HOME="$ROOT/home" USER=fixture \
    SYAUTH_CALLS="$ROOT/calls" bash desktop/bin/syauth-control on >"$ROOT/output" 2>&1; then
    printf 'FAIL syauth-control on exited non-zero\n'
    cat "$ROOT/output"
    exit 1
fi

grep -q 'enable-linger fixture' "$ROOT/calls" || { echo 'FAIL linger was not enabled'; exit 1; }
printf '1 passed, 0 failed\n'
