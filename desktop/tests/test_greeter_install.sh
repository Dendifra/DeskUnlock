#!/usr/bin/env bash
# Journey: specs/bugs/BUG-20260926-172757-greeter-readiness.md
# Fresh-install contract: Plasma waits for Bluetooth before exposing PAM.
set -uo pipefail

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT
mkdir -p "$ROOT/bin" "$ROOT/etc/pam.d" "$ROOT/etc/systemd/system"
cat >"$ROOT/etc/pam.d/plasmalogin" <<'PAM'
#%PAM-1.0
auth        include     system-login
account     include     system-login
PAM

cat >"$ROOT/bin/id" <<'SH'
#!/usr/bin/env bash
[[ "${1:-}" == "-u" ]] && { echo 0; exit 0; }
exec /usr/bin/id "$@"
SH
cat >"$ROOT/bin/syauth" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
service=plasmalogin
pam_dir=/etc/pam.d
while (($#)); do
    case "$1" in
        --service) service="$2"; shift 2 ;;
        --pam-dir) pam_dir="$2"; shift 2 ;;
        *) shift ;;
    esac
done
printf 'auth    sufficient    pam_syauth.so timeout=8000\n' >>"$pam_dir/$service"
: >"$pam_dir/$service.bak"
echo installed
SH
cat >"$ROOT/bin/systemctl" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"${SYAUTH_CALLS:?}"
exit 0
SH
chmod +x "$ROOT/bin/id" "$ROOT/bin/syauth" "$ROOT/bin/systemctl"
: >"$ROOT/calls"

if ! PATH="$ROOT/bin:$PATH" \
    SYAUTH_CALLS="$ROOT/calls" \
    SYAUTH_PAM_DIR="$ROOT/etc/pam.d" \
    SYAUTH_SYSTEMD_DIR="$ROOT/etc/systemd/system" \
    SYAUTH_SYAUTH_BIN="$ROOT/bin/syauth" \
    bash desktop/libexec/syauth-pam-sync install >"$ROOT/output" 2>&1; then
    printf 'FAIL helper exited non-zero\n'
    cat "$ROOT/output"
    exit 1
fi

grep -q 'pam_syauth.so' "$ROOT/etc/pam.d/plasmalogin" || { echo 'FAIL PAM line missing'; exit 1; }
grep -q 'Wants=bluetooth.service' "$ROOT/etc/systemd/system/plasmalogin.service.d/deskunlock.conf" || { echo 'FAIL Wants missing'; exit 1; }
grep -q 'After=bluetooth.service' "$ROOT/etc/systemd/system/plasmalogin.service.d/deskunlock.conf" || { echo 'FAIL After missing'; exit 1; }
grep -q -- '--system daemon-reload' "$ROOT/calls" || { echo 'FAIL daemon-reload missing'; exit 1; }
printf '1 passed, 0 failed\n'
